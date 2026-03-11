# TigerBeetle Manager

A comprehensive Rust workspace for TigerBeetle database management, including:

- **Data file reader** — Read accounts and transfers directly from `.tigerbeetle` files
- **Data compressor** — Create balance snapshot imports to compress database size
- **Process manager** — Run TigerBeetle with periodic S3 backups and REST API control

## Quick Start

### Install and run the manager server

```bash
cargo build --release --bin tigerbeetle-manager-server

./target/release/tigerbeetle-manager-server \
  --interval-secs 3600 \
  --backup-file /data/0_0.tigerbeetle \
  --bucket my-s3-bucket \
  --port 8080 \
  -- start --addresses=3000 /data/0_0.tigerbeetle
```

### Control via REST API

```bash
# Check status
curl http://localhost:8080/status

# Enable backups every hour
curl -X POST http://localhost:8080/backup/start \
  -H 'Content-Type: application/json' \
  -d '{"interval_secs": 3600}'

# Disable backups
curl -X POST http://localhost:8080/backup/stop \
  -H 'Content-Type: application/json' \
  -d '{}'
```

## Crates

### `tigerbeetle-manager-reader` (`crates/reader/`)

Read TigerBeetle data files without connecting to a cluster. Supports streaming iteration over millions of records with
O(1) memory.

```rust
use tigerbeetle_manager_reader::DataFileReader;

let mut reader = DataFileReader::open("0_0.tigerbeetle") ?;
for account in reader.iter_accounts() ? {
println ! ("Account: {:?}", account ? );
}
```

**Features:**

- Account reader (tree_id = 7)
- Transfer reader (tree_id = 18)
- Lazy streaming iterators
- Custom cluster configurations

### `tigerbeetle-manager-compressor` (`crates/compressor/`)

Compress TigerBeetle databases by creating balance snapshots — each account gets at most 2 synthetic transfers that
reconstruct its exact posted balances.

```rust
use tigerbeetle_manager_compressor::{BalancePlan, Importer};

let accounts = reader.read_accounts() ?;
let plan = BalancePlan::build(accounts);
let importer = Importer::connect(0, "3000").await?;
importer.import_accounts( & plan).await?;
importer.import_transfers( & plan).await?;
```

**Features:**

- Genesis account generation per ledger
- Synthetic transfer generation (credit side → debit side ordering)
- Batch import with tigerbeetle-unofficial client
- Preserves all account metadata and flags

### `tigerbeetle-manager` (`crates/manager/`)

Library for managing TigerBeetle processes with periodic backups.

```rust
use tigerbeetle_manager::{ProcessManager, S3BackupStrategy, ManagerConfig};

let config = ManagerConfig { /* ... */ };
let backup = S3BackupStrategy::new().await;
let manager = ProcessManager::new(config, backup);
manager.run().await?;
```

**Features:**

- Process spawning with log streaming
- Periodic backups (stop → compress → S3 upload → restart)
- REST API for dynamic control
- Graceful shutdown handling

### `tigerbeetle-manager-server` (`crates/manager-server/`) ⭐

**Official binary** for production use. Runs TigerBeetle with optional periodic backups controlled via REST API.

```bash
tigerbeetle-manager-server \
  --interval-secs 3600 \
  --backup-file /data/0_0.tigerbeetle \
  --bucket my-s3-bucket \
  --port 8080 \
  -- start --addresses=3000 /data/0_0.tigerbeetle
```

**REST API:**

- `GET /status` — Get manager state
- `POST /backup/start` — Enable backups
- `POST /backup/stop` — Disable backups

See [`crates/manager-server/README.md`](crates/manager-server/README.md) for full documentation.

## Examples

### Read accounts from data file

```bash
cargo run --bin read-accounts -- /path/to/0_0.tigerbeetle
```

### Read transfers from data file

```bash
cargo run --bin read-transfers -- /path/to/0_0.tigerbeetle
```

### Run manager server

```bash
cargo run --bin tigerbeetle-manager-server -- \
  --port 8080 \
  -- start --addresses=3000 /path/to/0_0.tigerbeetle
```

## Development

### Build all crates

```bash
cargo build --workspace
```

### Build release binary

```bash
cargo build --release --bin tigerbeetle-manager-server
```

Binary will be at: `target/release/tigerbeetle-manager-server`

### Run tests

```bash
cargo test --workspace
```

## Architecture

```
tigerbeetle-manager/
├── crates/
│   ├── core/           # Shared types (currently minimal)
│   ├── reader/         # Data file reader (accounts + transfers)
│   ├── compressor/     # Balance snapshot compression
│   ├── manager/        # Process + backup management library
│   └── manager-server/ # Official binary (REST API server)
└── examples/
    ├── read-accounts/
    └── read-transfers/
```

## AWS Configuration

The manager uses the AWS SDK for S3 backups. Ensure credentials are configured:

1. Environment variables (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`)
2. AWS credentials file (`~/.aws/credentials`)
3. IAM role (if running on EC2/ECS)

Required permissions: `s3:PutObject` on the backup bucket.

## License

MIT/Apache-2.0

cd /Users/tiktuzki/Desktop/repos/personal/tigerbeetle/tigerbeetle-manager/crates/manager-server && \
cargo run --release -- \
--exe /Users/tiktuzki/Desktop/repos/personal/tigerbeetle/.zig-cache/o/025b6d2171dded34c6053b65aaf1149a/tigerbeetle \
--port 8080 \
-- start --addresses=3000 ../../data/0_0.tigerbeetle

cargo run --bin tb-manager-node -- \
--backup-config-file ./backup_config.toml \
--exe tigerbeetle \
-- start --addresses=3000 \
/Users/tiktuzki/Desktop/repos/ewallet/core-ledger-ms/compose/data/tigerbeetle-data/0_0.tigerbeetle

Forest = ForestType(
Storage, .{                                                                                                                                                                                                         
.accounts → Account objects
.transfers → Transfer
objects                                                                                                                                                                                               
.transfers_pending → Pending transfer state (two-phase commit)
.account_events → Account event log (balance history / change events)
})

So beyond accounts and transfers, TigerBeetle also stores:

1. transfers_pending — Tracks active two-phase (pending) transfers. When you create a transfer with flags.pending =
   true, an entry goes here. It gets removed when the transfer is posted or voided. This is what enables idempotent
   post_pending_transfer / void_pending_transfer operations.
2. account_events — Account event/balance history. This backs the get_account_balances query — it records a snapshot of
   account balances at each transfer that touched the account, allowing you to query balance at any point in time.

For the compressor / reader crate, this matters: to faithfully replicate a cluster's state to a new one, you need to
replay:

- All create_account operations
- All create_transfer operations (including pending, post, void)

The transfers_pending and account_events grooves are derived state — they get rebuilt automatically when you replay the
account and transfer operations through TigerBeetle's state machine. You don't need to copy them separately.