# TigerBeetle Database File Analysis

> Reverse-engineered from TigerBeetle source (v0.16.x).
> Sources: `src/vsr.zig`, `src/vsr/superblock.zig`, `src/lsm/schema.zig`, `src/state_machine.zig`, `src/tigerbeetle.zig`

---

## 1. File Layout (Zones)

A TigerBeetle data file is divided into six contiguous **zones** laid out sequentially on disk.
All zones are sector-aligned (4096 B). The grid zone is additionally block-aligned (512 KiB).

```
Offset (default config)    Zone              Size
──────────────────────────────────────────────────────────────────────────
0                          superblock        98,304 B   (4 copies × 24 KiB)
98,304                     wal_headers       262,144 B  (journal header ring)
360,448                    wal_prepares      1,073,741,824 B  (~1 GiB, message ring)
1,074,102,272              client_replies    67,108,864 B  (64 MiB)
1,141,211,136              grid_padding      163,840 B  (alignment filler, zeroed)
1,141,374,976              grid              unbounded  (LSM blocks)
```

Default config parameters that determine these offsets:

| Parameter            | Value   |
|----------------------|---------|
| `block_size`         | 512 KiB |
| `superblock_copies`  | 4       |
| `journal_slot_count` | 1024    |
| `message_size_max`   | 1 MiB   |
| `clients_max`        | 64      |
| `sector_size`        | 4096 B  |

If a cluster is started with non-default flags (e.g. `--clients-max 128`), zone offsets shift accordingly.

---

## 2. Zone Details

### 2.1 Superblock Zone

Stores 4 redundant copies of the `SuperBlockHeader` (quorum-based durability).
The copy with the highest `sequence` number and a valid checksum is the active superblock.

**Key fields:**

| Field                     | Type   | Description                                                  |
|---------------------------|--------|--------------------------------------------------------------|
| `checksum`                | u128   | BLAKE3 checksum of the header                                |
| `version`                 | u16    | Superblock format version (currently 2)                      |
| `release_format`          | u32    | TigerBeetle release that formatted this file                 |
| `sequence`                | u64    | Monotonically increasing; identifies the latest copy         |
| `cluster`                 | u128   | Cluster ID — protects against cross-cluster reads            |
| `replica`                 | u8     | Replica index within the cluster (0–5)                       |
| `replica_count`           | u8     | Total replicas in the cluster (1–6)                          |
| `vsr_state`               | struct | VSR consensus state (view, log_view, commit_max, checkpoint) |
| `manifest_block_count`    | u32    | Number of manifest blocks in the grid                        |
| `manifest_oldest_address` | u64    | First manifest block address                                 |
| `manifest_newest_address` | u64    | Last manifest block address                                  |
| `free_set_*`              | u64    | Free set block address and checksum                          |
| `client_sessions_*`       | u64    | Client sessions block address and checksum                   |

**Important invariant:** `release_format` must match the running binary's release. A version mismatch causes an
immediate startup failure.

### 2.2 WAL Headers Zone

A circular ring of 1024 fixed-size message **headers** (256 bytes each).
Each slot stores the header of the corresponding WAL prepare, used to verify prepares on startup.

- Size: 1024 × 256 B = 262,144 B
- Purpose: fast header validation without reading 1 GiB of prepare bodies

### 2.3 WAL Prepares Zone

A circular ring of 1024 full prepare **messages** (1 MiB each).
Contains the raw VSR prepare messages (client batches + metadata) not yet checkpointed.

- Size: 1024 × 1 MiB = 1,073,741,824 B (~1 GiB)
- Purpose: Write-Ahead Log for uncommitted operations
- On recovery: replayed to reconstruct in-flight operations after a crash

### 2.4 Client Replies Zone

Caches the last reply sent to each client session (64 sessions × 1 MiB each).
Used to deduplicate client retries without re-executing operations.

- Size: 64 × 1 MiB = 67,108,864 B (64 MiB)
- Purpose: at-most-once semantics for client requests

### 2.5 Grid Padding Zone

Zeroed padding to align the grid zone start to `block_size` (512 KiB).
Has no semantic content.

