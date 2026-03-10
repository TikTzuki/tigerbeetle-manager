# ── Build stage ──────────────────────────────────────────────────────────────
FROM rust:1.92-slim-bookworm AS builder

WORKDIR /build

# Install protoc (required by tonic-build)
RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace manifests first for better layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY examples/ examples/
COPY proto/ proto/

RUN cargo build --bin tb-manager-node --release

# ── TigerBeetle downloader stage ─────────────────────────────────────────────
FROM debian:bookworm-slim AS tb-downloader

ARG TIGERBEETLE_VERSION=0.16.67
# TARGETARCH is set automatically by Buildx: "amd64" or "arm64"
ARG TARGETARCH

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    unzip \
    && rm -rf /var/lib/apt/lists/*

RUN case "$TARGETARCH" in \
      amd64) TB_ZIP="tigerbeetle-x86_64-linux.zip" ;; \
      arm64) TB_ZIP="tigerbeetle-aarch64-linux.zip" ;; \
      *)     echo "Unsupported arch: $TARGETARCH" && exit 1 ;; \
    esac && \
    curl -fsSL \
      "https://github.com/tigerbeetle/tigerbeetle/releases/download/${TIGERBEETLE_VERSION}/${TB_ZIP}" \
      -o /tmp/tigerbeetle.zip && \
    unzip /tmp/tigerbeetle.zip -d /tmp/tb && \
    chmod +x /tmp/tb/tigerbeetle

# ── Runtime stage ─────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/tb-manager-node /usr/local/bin/tb-manager-node
COPY --from=tb-downloader /tmp/tb/tigerbeetle /usr/local/bin/tigerbeetle

ENTRYPOINT ["tb-manager-node"]
