# Project Guidelines

## First-Time Setup

When starting a new session in this project for the first time, run `/project-memory` to initialize the project memory
system. This sets up `docs/project_notes/` for tracking bugs, architectural decisions, key facts, and work history.

## Stack

- **Framework**: Next.js 16 (App Router, Turbopack default)
- **API**: tRPC v11 with TanStack React Query v5
- **Styling**: Tailwind CSS v4
- **Validation**: Zod
- **ORM**: Drizzle ORM
- **Language**: TypeScript 5.6+

## Conventions

- Pages and layouts go in `src/app/` following Next.js App Router conventions
- tRPC routers go in `src/server/routers/` — merge new routers into `root.ts`
- Client-side tRPC hooks are accessed via `import { trpc } from "@/trpc/client"`
- Components using tRPC hooks or React state need `"use client"` directive
- Server-only code stays in `src/server/`
- Use Zod schemas for all tRPC procedure inputs
