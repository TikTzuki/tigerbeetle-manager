# TigerBeetle Manager Dashboard

Web dashboard for managing TigerBeetle database clusters. Communicates with
[tigerbeetle-manager-node](../crates/manager-node/) instances via gRPC.

## Features

- **Cluster Overview** — Auto-discovers clusters by reading `cluster_id` from each node's
  superblock. Groups nodes by cluster, shows online/offline status with 5-second polling.
- **Node Status** — Process state, PID, uptime, replica index, data file capacity with color-coded
  usage bar.
- **Backup Management** — Start/stop cron-scheduled S3 backups, trigger immediate backups, configure
  AWS credentials. Supports 6-field cron patterns (sec min hour dom mon dow) with preset shortcuts.
- **Account Browser** — Paginated browsing of accounts from both LSM (checkpointed) and WAL
  (pre-checkpoint) sources. ID format toggle (UInt128 / UUID), flags format toggle (hex / binary),
  copy-to-clipboard.
- **Transfer Browser** — Same dual-source browsing for transfers with per-column format toggles.
- **Log Streaming** — Real-time SSE log stream with level filtering, text search, pause/resume,
  and auto-scroll.
- **Migration** — Two-step cluster migration: read-only pre-flight check with drill-down into
  accounts, ledger summaries, and synthetic transfers, then streaming execution with live progress
  bar. Supports time-window migration with cutoff date for partial transfer replay.

## Quick Start

```bash
# Install dependencies
npm install

# Copy environment file and set your admin secret
cp .env.example .env

# Start development server
npm run dev
```

Open [http://localhost:3000](http://localhost:3000) and sign in with your `ADMIN_SECRET_KEY`.

## Prerequisites

Each TigerBeetle instance must be managed by a
[tigerbeetle-manager-node](../crates/manager-node/) process exposing a gRPC port:

```bash
# Start a manager node (one per TigerBeetle instance)
cargo run --bin tb-manager-node -- \
  --grpc-port 9090 \
  --exe ./tigerbeetle \
  --backup-config-file ./backup_config.toml \
  -- start --addresses=3000 ./data/0_0.tigerbeetle
```

The dashboard connects to these manager nodes via the `MANAGER_NODES` environment variable:

```bash
# Multi-node setup
MANAGER_NODES=10.0.0.1:9090,10.0.0.2:9090,10.0.0.3:9090

# Default (no env var): 6 nodes on localhost:9090-9095
```

Node IDs and cluster membership are discovered automatically from each node's superblock.

## Environment Variables

| Variable              | Required | Default                 | Description                                 |
|-----------------------|----------|-------------------------|---------------------------------------------|
| `ADMIN_SECRET_KEY`    | Yes      | —                       | Admin password for dashboard login.         |
| `MANAGER_NODES`       | No       | `localhost:9090`–`9095` | Comma-separated `host:port` gRPC addresses. |
| `NEXT_PUBLIC_APP_URL` | No       | `http://localhost:3000` | Base URL for tRPC SSR.                      |

## Project Structure

```
src/
  app/
    page.tsx                        # Login + cluster overview
    nodes/[nodeId]/page.tsx         # Node detail (6 tabs)
    api/
      trpc/[trpc]/route.ts         # tRPC handler
      logs/[nodeId]/route.ts        # SSE log stream
      migration/execute/route.ts    # SSE migration progress
  server/
    nodes.ts                        # Node config (MANAGER_NODES parser)
    grpc-client.ts                  # gRPC client (proto at ../proto/manager.proto)
    routers/manager.ts              # tRPC procedures (auth, nodes, backup, data, migration)
    trpc.ts                         # tRPC init + auth middleware
  trpc/
    client.ts                       # Client hooks
    provider.tsx                    # React Query + tRPC provider
```

## Scripts

| Command         | Description                          |
|-----------------|--------------------------------------|
| `npm run dev`   | Development server with Turbopack    |
| `npm run build` | Production build (standalone output) |
| `npm run start` | Start production server              |
| `npm run lint`  | Run ESLint                           |

## Related

- [tigerbeetle-manager](../) — Parent project with Rust crates
- [manager.proto](../proto/manager.proto) — gRPC service definition
- [tigerbeetle-manager-node](../crates/manager-node/) — Per-node gRPC server