### 2.6 Grid Zone

The main data store. Starts at byte offset 1,141,374,976 (default config).
Contains a flat array of fixed-size **blocks** (512 KiB each), addressed by 1-indexed integer.

Block address 0 is a null sentinel (never written).
Block address `n` maps to file offset: `grid_start + (n - 1) × 512 KiB`

---

## 3. Grid Block Types

Each grid block has a 256-byte header (same format as VSR message headers) that identifies its type via
`header.block_type`:

| BlockType         | Value | Description                                                |
|-------------------|-------|------------------------------------------------------------|
| `reserved`        | 0     | Null sentinel — never written                              |
| `free_set`        | 1     | Bitmap of free/allocated block addresses                   |
| `client_sessions` | 2     | Active client session table                                |
| `manifest`        | 3     | LSM manifest log entry (table metadata)                    |
| `index`           | 4     | LSM table index block (key ranges + value block addresses) |
| `value`           | 5     | LSM table value block (sorted key-value pairs)             |

### 3.1 Free Set Block (`free_set`)

A single block (or chain of blocks) storing a compressed bitmap of which grid block addresses are allocated vs. free.
Read at startup to reconstruct the allocator state.
Its address is stored in the superblock.

### 3.2 Client Sessions Block (`client_sessions`)

Stores the `clients_max` (64) active client session records.
Each record: `session_id`, `request` counter, last reply checksum, and the address of the reply in the client replies
zone.

### 3.3 Manifest Blocks (`manifest`)

A linked list of manifest log blocks, rooted at `superblock.manifest_oldest_address`.
Each block is an append-only log of **manifest events** — records of LSM tables being added to, moved between, or
removed from levels.

Reading the manifest tells you which LSM tables exist, their key ranges, their level, and where their index blocks are
in the grid.

### 3.4 Index Blocks (`index`)

One per LSM table. Contains:

- `tree_id` — identifies which groove and tree this table belongs to
- Key range covered by this table (`key_min`, `key_max`)
- Addresses of all **value blocks** for this table
- Checksums for each value block

### 3.5 Value Blocks (`value`)

One or more per LSM table. Contains sorted key-value pairs:

- **Key**: derived from the indexed field (e.g. account ID, transfer timestamp)
- **Value**: for the object tree — the full serialized object; for index trees — a pointer back to the object's key

---

## 4. LSM Forest Structure

The state machine uses a **Forest** of four **Grooves** (logical collections), each backed by multiple LSM trees:

```
Forest
├── accounts           (AccountsGroove)
├── transfers          (TransfersGroove)
├── transfers_pending  (TransfersPendingGroove)
└── account_events     (AccountEventsGroove)
```

Each groove has:

- One **object tree** (stores full object serialized by primary key = timestamp)
- Multiple **index trees** (one per secondary index field, stores field_value → timestamp mappings)
- An in-memory **cache** for hot objects

### 4.1 `accounts` Groove

Stores `Account` objects (128 bytes each).

**Object layout:**

| Field             | Type | Bytes   | Description                       |
|-------------------|------|---------|-----------------------------------|
| `id`              | u128 | 16      | Unique account identifier         |
| `debits_pending`  | u128 | 16      | Sum of pending debit amounts      |
| `debits_posted`   | u128 | 16      | Sum of posted debit amounts       |
| `credits_pending` | u128 | 16      | Sum of pending credit amounts     |
| `credits_posted`  | u128 | 16      | Sum of posted credit amounts      |
| `user_data_128`   | u128 | 16      | Opaque user-defined data          |
| `user_data_64`    | u64  | 8       | Opaque user-defined data          |
| `user_data_32`    | u32  | 4       | Opaque user-defined data          |
| `reserved`        | u32  | 4       | Must be zero                      |
| `ledger`          | u32  | 4       | Ledger this account belongs to    |
| `code`            | u16  | 2       | Chart-of-accounts code            |
| `flags`           | u16  | 2       | Bitfield (see below)              |
| `timestamp`       | u64  | 8       | Consensus timestamp (primary key) |
| **Total**         |      | **128** |                                   |

