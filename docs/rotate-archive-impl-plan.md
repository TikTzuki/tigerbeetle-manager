# Cluster Migration Plan (6-node → new 6-node)

## Why it's needed

TigerBeetle data files are pre-formatted with a fixed size at format time (`--size` flag). When the LSM fills up,
no more writes are accepted. Since all 6 replicas are identical (replicated state machine, not sharded), they all
fill up simultaneously — you only need to read from **one node's file**.

---

## Migration Mode: Archive + Reproduce (Time-Window)

Migration is split at a user-defined **cutoff timestamp** (`cutoff_ts`, nanoseconds since epoch):

```
time ──────────────────────────────────────────────────────▶
     │◄── ARCHIVE ZONE ──────────────────►│◄── TIME WINDOW ──►│
     0                               cutoff_ts              now
```

| Zone                                             | What happens                                                                                                                                                    |
|--------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------|
| **Before `cutoff_ts`** (Archive)                 | Account balances at `cutoff_ts` are compressed into genesis accounts + synthetic transfers. Individual historical transfers are discarded.                      |
| **Within window `[cutoff_ts, now]`** (Reproduce) | All actual transfers are read from the old data file and replayed verbatim into the new cluster — original IDs, amounts, timestamps, and account IDs preserved. |

### Why this matters

- **Audit & reconciliation** — recent transactions remain queryable in the new cluster with their original IDs.
- **Compression ratio** — only ancient history is discarded; the time window is configurable (days, weeks, months).
- **Balance correctness** — account balances at `cutoff_ts` are computed by subtracting windowed transfers from
  the final (current) balances, then reconstructed synthetically.

### Balance computation at `cutoff_ts`

```
For each account A:
  balance_at_cutoff.debits_posted  = A.debits_posted  − Σ(t.amount  where t.debit_account_id  == A.id AND t.timestamp >= cutoff_ts)
  balance_at_cutoff.credits_posted = A.credits_posted − Σ(t.amount  where t.credit_account_id == A.id AND t.timestamp >= cutoff_ts)
```

These derived balances are then passed to `BalancePlan::build()` as the "final" account state,
generating synthetic transfers that reconstruct exactly `balance_at_cutoff` in the new cluster.
The windowed transfers are then replayed on top, restoring the full final balance.

---

## Architecture: Migration via `tb-manager-node` gRPC

**No separate `tb-migrator` binary.** Migration is built into `tb-manager-node` as gRPC RPCs.

The dashboard acts as the orchestrator — it already knows all 6 nodes and can coordinate the workflow.
Each `tb-manager-node` already has access to:

- Data file via the reader crate
- Process control (stop/start TigerBeetle)
- S3 upload (backup strategy)
- `FormatDataFile` gRPC RPC
- The compressor crate (`BalancePlan` + `Importer`)

### Coordination model

```
┌──────────────────────────────────────────────────────────┐
│  Dashboard (orchestrator)                                │
│  Knows all 6 nodes, coordinates the multi-node workflow  │
└─────┬──────────┬──────────┬──────────────────────┬───────┘
      │          │          │                      │
 ┌────▼────┐ ┌───▼───┐ ┌────▼────┐          ┌──────▼─────┐
 │ node-0  │ │node-1 │ │ node-2  │   ...    │  node-5    │
 │(source) │ │       │ │         │          │            │
 │         │ │       │ │         │          │            │
 │ PlanMig │ │Format │ │ Format  │          │  Format    │
 │ ExecMig │ │       │ │         │          │            │
 └─────────┘ └───────┘ └─────────┘          └────────────┘
```

Only ONE node (the "source") runs `PlanMigration` + `ExecuteMigration`.
All 6 nodes can run `FormatDataFile` (each formats its own replica).

### gRPC RPCs on `tb-manager-node`

| RPC                | Input                                              | Output                                                                    | Side effects                          |
|--------------------|----------------------------------------------------|---------------------------------------------------------------------------|---------------------------------------|
| `PlanMigration`    | `cutoff_ts` (optional, defaults to snapshot-only)  | accounts, pending, synthetic transfers, windowed transfers, ledgers, safe | **Read-only** — no side effects       |
| `ExecuteMigration` | new cluster addresses, new cluster ID, `cutoff_ts` | progress stream (phase, imported, total)                                  | Connects to NEW cluster, imports data |

