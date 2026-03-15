# TigerBeetle Manager Dashboard

Next.js dashboard for managing TigerBeetle clusters via gRPC.

## Stack

- **Framework**: Next.js 16 (App Router, Turbopack, `output: "standalone"`)
- **API**: tRPC v11 with TanStack React Query v5
- **Styling**: Tailwind CSS v4
- **Validation**: Zod
- **gRPC**: `@grpc/grpc-js` + `@grpc/proto-loader` (proto at `../proto/manager.proto`)
- **Language**: TypeScript 5.6+

## Architecture

```
src/
  app/
    page.tsx                        # Home — login gate, cluster overview (groups nodes by cluster_id)
    nodes/[nodeId]/page.tsx         # Node detail — 6 tabs: Overview, Backup, Accounts, Transfers, Logs, Migrate
    api/
      trpc/[trpc]/route.ts         # tRPC handler
      logs/[nodeId]/route.ts        # SSE log streaming (bridges gRPC StreamLogs)
      migration/execute/route.ts    # SSE migration progress (bridges gRPC ExecuteMigration)
  server/
    nodes.ts                        # Node config from MANAGER_NODES env (host:port list)
    grpc-client.ts                  # All gRPC client functions (unary + streaming)
    routers/manager.ts              # All tRPC procedures
    trpc.ts                         # tRPC init, auth middleware
  trpc/
    client.ts                       # Client-side tRPC hooks
    provider.tsx                    # TRPCProvider
```

## Key Concepts

### Node Identification

Nodes are identified by their **replica index** from the TigerBeetle superblock, not by a
config-assigned name. `MANAGER_NODES` is just `host:port,...` — the node ID is derived from the
superblock at query time via `GetStatus`. URLs use the 0-based config index (e.g., `/nodes/0`).

### Auto-Cluster Discovery

No cluster configuration needed. The dashboard fans out `GetStatus` gRPC calls to all configured
nodes, reads `cluster_id` from each node's superblock, and groups nodes by `cluster_id` client-side.
Offline nodes (no gRPC response) appear in a separate "Offline / Unreachable" section.

### Data File Sources

Account and transfer browsing uses two sources:

- **LSM (checkpointed)** — authoritative current balances/state
- **WAL (pre-checkpoint)** — recent writes not yet flushed to LSM (~960 ops window)

Both are shown side-by-side with independent pagination.

### Migration

Two-step: PlanMigration (read-only pre-flight) → ExecuteMigration (streaming import).
Supports pure balance-snapshot migration (default) and time-window migration (with `cutoff_ts`).
Target cluster is selected from auto-discovered clusters — cluster ID and addresses are resolved
automatically.

## Conventions

- Pages and layouts go in `src/app/` following Next.js App Router conventions
- tRPC routers go in `src/server/routers/` — merge new routers into `root.ts`
- Client-side tRPC hooks are accessed via `import { trpc } from "@/trpc/client"`
- Components using tRPC hooks or React state need `"use client"` directive
- Server-only code stays in `src/server/`
- Use Zod schemas for all tRPC procedure inputs
- gRPC proto at `../proto/manager.proto` — `longs: String` means uint64 fields are strings in TS
- SSE routes (logs, migration) handle auth via `admin_session` cookie check, not tRPC middleware

## Environment Variables

| Variable              | Required | Description                                                                             |
|-----------------------|----------|-----------------------------------------------------------------------------------------|
| `ADMIN_SECRET_KEY`    | Yes      | Admin password for login.                                                               |
| `MANAGER_NODES`       | No       | Comma-separated `host:port` gRPC addresses. Default: `localhost:9090`–`9095` (6 nodes). |
| `NEXT_PUBLIC_APP_URL` | No       | Base URL for tRPC SSR. Default: `http://localhost:3000`.                                |

## Commands

```bash
npm run dev          # Development server (http://localhost:3000)
npm run build        # Production build (standalone output)
npm run start        # Production server
npm run lint         # ESLint
```