**`AccountFlags` bitfield:**

| Bit | Name                             | Description                                                  |
|-----|----------------------------------|--------------------------------------------------------------|
| 0   | `linked`                         | Link this account with the next in the batch (chain)         |
| 1   | `debits_must_not_exceed_credits` | Balance constraint                                           |
| 2   | `credits_must_not_exceed_debits` | Balance constraint                                           |
| 3   | `history`                        | Enable balance history (required for `get_account_balances`) |
| 4   | `imported`                       | Account was imported (timestamp set by client)               |
| 5   | `closed`                         | Account is closed (no new transfers)                         |

**Index trees** (secondary indexes stored in grid):

| Index           | Tree ID | Key type | Used for                |
|-----------------|---------|----------|-------------------------|
| `id`            | 1       | u128     | Lookup by account ID    |
| `user_data_128` | 2       | u128     | Query by user_data_128  |
| `user_data_64`  | 3       | u64      | Query by user_data_64   |
| `user_data_32`  | 4       | u32      | Query by user_data_32   |
| `ledger`        | 5       | u32      | Query by ledger         |
| `code`          | 6       | u16      | Query by code           |
| `timestamp`     | 7       | u64      | Primary object tree     |
| `imported`      | 23      | void     | Query imported accounts |
| `closed`        | 25      | void     | Query closed accounts   |

> **Note:** `debits_posted`, `debits_pending`, `credits_posted`, `credits_pending`, `flags`, `reserved` are **not
indexed** — they are stored only in the object tree (they change on every transfer).

### 4.2 `transfers` Groove

Stores `Transfer` objects (128 bytes each).

**Object layout:**

| Field               | Type | Bytes   | Description                                          |
|---------------------|------|---------|------------------------------------------------------|
| `id`                | u128 | 16      | Unique transfer identifier                           |
| `debit_account_id`  | u128 | 16      | Account to debit                                     |
| `credit_account_id` | u128 | 16      | Account to credit                                    |
| `amount`            | u128 | 16      | Transfer amount                                      |
| `pending_id`        | u128 | 16      | For post/void: ID of the pending transfer            |
| `user_data_128`     | u128 | 16      | Opaque user-defined data                             |
| `user_data_64`      | u64  | 8       | Opaque user-defined data                             |
| `user_data_32`      | u32  | 4       | Opaque user-defined data                             |
| `timeout`           | u32  | 4       | Pending transfer timeout in seconds (0 = no timeout) |
| `ledger`            | u32  | 4       | Ledger this transfer belongs to                      |
| `code`              | u16  | 2       | Chart-of-accounts code                               |
| `flags`             | u16  | 2       | Bitfield (see below)                                 |
| `timestamp`         | u64  | 8       | Consensus timestamp (primary key)                    |
| **Total**           |      | **128** |                                                      |

**`TransferFlags` bitfield:**

| Bit | Name                    | Description                                 |
|-----|-------------------------|---------------------------------------------|
| 0   | `linked`                | Chain with next transfer in batch           |
| 1   | `pending`               | Two-phase: create a pending hold            |
| 2   | `post_pending_transfer` | Two-phase: post (commit) a pending transfer |
| 3   | `void_pending_transfer` | Two-phase: void (cancel) a pending transfer |
| 4   | `balancing_debit`       | Auto-set amount to available debit balance  |
| 5   | `balancing_credit`      | Auto-set amount to available credit balance |
| 6   | `closing_debit`         | Close the debit account after posting       |
| 7   | `closing_credit`        | Close the credit account after posting      |
| 8   | `imported`              | Timestamp set by client (historical import) |

**Index trees:**