---

## Pending Transfer Policy

**Decision: Void all pending transfers before migration.**

Before migration begins, the application must:

1. Stop issuing new pending (two-phase) transfers.
2. Post or void every open pending transfer.
3. Confirm zero pending transfers remain — `debits_pending == 0` and `credits_pending == 0` across all accounts.

`PlanMigration` checks this and returns `safe: false` if any account has non-zero pending balances.
`ExecuteMigration` refuses to proceed if `safe` would be false.

---

## Migration Phases

### Phase 1 — Pre-migration (no downtime)

1. Monitor data file usage via the dashboard (capacity meter shows % of file used).
2. Decide on `cutoff_ts` — e.g. `now − 30 days` (nanoseconds). Transfers within the last 30 days will be reproduced.
3. Application stops issuing new pending transfers and settles all open ones.
4. Dashboard calls `PlanMigration(cutoff_ts)` on node-0 → confirms `pending: 0, safe: true, windowed_transfers: N`.
5. Provision 6 new `tb-manager-node` instances (or prepare new data file paths on existing hosts).

### Phase 2 — Cutover (maintenance window)

```
1.  Stop all application writes (traffic cutover)

2.  Dashboard calls PlanMigration(cutoff_ts) on node-0 (final pre-flight check)
      → { accounts: 142350, pending: 0, synthetic_transfers: 238100,
          windowed_transfers: 18400, ledgers: 3, safe: true }

3.  Stop all 6 old TigerBeetle processes via existing gRPC

4.  Dashboard calls FormatDataFile on each of the 6 new nodes:
      FormatDataFile {
        cluster_id:     <new_cluster_id>,
        replica:        0..5,
        replica_count:  6,
        size:           "128GiB",
        data_file_path: "/data/new_0_N.tigerbeetle"
      }
      → each node formats locally (process must be stopped)

5.  Start the 6 new TigerBeetle processes

6.  Dashboard calls ExecuteMigration on node-0:
      ExecuteMigration {
        new_cluster_id:  <new_cluster_id>,
        new_addresses:   "h1:3000,h2:3000,h3:3000,h4:3000,h5:3000,h6:3000",
        cutoff_ts:       <cutoff_ts_ns>
      }

      Import pipeline (node-0):
      ┌─────────────────────────────────────────────────────────────────┐
      │ a. Read accounts from old LSM                                    │
      │ b. Read windowed transfers (timestamp >= cutoff_ts) from LSM+WAL │
      │ c. Compute balance_at_cutoff per account (subtract window delta) │
      │ d. Build BalancePlan from balance_at_cutoff accounts             │
      │    → genesis accounts + synthetic transfers                      │
      │ e. Import into new cluster:                                      │
      │    1. Genesis accounts      (timestamps: 1ns … 2K ns)           │
      │    2. Regular accounts      (imported, balances at cutoff_ts)    │
      │    3. Synthetic transfers   (imported, reconstruct pre-cutoff)   │
      │    4. Windowed transfers    (imported, original IDs + timestamps) │
      └─────────────────────────────────────────────────────────────────┘
      → streams progress back to dashboard (one message per batch)

7.  Dashboard calls TriggerBackup on each old node to archive old data files to S3
```

### Phase 3 — Post-migration

1. Update application connection strings to new cluster addresses.
2. Application resumes normally — no ID offset required (see ID Space section).
3. Confirm new cluster is healthy via dashboard.
4. Decommission old nodes.
5. Old data files are already archived in S3 — retain for audit (90-day minimum).

---

## TigerBeetle `imported` Flag — Timestamp Strategy

All accounts and transfers are created with TigerBeetle's `imported` flag, which lets the importer
set explicit timestamps instead of relying on the cluster clock.

### Constraints (from TigerBeetle docs)

