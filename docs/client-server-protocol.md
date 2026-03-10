# TigerBeetle Client-Server Protocol

A comprehensive breakdown of how the TigerBeetle client connects to and communicates with the server.

---

## 1. Transport Layer: TCP over io_uring/kqueue/IOCP

The client uses **TCP/IP** with platform-specific async I/O:

- **Linux**: `io_uring` (see `src/io/linux.zig`)
- **macOS**: `kqueue` (see `src/io/darwin.zig`)
- **Windows**: IOCP (see `src/io/windows.zig`)

Connections are managed in `src/message_bus.zig`. The client creates one TCP connection per replica and cycles through
them on each tick:

```
tick_client() → for each replica without a connection → connect()
    → IO.connect() submitted → on_connect callback
    → .connected state → submit first recv()
```

---

## 2. Message Format: 256-byte Fixed Header + Body

Every message has a **256-byte header** defined in `src/vsr/message_header.zig`:

```
Header (256 bytes):
├── Checksums (64 bytes)
│   ├── checksum: u128        ← BLAKE3 hash of entire header
│   ├── checksum_body: u128   ← BLAKE3 hash of body
│   └── padding
├── Cluster ID (48 bytes)
│   ├── cluster: u128         ← prevents cross-cluster messages
│   └── nonce_reserved
├── Size/Routing (8 bytes)
│   ├── size: u32             ← header + body (max 16 MB)
│   └── epoch: u32
├── Protocol Fields (32 bytes)
│   ├── view: u32             ← current view number
│   ├── command: Command      ← message type enum
│   ├── replica: u8           ← source replica index
│   └── protocol, release, etc.
└── Command-Specific (128 bytes)
    └── varies per command type
```

Body is up to **~16 MB - 256 bytes** of operation-specific data.

---

## 3. Connection Lifecycle

```
┌─────────────┐     TCP connect      ┌──────────────┐
│  Client      │ ──────────────────→  │  Replica      │
│  (.connecting)│                      │  (.accepting)  │
└──────┬──────┘                      └──────┬───────┘
       │         on_connect()                │
       ▼                                     ▼
┌─────────────┐    .register request  ┌──────────────┐
│  .connected  │ ──────────────────→  │  .connected   │
│              │ ←────────────────── │              │
└──────┬──────┘    .register reply    └──────────────┘
       │           (session ID +
       │            batch_size_limit)
       ▼
   Ready for requests
```

**Registration** (`src/vsr/client.zig:257-299`):

- Client sends a `.register` request with `session=0, request=0`
- Replica assigns a **session number** and returns `batch_size_limit`
- All subsequent requests use this session

---

## 4. Request/Response Protocol

**One request inflight at a time** per client. This is a core invariant.

**Request header** contains:

- `parent: u128` — checksum of the **previous reply** (hash chain for linearizability)
- `client: u128` — ephemeral random client ID
- `session: u64` — monotonically increasing session from registration
- `request: u32` — monotonically increasing request counter

**Reply header** contains:

- `request_checksum: u128` — checksum of the corresponding request
- `context: u128` — becomes the `parent` for the next request
- `client: u128` — target client

**Matching logic** (`src/vsr/client.zig:502-627`):

1. Check `reply.client == self.id`
2. Check `reply.request == inflight.request`
3. Verify `reply.request_checksum == inflight.message.checksum`
4. Update `self.parent = reply.context` for next request
5. Call user callback with results

The **parent hash chain** guarantees linearizability:

```
request1.parent=0 → reply1.context → request2.parent → reply2.context → ...
```

---

## 5. Message Framing (Receive Side)

`src/message_buffer.zig` implements a **ring buffer parser**:

```
recv() → bytes arrive → advance()
    ├── advance_header(): wait for 256 bytes, validate header checksum
    ├── advance_body(): wait for full message, validate body checksum
    └── next_header() → consume_message() or suspend_message()
```

Checksums are validated **exactly once** and the result cached via `advance_size` tracking. Invalid checksums cause the
message to be rejected immediately.

---

## 6. Hedging and Timeouts

**Request hedging** (`src/vsr/client.zig:724-734`):

- Sends each request to the **primary** (`view % replica_count`) and a **random backup**
- If primary is down, the backup forwards to the new primary

**Timeouts** (`src/constants.zig:716-732`):

- `request_timeout`: starts at `2 * RTT`, exponential backoff with jitter on failure
- `ping_timeout`: every ~30 seconds, sends `ping_client` to all replicas
- RTT measured via `ping_client`/`pong_client` timestamps, dynamically adjusts request timeout

---

## 7. Eviction

A client can be evicted (`src/vsr/client.zig:403-455`) if:

- Too many concurrent clients (client table overflow)
- Version mismatch (`client_release_too_low` / `client_release_too_high`)
- Session number mismatch

The replica sends an `.eviction` message and the client must re-register with a new session.

---

## Key Files

| File                         | Purpose                                      |
|------------------------------|----------------------------------------------|
| `src/vsr/message_header.zig` | All message format definitions               |
| `src/vsr/client.zig`         | Client state machine, request/reply matching |
| `src/message_bus.zig`        | TCP connection management, send/recv         |
| `src/message_buffer.zig`     | Receive-side framing and checksum validation |
| `src/constants.zig`          | Timeout constants, message size limits       |
| `src/io.zig`                 | Platform I/O abstraction                     |