| Index               | Tree ID | Key type | Used for                                         |
|---------------------|---------|----------|--------------------------------------------------|
| `id`                | 8       | u128     | Lookup by transfer ID                            |
| `debit_account_id`  | 9       | u128     | Transfers by debit account                       |
| `credit_account_id` | 10      | u128     | Transfers by credit account                      |
| `amount`            | 11      | u128     | Query by amount                                  |
| `pending_id`        | 12      | u128     | Lookup post/void by pending ID                   |
| `user_data_128`     | 13      | u128     | Query by user_data_128                           |
| `user_data_64`      | 14      | u64      | Query by user_data_64                            |
| `user_data_32`      | 15      | u32      | Query by user_data_32                            |
| `ledger`            | 16      | u32      | Query by ledger                                  |
| `code`              | 17      | u16      | Query by code                                    |
| `timestamp`         | 18      | u64      | Primary object tree                              |
| `expires_at`        | 19      | u64      | Pending transfer expiry (timestamp + timeout_ns) |
| `imported`          | 24      | void     | Query imported transfers                         |
| `closing`           | 26      | void     | Query closing transfers                          |

> `timeout` and `flags` are not indexed. `pending_id` is a sparse index (null for single-phase transfers).

### 4.3 `transfers_pending` Groove

Stores `TransferPending` objects (16 bytes each) — tracks the lifecycle of two-phase (pending) transfers.

An entry is **created** when `flags.pending = true` on a transfer.
The entry is **updated** (status changed) when the transfer is posted, voided, or expires.

**Object layout:**

| Field       | Type                  | Bytes  | Description                                                   |
|-------------|-----------------------|--------|---------------------------------------------------------------|
| `timestamp` | u64                   | 8      | Primary key — same timestamp as the original pending transfer |
| `status`    | TransferPendingStatus | 1      | Current status (see below)                                    |
| `padding`   | [7]u8                 | 7      | Zero padding                                                  |
| **Total**   |                       | **16** |                                                               |

**`TransferPendingStatus` enum:**

| Value | Name      | Description                     |
|-------|-----------|---------------------------------|
| 0     | `none`    | Not a pending transfer (unused) |
| 1     | `pending` | Pending, awaiting post or void  |
| 2     | `posted`  | Successfully posted (committed) |
| 3     | `voided`  | Voided (cancelled)              |
| 4     | `expired` | Timed out automatically         |

**Index trees:**

| Index       | Tree ID | Key type              | Used for                                             |
|-------------|---------|-----------------------|------------------------------------------------------|
| `timestamp` | 20      | u64                   | Primary object tree                                  |
| `status`    | 21      | TransferPendingStatus | Query by lifecycle status (e.g. "all still-pending") |

**Derived state:** This groove is NOT needed to replay history. When replaying `create_transfer` operations, TigerBeetle
rebuilds this automatically.

### 4.4 `account_events` Groove

Stores `AccountEvent` objects — a snapshot of both account balances **at the moment of each transfer**.
This is what backs `get_account_balances` queries (balance as-of a timestamp).

Only populated when the account has `flags.history = true`.

**Object layout (256 bytes):**

| Field                     | Type   | Description                                                       |
|---------------------------|--------|-------------------------------------------------------------------|
| `dr_account_id`           | u128   | Debit account ID                                                  |
| `dr_debits_pending`       | u128   | Debit account's debits_pending at this event                      |
| `dr_debits_posted`        | u128   | Debit account's debits_posted at this event                       |
| `dr_credits_pending`      | u128   | Debit account's credits_pending at this event                     |
| `dr_credits_posted`       | u128   | Debit account's credits_posted at this event                      |
| `cr_account_id`           | u128   | Credit account ID                                                 |
| `cr_debits_pending`       | u128   | Credit account's debits_pending at this event                     |
| `cr_debits_posted`        | u128   | Credit account's debits_posted at this event                      |
| `cr_credits_pending`      | u128   | Credit account's credits_pending at this event                    |
| `cr_credits_posted`       | u128   | Credit account's credits_posted at this event                     |
| `timestamp`               | u64    | Primary key — same as the transfer's timestamp                    |
| `dr_account_timestamp`    | u64    | When the debit account was created                                |
| `cr_account_timestamp`    | u64    | When the credit account was created                               |
| `dr_account_flags`        | u16    | Debit account flags at time of event                              |
| `cr_account_flags`        | u16    | Credit account flags at time of event                             |
| `transfer_flags`          | u16    | The transfer's flags                                              |
| `transfer_pending_flags`  | u16    | The original pending transfer's flags (if two-phase)              |
| `transfer_pending_id`     | u128   | ID of the pending transfer (if posted/voided)                     |
| `amount_requested`        | u128   | Requested amount (may differ from amount for balancing transfers) |
| `amount`                  | u128   | Actual amount transferred                                         |
| `ledger`                  | u32    | Ledger                                                            |
| `transfer_pending_status` | u8     | Status of the related pending transfer                            |
| `reserved`                | [11]u8 | Zero padding                                                      |

