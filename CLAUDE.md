# TigerBeetle Manager

A Rust-based management system for TigerBeetle database with automated S3 backups, REST API control, and a Next.js
frontend.

## Project Overview

This project provides:

- **Process management** for TigerBeetle database instances
- **Automated backups** to S3 with configurable cron schedules (UTC timezone)
- **REST API** for controlling backups and monitoring status
- **Next.js frontend** with type-safe tRPC API integration
- **Data file reader** for inspecting TigerBeetle binary data structures

## Architecture

```
tigerbeetle-manager/
├── crates/
│   ├── reader/          # Read TigerBeetle data files (superblock, manifest, blocks)
│   ├── manager/         # Core process & backup management logic
│   ├── manager-server/  # REST API server with Axum
│   └── compressor/      # (Future) Compress & copy functionality
├── examples/
│   ├── read-accounts/   # Example: Read accounts from data file
│   └── read-transfers/  # Example: Read transfers from data file
├── tigerbeetle-manager-fe/  # Next.js 16 frontend with tRPC
└── data/
    └── 0_0.tigerbeetle  # TigerBeetle data file (managed)
```

## Stack

### Backend (Rust)

- **Language**: Rust (edition 2024)
- **Build**: Cargo workspace
- **CLI**: clap 4 (derive)
- **Error handling**: anyhow + thiserror
- **Logging**: tracing + tracing-subscriber
- **Async runtime**: tokio
- **Web framework**: axum
- **Cron scheduler**: tokio-cron-scheduler (UTC timezone)
- **AWS SDK**: aws-sdk-s3

### Frontend (TypeScript)

- **Framework**: Next.js 16 with App Router
- **Build**: Turbopack
- **UI**: React Server Components
- **API**: tRPC with HTTP-only cookie auth
- **Styling**: Tailwind CSS

## Quick Start

### Prerequisites

```bash
# TigerBeetle executable (v0.16.66)
# Built from parent directory: ../zig-out/bin/tigerbeetle
# Or use system tigerbeetle if version matches

# Rust toolchain
rustup default stable
```

### Data File Setup

```bash
# Format a new TigerBeetle data file (if needed)
cd /Users/tiktuzki/Desktop/repos/personal/tigerbeetle
zig build run -- format --cluster=0 --replica=0 --replica-count=1 \
  tigerbeetle-manager/data/0_0.tigerbeetle

# Or use built binary directly
.zig-cache/o/025b6d2171dded34c6053b65aaf1149a/tigerbeetle format \
  --cluster=0 --replica=0 --replica-count=1 \
  tigerbeetle-manager/data/0_0.tigerbeetle
```

**Important**: Data file version must match TigerBeetle binary version. Version mismatches will cause startup failures.

### Run Backend (Manager Server)

```bash
cd tigerbeetle-manager

# Run with cron backups (every 5 minutes example)
cargo run --bin tigerbeetle-manager-server -- \
  --exe ../zig-out/bin/tigerbeetle \
  --cron-schedule "*/5 * * * *" \
  --backup-file ./data/0_0.tigerbeetle \
  --bucket tigerbeetle-backups \
  --port 8080 \
  -- start --addresses=3000 ./data/0_0.tigerbeetle

# Or without automated backups
cargo run --bin tigerbeetle-manager-server -- \
  --exe ../zig-out/bin/tigerbeetle \
  --backup-file ./data/0_0.tigerbeetle \
  --bucket tigerbeetle-backups \
  --port 8080 \
  -- start --addresses=3000 ./data/0_0.tigerbeetle
```

REST API will be available at:

- `GET http://localhost:8080/status` — Manager status
- `POST http://localhost:8080/backup/start` — Start backup job
- `POST http://localhost:8080/backup/stop` — Stop backup job

### Run Frontend

```bash
cd tigerbeetle-manager-fe

# Install dependencies (first time)
npm install

# Run dev server
npm run dev
```

Frontend will be available at http://localhost:3000

## Cron Scheduling

### Timezone: UTC Only

**CRITICAL**: All cron schedules run in **UTC timezone** (tokio-cron-scheduler default). Convert local time to UTC
before setting schedules.

Examples:

- 2:00 AM PST = 10:00 AM UTC
- 5:00 PM EST = 10:00 PM UTC

### Cron Pattern Format

5-field format: `minute hour day month weekday`

Common patterns:

```
*/5 * * * *    — Every 5 minutes
0 * * * *      — Every hour
0 */6 * * *    — Every 6 hours
0 0 * * *      — Daily at midnight UTC
0 2 * * *      — Daily at 2:00 AM UTC
0 0 * * 0      — Weekly (Sunday at midnight UTC)
0 0 1 * *      — Monthly (1st day at midnight UTC)
```

### UI Features

The frontend includes:

- Clickable cron pattern examples
- UTC timezone warning (amber box)
- Server restart reminder (schedule changes require restart)

## Development

### Workspace Conventions

- Multi-crate workspace: `crates/*` for main crates, `examples/*` for examples
- All dependency versions are centralized in the root `[workspace.dependencies]`
- New crate: create a directory under `crates/` with its own `Cargo.toml` using `version.workspace = true`,
  `edition.workspace = true`, etc.
- Library crates go in `crates/` with `src/lib.rs`
- Binary crates go in `crates/` with `[[bin]]` in Cargo.toml and `src/main.rs`
- Shared error types live in `crates/core/src/error.rs`
- Cross-crate deps use workspace references: `tigerbeetle-manager-core = { workspace = true }`
- Workspace lints enforce `missing_docs`, `unreachable_pub`, `unused_must_use`
- Use `tracing` macros (`info!`, `warn!`, `error!`) for logging, not `println!`

### Build Commands

```bash
# Build all crates
cargo build

# Build release
cargo build --release

# Run tests
cargo test

# Run specific binary
cargo run --bin tigerbeetle-manager-server -- [args]
cargo run --example read-accounts

# Check formatting
cargo fmt --check

# Lint
cargo clippy -- -D warnings
```

### Frontend Commands

```bash
cd tigerbeetle-manager-fe

npm run dev          # Development server (http://localhost:3000)
npm run build        # Production build
npm run start        # Production server
npm run lint         # ESLint
npm run type-check   # TypeScript type checking
```

## Key Components

### crates/reader

Read TigerBeetle binary data structures:

- **Superblock** — Replica metadata, VSR state, checksums
- **Manifest** — LSM table index (levels, addresses, snapshots)
- **Blocks** — Grid block cache with checksums

Example usage:

```bash
cargo run --example read-accounts
cargo run --example read-transfers
```

### crates/manager

Core logic for:

- TigerBeetle process lifecycle (start, stop, restart, health checks)
- S3 backup orchestration
- Cron job scheduling
- Shared state management (Arc<RwLock<ManagerState>>)

### crates/manager-server

Axum REST API server that:

- Exposes manager control endpoints
- Spawns TigerBeetle subprocess
- Runs cron backup jobs
- Provides status monitoring

### tigerbeetle-manager-fe

Next.js frontend with:

- tRPC API integration
- Status dashboard
- Backup controls with cron scheduling
- UTC timezone warnings
- Clickable cron pattern examples

## Common Issues

### Version Mismatch

```
error: release 0.16.68 is not available; upgrade (or downgrade) the binary
```

**Fix**: Reformat data file with matching TigerBeetle version:

```bash
zig build run -- format --cluster=0 --replica=0 --replica-count=1 \
  tigerbeetle-manager/data/0_0.tigerbeetle
```

### Missing Subcommand

```
error: subcommand required, expected 'format', 'recover', 'start', 'version'...
```

**Fix**: Add TigerBeetle arguments after `--`:

```bash
cargo run --bin tigerbeetle-manager-server -- \
  --exe tigerbeetle \
  -- start --addresses=3000 ./data/0_0.tigerbeetle
```

### Cron Schedule Not Working

**Fix**:

1. Verify schedule is in UTC timezone
2. Restart manager server (schedule changes require restart)
3. Check logs for cron job errors

## Git Ignore

The `.gitignore` excludes:

- Rust build artifacts: `/target`, `Cargo.lock`
- TigerBeetle data files: `*.tigerbeetle`, `data/`
- Next.js build output: `tigerbeetle-manager-fe/.next/`, `tigerbeetle-manager-fe/out/`
- Dependencies: `tigerbeetle-manager-fe/node_modules/`
- Environment files: `.env`, `.env*.local`
- Editor configs: `.idea`, `.vscode`, `.nvim`

## Related Documentation

- TigerBeetle main docs: `../CLAUDE.md`
- TigerBeetle binary format: Reverse-engineered from `src/vsr/superblock.zig`, `src/lsm/manifest.zig`
- VSR protocol: `../src/vsr/replica.zig`