- Timestamps must be **> 0** and **< 2^63** (nanoseconds since epoch).
- Timestamps must be **strictly increasing** within each object type.
- Timestamps must be **in the past** (less than the new cluster's current clock).
- **Cannot mix** imported and non-imported objects in the same batch.
- Transfer timestamps must **postdate** both the debit and credit account timestamps.

### Import ordering (time-window mode)

```
Step 1: Genesis accounts      (imported, timestamps: 1ns … 2K ns)
Step 2: Regular accounts      (imported, sorted by original timestamp, deduplicated)
                               balances reflect state AT cutoff_ts, not final state
Step 3: Synthetic transfers   (imported, timestamps in (max_account_ts, cutoff_ts))
                               reconstruct account balances as of cutoff_ts
Step 4: Windowed transfers    (imported, original timestamps >= cutoff_ts)
                               replay actual transfers in the time window verbatim
```

| Object                                | `imported` | Timestamp source                                                                  |
|---------------------------------------|------------|-----------------------------------------------------------------------------------|
| Genesis accounts (2 per ledger)       | YES        | Sequential: `1, 2, …, 2K` ns                                                      |
| Regular accounts                      | YES        | Original from old file (deduplicated if needed)                                   |
| Synthetic transfers (≤ 2 per account) | YES        | Sequential in `(max_account_ts, cutoff_ts)` — must not overlap windowed transfers |
| Windowed transfers                    | YES        | Original from old file (strictly increasing, original IDs preserved)              |

### Timestamp ordering guarantee

Synthetic transfers must be timestamped **before** the first windowed transfer:

```
max_account_ts  <  synthetic_ts range  <  cutoff_ts  ≤  windowed_transfer_ts
```

If `max_account_ts >= cutoff_ts`, migration is rejected (accounts created after the cutoff
cannot be balanced before it). This is validated in `PlanMigration`.

### Snapshot-only mode (no `cutoff_ts`)

If `cutoff_ts` is omitted, migration falls back to pure balance snapshot — no windowed transfers
are read or replayed. This is equivalent to `cutoff_ts = 0`.

---

## ID Space

TigerBeetle IDs are 128-bit numbers where:

```
id = (timestamp_ms << 80) | random_80_bits
```

- **High 48 bits** — millisecond timestamp (current time when the ID is generated)
- **Low 80 bits** — random

Application-generated IDs have their high 48 bits set to the current timestamp (e.g.,
`~1_700_000_000_000` ms since epoch). Synthetic IDs (`1, 2, 3, …, K`) have timestamp bits = 0.
Windowed transfer IDs are original application IDs — they are replayed as-is.

**Collision is impossible by construction.** No `resume_id` is needed.

---

## What needs to be built

| Component                                                                                             | Location                                                        | Status              |
|-------------------------------------------------------------------------------------------------------|-----------------------------------------------------------------|---------------------|
| gRPC `FormatDataFile`                                                                                 | `proto/manager.proto` + `grpc_service.rs`                       | **Done**            |
| Dashboard: capacity meter                                                                             | `tigerbeetle-manager-dashboard/`                                | **Done**            |
| Compressor: `imported` flag + timestamp strategy                                                      | `crates/compressor/src/{plan,importer}.rs`                      | **Done**            |
| Compressor: two genesis accounts per ledger (debit/credit)                                            | `crates/compressor/src/plan.rs`                                 | **Done**            |
| gRPC `PlanMigration` (snapshot-only)                                                                  | `proto/manager.proto` + `grpc_service.rs`                       | **Done**            |
| gRPC `ExecuteMigration` (snapshot-only)                                                               | `proto/manager.proto` + `grpc_service.rs`                       | **Done**            |
| Dashboard: Migrate tab                                                                                | `tigerbeetle-manager-dashboard/src/app/nodes/[nodeId]/page.tsx` | **Done**            |
| tRPC `PlanMigration` + `getClusterForMigration`                                                       | `src/server/routers/manager.ts`                                 | **Done**            |
| SSE route for `ExecuteMigration` progress                                                             | `src/app/api/migration/execute/route.ts`                        | **Done**            |
| gRPC client wrappers for migration RPCs                                                               | `src/server/grpc-client.ts`                                     | **Done**            |
| Reader: `read_all_transfers_since(cutoff_ts)` — read transfers after timestamp (LSM+WAL merged)       | `crates/manager-node/src/grpc_service.rs`                       | **Done**            |
| Compressor: `BalancePlan::build_windowed(accounts, transfers, cutoff_ts)` — compute balance at cutoff | `crates/compressor/src/plan.rs`                                 | **Done**            |
| Importer: Phase 4 `windowed_transfers` replay in `import_all_with_progress()`                         | `crates/compressor/src/importer.rs`                             | **Done**            |
| gRPC: add `cutoff_ts` to `PlanMigrationRequest` + `ExecuteMigrationRequest`                           | `proto/manager.proto` + `grpc_service.rs`                       | **Done**            |
| gRPC: add `windowed_transfers` count to `PlanMigrationResponse`                                       | `proto/manager.proto`                                           | **Done**            |
| gRPC `ExecuteMigration`: Phase 4 — windowed transfer replay via `build_windowed()`                    | `grpc_service.rs`                                               | **Done**            |
| Dashboard: `cutoff_ts` date/time picker in Migrate tab                                                | `page.tsx`                                                      | **Done**            |
| Dashboard: show `windowed_transfers` count in pre-flight summary                                      | `page.tsx`                                                      | **Done**            |
| Archive old data file to S3 via existing backup strategy                                              | `crates/manager-node/` (reuse `TriggerBackup`)                  | **Done** (existing) |

### Not needed

- Separate `tb-migrator` binary — migration is handled by `tb-manager-node` gRPC RPCs.
- `resume_id` tracking — TigerBeetle IDs embed the current timestamp in high 48 bits; synthetic IDs (1..K, timestamp=0)
  can never collide with real application IDs.
- Pending transfer replay (two-phase commit) — voiding is the precondition.

---

## Proto definitions

```protobuf
// Pre-flight migration check — read-only, no side effects.
        rpc PlanMigration (PlanMigrationRequest) returns (PlanMigrationResponse);

          // Execute migration: archive pre-cutoff balances, reproduce windowed transfers.
          // Streams progress updates back to the caller.
        rpc ExecuteMigration (ExecuteMigrationRequest) returns (stream MigrationProgress);

        message PlanMigrationRequest {
          // Optional cutoff timestamp (nanoseconds since epoch).
          // Transfers at or after this timestamp will be reproduced in the new cluster.
          // Omit (= 0) for pure balance-snapshot migration (no transfer replay).
        uint64 cutoff_ts = 1;
}

        message PlanMigrationResponse {
          // Number of accounts found in the data file.
        uint64 accounts = 1;
    // Number of accounts with non-zero debits_pending or credits_pending.
        uint64 pending_transfers = 2;
          // Number of synthetic transfers that will be generated (≤ 2 × accounts).
        uint64 synthetic_transfers = 3;
    // True if pending_transfers == 0 and migration is safe to proceed.
        bool safe = 4;
          // Number of distinct ledgers found.
        uint32 ledgers = 5;
  // Number of actual transfers in the time window [cutoff_ts, now] to be reproduced.
  // 0 if cutoff_ts was omitted.
        uint64 windowed_transfers = 6;
}

        message ExecuteMigrationRequest {
          // Cluster ID of the NEW cluster to import into.
        uint64 new_cluster_id = 1;
    // Comma-separated addresses of the new cluster (e.g., "h1:3000,h2:3000,...").
        string new_addresses = 2;
          // Optional cutoff timestamp (nanoseconds). Must match the value used in PlanMigration.
          // Omit (= 0) for pure balance-snapshot migration.
        uint64 cutoff_ts = 3;
}

        message MigrationProgress {
          // Current phase:
          //   "genesis_accounts"   — importing genesis placeholder accounts
          //   "accounts"           — importing regular accounts (balances at cutoff_ts)
          //   "synthetic_transfers"— importing synthetic transfers (reconstruct pre-cutoff balances)
          //   "windowed_transfers" — replaying actual transfers from the time window
        string phase = 1;
    // Records imported so far in current phase.
        uint64 imported = 2;
          // Total records to import in current phase.
        uint64 total = 3;
  // True when all phases are complete.
        bool done = 4;
          // Error message (non-empty only on failure).
        string error = 5;
}
```