**Index trees:**

| Index                         | Tree ID | Description                                                         |
|-------------------------------|---------|---------------------------------------------------------------------|
| `timestamp`                   | 22      | Primary object tree                                                 |
| `account_timestamp`           | 27      | Balance query by account ID + timestamp (special dual-insert index) |
| `transfer_pending_status`     | 28      | Query events by pending status                                      |
| `dr_account_id_expired`       | 29      | Expired events by debit account                                     |
| `cr_account_id_expired`       | 30      | Expired events by credit account                                    |
| `transfer_pending_id_expired` | 31      | Expired events by pending transfer ID                               |
| `ledger_expired`              | 32      | Expired events by ledger                                            |
| `prunable`                    | 33      | Events eligible for pruning                                         |

---

## 5. What Needs to Be Replicated

When copying data to a new cluster or performing a migration, only the **user-facing operations** need to be replayed
through TigerBeetle's API. All internal structures are derived:

| What to replicate            | Why                             |
|------------------------------|---------------------------------|
| `create_account` operations  | Accounts are primary user data  |
| `create_transfer` operations | Transfers are primary user data |

**Not needed** (derived/rebuilt automatically):

| What is derived                  | How                                                     |
|----------------------------------|---------------------------------------------------------|
| `transfers_pending` groove       | Rebuilt from transfer flags during replay               |
| `account_events` groove          | Rebuilt when `flags.history` accounts receive transfers |
| Secondary index trees            | Rebuilt as objects are inserted                         |
| WAL (wal_headers + wal_prepares) | Fresh on new cluster                                    |
| Client replies zone              | Fresh on new cluster                                    |
| Free set block                   | Rebuilt from allocated blocks                           |
| Client sessions block            | Fresh on new cluster                                    |
| Superblock VSR state             | Fresh on new cluster                                    |
| Manifest blocks                  | Rebuilt as LSM compaction runs                          |

**Read order matters:** Accounts must be created before transfers that reference them.
The correct extraction order is: sort all objects by `timestamp` ascending — this preserves causal order.

---

## 6. Reader Crate Mapping

The `crates/reader` crate in this project reads the data file and maps to the above:

| Reader module   | Reads from                      | Purpose                                         |
|-----------------|---------------------------------|-------------------------------------------------|
| `superblock.rs` | Zone 0 (superblock)             | Replica metadata, VSR state, manifest root      |
| `manifest.rs`   | Grid (`manifest` blocks)        | Which LSM tables exist and their levels         |
| `block.rs`      | Grid (`index` + `value` blocks) | Individual LSM table data blocks                |
| `reader.rs`     | Orchestrates all above          | High-level API to iterate accounts / transfers  |
| `layout.rs`     | N/A (constants)                 | File zone offsets and block address translation |
| `types.rs`      | N/A (definitions)               | Rust equivalents of TigerBeetle data types      |

---

## 7. Key Invariants

1. **Checksum everywhere** — every block (grid, superblock, WAL) has a BLAKE3 checksum in its header. Reads must verify
   checksums.
2. **Addresses are 1-indexed** — grid block address 0 is the null sentinel; block 1 is at `grid_start + 0`.
3. **Timestamps are primary keys** — all LSM object trees key on `timestamp`. Timestamps are unique per groove and
   assigned by consensus.
4. **Version immutability** — `superblock.release_format` records the version that formatted the file. Upgrades do not
   change this field; they add migration logic.
5. **No partial state** — the grid and WAL are only consistent at a **checkpoint boundary**. Reading a data file from a
   crashed replica mid-operation may yield stale or incomplete data.
6. **Replica count is immutable** — `replica_count` in the superblock cannot change after formatting.
