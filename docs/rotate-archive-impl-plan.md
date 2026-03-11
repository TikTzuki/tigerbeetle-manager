# Cluster Migration Plan (6-node → new 6-node)

## Why it's needed

TigerBeetle data files are pre-formatted with a fixed size at format time (`--size` flag). When the LSM fills up,
no more writes are accepted. Since all 6 replicas are identical (replicated state machine, not sharded), they all
fill up simultaneously — you only need to read from **one node's file**.

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

### New gRPC RPCs on `tb-manager-node`

| RPC                | Input                                 | Output                                                             | Side effects                          |
|--------------------|---------------------------------------|--------------------------------------------------------------------|---------------------------------------|
| `PlanMigration`    | (none)                                | accounts count, pending count, synthetic transfer count, safe flag | **Read-only** — no side effects       |
| `ExecuteMigration` | new cluster addresses, new cluster ID | progress stream (accounts imported, transfers imported)            | Connects to NEW cluster, imports data |

---

## Pending Transfer Policy

**Decision: Void all pending transfers before migration (Option A).**

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
2. Application stops issuing new pending transfers and settles all open ones.
3. Dashboard calls `PlanMigration` on node-0 → confirms `pending: 0, safe: true`.
4. Provision 6 new `tb-manager-node` instances (or prepare new data file paths on existing hosts).

### Phase 2 — Cutover (maintenance window)

```
1.  Stop all application writes (traffic cutover)

2.  Dashboard calls PlanMigration on node-0 (final pre-flight check)
      → { accounts: 142350, pending: 0, synthetic_transfers: 238100, safe: true }

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
        new_addresses:   "h1:3000,h2:3000,h3:3000,h4:3000,h5:3000,h6:3000"
      }
      → node-0 reads its OLD data file
      → builds BalancePlan (compressor crate)
      → connects to NEW cluster via Importer
      → imports genesis accounts → regular accounts → synthetic transfers
      → streams progress back to dashboard

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
set explicit timestamps instead of relying on the cluster clock. This is critical for migration
because it gives full control over ordering.

### Constraints (from TigerBeetle docs)

- Timestamps must be **> 0** and **< 2^63** (nanoseconds since epoch).
- Timestamps must be **strictly increasing** within each object type.
- Timestamps must be **in the past** (less than the new cluster's current clock).
- **Cannot mix** imported and non-imported objects in the same batch.
- Transfer timestamps must **postdate** both the debit and credit account timestamps.

### Import ordering

```
Step 1: Genesis accounts   (imported, timestamps: 1ns, 2ns, …, K ns)
Step 2: Regular accounts    (imported, sorted by original timestamp, deduplicated)
Step 3: Synthetic transfers (imported, sequential timestamps > max account timestamp)
```

| Object                                | `imported` flag | Timestamp source                                                                             |
|---------------------------------------|-----------------|----------------------------------------------------------------------------------------------|
| Genesis accounts (1 per ledger)       | YES             | Sequential: `1, 2, …, K` (nanoseconds)                                                       |
| Regular accounts                      | YES             | Original timestamps from old data file (sorted, strictly increasing, deduplicated if needed) |
| Synthetic transfers (≤ 2 per account) | YES             | Sequential: `max_account_ts + 1, +2, +3, …`                                                  |

### Why genesis accounts must also be imported

If genesis accounts were created **without** the `imported` flag, TigerBeetle would assign them
the current cluster time (~now in nanoseconds). Then regular accounts with the `imported` flag
would need timestamps > now AND < cluster_clock — an impossibly narrow window since the original
account timestamps are historical. By making genesis accounts imported with very early timestamps
(`1ns, 2ns, …`), we ensure the entire timestamp chain is consistent.

### Timestamp deduplication

TigerBeetle requires strictly increasing timestamps. If two accounts from the old cluster share
the same timestamp, the importer bumps the second account's timestamp by 1ns. This preserves
relative order while satisfying the uniqueness constraint.

---

## ID Space

TigerBeetle IDs are 128-bit numbers where:

```
id = (timestamp_ms << 80) | random_80_bits
```

- **High 48 bits** — millisecond timestamp (current time when the ID is generated)
- **Low 80 bits** — random

This means any ID generated by the application has its high 48 bits set to the current timestamp (e.g.,
`~1_700_000_000_000` ms since epoch), making it orders of magnitude larger than the synthetic transfer IDs
used by the compressor (`1, 2, 3, …, K` — all with timestamp bits = 0 / epoch).

**Collision is impossible by construction.** No `resume_id` is needed — the application simply continues
generating time-ordered IDs normally after migration.

---

## What needs to be built

| Component                                                          | Location                                                        | Status              |
|--------------------------------------------------------------------|-----------------------------------------------------------------|---------------------|
| gRPC `FormatDataFile` — format a new data file on a stopped node   | `proto/manager.proto` + `grpc_service.rs`                       | **Done**            |
| Dashboard: capacity meter (% of data file used)                    | `tigerbeetle-manager-dashboard/`                                | **Done**            |
| Compressor: `imported` flag + timestamp strategy                   | `crates/compressor/src/{plan,importer}.rs`                      | **Done**            |
| gRPC `PlanMigration` — read-only pre-flight check                  | `proto/manager.proto` + `grpc_service.rs`                       | **Done**            |
| gRPC `ExecuteMigration` — import into new cluster, stream progress | `proto/manager.proto` + `grpc_service.rs`                       | **Done**            |
| Add `tigerbeetle-manager-compressor` dep to `manager-node`         | `crates/manager-node/Cargo.toml`                                | **Done**            |
| Archive old data file to S3 via existing backup strategy           | `crates/manager-node/` (reuse `TriggerBackup`)                  | **Done** (existing) |
| Dashboard: Migrate tab — preflight check, trigger, progress        | `tigerbeetle-manager-dashboard/src/app/nodes/[nodeId]/page.tsx` | **Done**            |
| tRPC procedure for `PlanMigration`                                 | `src/server/routers/manager.ts`                                 | **Done**            |
| SSE route for `ExecuteMigration` streaming progress                | `src/app/api/migration/execute/route.ts`                        | **Done**            |
| gRPC client wrappers for migration RPCs                            | `src/server/grpc-client.ts`                                     | **Done**            |

### Not needed

- Separate `tb-migrator` binary — migration is handled by `tb-manager-node` gRPC RPCs.
- `resume_id` tracking — TigerBeetle IDs embed the current timestamp in high 48 bits; synthetic IDs (1..K, timestamp=0)
  can never collide with real application IDs.
- Pending transfer replay (Option C) — voiding is the precondition.

---

## Proto definitions (to add)

```protobuf
// Pre-flight migration check — read-only, no side effects.
        rpc PlanMigration (PlanMigrationRequest) returns (PlanMigrationResponse);

            // Execute migration: read old data file, import into new cluster.
            // Streams progress updates back to the caller.
        rpc ExecuteMigration (ExecuteMigrationRequest) returns (stream MigrationProgress);

        message PlanMigrationRequest {}

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
}

        message ExecuteMigrationRequest {
            // Cluster ID of the NEW cluster to import into.
        uint64 new_cluster_id = 1;
    // Comma-separated addresses of the new cluster (e.g., "h1:3000,h2:3000,...").
        string new_addresses = 2;
}

        message MigrationProgress {
            // Current phase: "accounts" or "transfers".
        string phase = 1;
    // Records imported so far in current phase.
        uint64 imported = 2;
            // Total records to import in current phase.
        uint64 total = 3;
    // True when migration is fully complete.
        bool done = 4;
            // Error message (non-empty only on failure).
        string error = 5;
}
```
