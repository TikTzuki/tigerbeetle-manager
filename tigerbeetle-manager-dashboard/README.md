# Next.js 16 + tRPC + Tailwind CSS Template

A boilerplate project using Next.js 16 (App Router), tRPC v11, TanStack React Query, Tailwind CSS v4, and Drizzle ORM.

## Getting Started

1. Install dependencies:

```bash
npm install
```

2. Copy the environment file:

```bash
cp .env.example .env
```

3. Start the development server:

```bash
npm run dev
```

Open [http://localhost:3000](http://localhost:3000) in your browser.

## Project Structure

```
eslint.config.mjs          # ESLint flat config
next.config.ts             # Next.js configuration
src/
  app/                     # Next.js App Router pages and layouts
    api/trpc/[trpc]/       # tRPC API route handler
    globals.css            # Global styles (Tailwind CSS)
    layout.tsx             # Root layout with TRPCProvider
    page.tsx               # Landing page
  server/                  # Server-side code
    trpc.ts                # tRPC initialization
    routers/
      root.ts              # Root router (merges all sub-routers)
      example.ts           # Example router with hello procedure
  trpc/                    # Client-side tRPC setup
    client.ts              # tRPC React hooks
    provider.tsx           # TRPCProvider with QueryClient
```

## Scripts

- `npm run dev` -- Start the development server (Turbopack)
- `npm run build` -- Build for production (Turbopack)
- `npm start` -- Start the production server
- `npm run lint` -- Run ESLint

## Adding New Procedures

1. Create a new router in `src/server/routers/`.
2. Merge it into the root router in `src/server/routers/root.ts`.
3. Use it on the client with `trpc.yourRouter.yourProcedure.useQuery()`.
