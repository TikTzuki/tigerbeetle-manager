FROM node:22-alpine AS base

# Install pnpm
RUN corepack enable && corepack prepare pnpm@latest --activate

# ---- deps ----
# Build context is the repo root so paths are relative to it
FROM base AS deps
WORKDIR /app

COPY tigerbeetle-manager-dashboard/package.json tigerbeetle-manager-dashboard/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile

# ---- builder ----
FROM base AS builder
WORKDIR /app

COPY --from=deps /app/node_modules ./node_modules
COPY tigerbeetle-manager-dashboard/ .
# Proto must be present at build time — grpc-client.ts calls loadSync at module level
COPY proto/ /proto/
# Ensure public/ exists — Next.js requires it even when empty
RUN mkdir -p /app/public

# Build args passed at image build time for public env vars
ARG NEXT_PUBLIC_APP_URL
ENV NEXT_PUBLIC_APP_URL=${NEXT_PUBLIC_APP_URL}

RUN pnpm build

# ---- runner ----
FROM node:22-alpine AS runner
WORKDIR /app

ENV NODE_ENV=production

RUN addgroup --system --gid 1001 nodejs && \
    adduser --system --uid 1001 nextjs

COPY --from=builder /app/public ./public

# Standalone output includes server + minimal node_modules
COPY --from=builder --chown=nextjs:nodejs /app/.next/standalone ./
COPY --from=builder --chown=nextjs:nodejs /app/.next/static ./.next/static

# grpc-client.ts resolves proto as process.cwd()+'/../proto/manager.proto' = /proto/manager.proto
COPY proto/ /proto/

USER nextjs

EXPOSE 3000
ENV PORT=3000
ENV HOSTNAME=0.0.0.0

CMD ["node", "server.js"]
