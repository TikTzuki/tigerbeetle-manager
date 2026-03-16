# TigerBeetle Sync Job — Java Implementation Plan

> Syncs TigerBeetle accounts and transfers into a general-purpose external database.
> Runs as a standalone Java service, connects **directly to TigerBeetle** (no manager-node involved).

---

## Architecture

```
TigerBeetle cluster
  └─ :3000 (or cluster addresses)
        │
        │  TigerBeetle Java client
        ▼
  ┌─────────────┐
  │  Sync Job   │   (Java, standalone JAR or Spring Boot app)
  │             │
  │ SyncCursor  │◄── stores last_synced_timestamp per table
  │ TBReader    │◄── queries TB via Java client
  │ DBWriter    │──► upserts into external DB via JDBC
  └─────────────┘
        │
        ▼
  External Database (PostgreSQL / MySQL / etc.)
```

The sync job does **not** connect to manager-node gRPC. It only talks to:

- TigerBeetle directly (TB client protocol over TCP)
- The external database (JDBC)

---

## Project Structure

```
tigerbeetle-sync-job/
├── build.gradle (or pom.xml)
├── src/main/java/com/yourorg/tbjsync/
│   ├── Main.java                  # entry point, wiring
│   ├── config/
│   │   └── SyncConfig.java        # env/config loading
│   ├── sync/
│   │   ├── SyncJob.java           # scheduler loop
│   │   ├── AccountSyncer.java     # syncs accounts table
│   │   └── TransferSyncer.java    # syncs transfers table
│   ├── tb/
│   │   └── TigerBeetleReader.java # wraps TB Java client
│   └── db/
│       ├── DatabaseWriter.java    # JDBC upserts
│       └── CursorStore.java       # reads/writes sync cursor
├── src/main/resources/
│   └── schema.sql                 # DDL for external DB tables
└── .env.example
```

---

## Dependencies (Gradle)

```groovy
dependencies {
    // TigerBeetle Java client
    implementation 'com.tigerbeetle:tigerbeetle-java:0.16.+'

    // JDBC driver (choose your DB)
    implementation 'org.postgresql:postgresql:42.7.3'
    // or: implementation 'com.mysql:mysql-connector-j:8.3.0'

    // Connection pooling
    implementation 'com.zaxxer:HikariCP:5.1.0'

    // Config from env
    implementation 'io.github.cdimascio:dotenv-java:3.0.0'

    // Logging
    implementation 'org.slf4j:slf4j-api:2.0.12'
    implementation 'ch.qos.logback:logback-classic:1.5.6'
}
```

---

## External DB Schema

```sql
-- Sync cursor: tracks last processed timestamp per table
CREATE TABLE tb_sync_cursor
(
    table_name     VARCHAR(64) PRIMARY KEY,
    last_timestamp BIGINT NOT NULL DEFAULT 0, -- TB nanosecond timestamp
    last_synced_at TIMESTAMPTZ,
    records_synced BIGINT NOT NULL DEFAULT 0
);

-- Accounts mirror
CREATE TABLE tb_accounts
(
    id              NUMERIC(39) PRIMARY KEY, -- u128 → NUMERIC
    ledger          BIGINT NOT NULL,
    code            INT    NOT NULL,
    user_data_128   NUMERIC(39),
    user_data_64    BIGINT,
    user_data_32    INT,
    flags           INT    NOT NULL,
    debits_pending  BIGINT NOT NULL,
    debits_posted   BIGINT NOT NULL,
    credits_pending BIGINT NOT NULL,
    credits_posted  BIGINT NOT NULL,
    tb_timestamp    BIGINT NOT NULL,         -- TigerBeetle-assigned timestamp
    synced_at       TIMESTAMPTZ DEFAULT NOW()
);

-- Transfers mirror
CREATE TABLE tb_transfers
(
    id                NUMERIC(39) PRIMARY KEY, -- u128 → NUMERIC
    debit_account_id  NUMERIC(39) NOT NULL,
    credit_account_id NUMERIC(39) NOT NULL,
    amount            BIGINT      NOT NULL,
    ledger            BIGINT      NOT NULL,
    code              INT         NOT NULL,
    user_data_128     NUMERIC(39),
    user_data_64      BIGINT,
    user_data_32      INT,
    pending_id        NUMERIC(39),
    flags             INT         NOT NULL,
    timeout           INT         NOT NULL,
    tb_timestamp      BIGINT      NOT NULL,    -- TigerBeetle-assigned timestamp
    synced_at         TIMESTAMPTZ DEFAULT NOW()
);

-- Index for time-range queries on the mirror
CREATE INDEX idx_tb_accounts_timestamp ON tb_accounts (tb_timestamp);
CREATE INDEX idx_tb_transfers_timestamp ON tb_transfers (tb_timestamp);
CREATE INDEX idx_tb_transfers_debit ON tb_transfers (debit_account_id);
CREATE INDEX idx_tb_transfers_credit ON tb_transfers (credit_account_id);
```

---

## Sync Algorithm

### Core Principle: Timestamp Watermark

TigerBeetle assigns a **monotonically increasing, cluster-unique nanosecond timestamp** to every
committed account and transfer. This timestamp is the sync cursor — never reused, never out of order.

```
cursor = SELECT last_timestamp FROM tb_sync_cursor WHERE table_name = 'transfers'

loop:
    batch = tb.queryTransfers(timestamp_min = cursor + 1, limit = 1000)
    if batch is empty → sleep(pollInterval), continue

    upsert batch into tb_transfers
    cursor = batch.last().timestamp
    UPDATE tb_sync_cursor SET last_timestamp = cursor, last_synced_at = NOW()

    if batch.size == 1000 → continue immediately (more pages)
    else → sleep(pollInterval)
```

### Pagination

TigerBeetle returns at most `limit` records per query. If the batch is full (== limit),
fetch the next page immediately without sleeping.

### Failure Safety

- **Cursor is only advanced after successful DB write** — a write failure leaves cursor unchanged,
  so the batch is retried on next iteration.
- **Upsert (not insert)** — if a record already exists (from a previous partial run), it is
  overwritten safely. Use `ON CONFLICT (id) DO UPDATE SET ...` in PostgreSQL.
- **No partial cursor advance** — the cursor moves only for the full batch atomically.

---

## Key Classes

### `SyncConfig.java`

```java
public record SyncConfig(
        String[] tbAddresses,    // e.g. ["localhost:3000", "localhost:3001"]
        long clusterId,      // TigerBeetle cluster ID
        String jdbcUrl,
        String dbUser,
        String dbPassword,
        int batchSize,      // default 1000
        long pollIntervalMs  // default 5000
) {
    public static SyncConfig fromEnv() { ...}
}
```

### `TigerBeetleReader.java`

```java
public class TigerBeetleReader implements Closeable {
    private final Client client;

    public List<AccountBatch> queryAccounts(long timestampMin, int limit) {
        AccountFilter filter = new AccountFilter();
        filter.setTimestampMin(timestampMin);
        filter.setLimit(limit);
        filter.setReversed(false);
        return client.queryAccounts(filter);
    }

    public List<TransferBatch> queryTransfers(long timestampMin, int limit) {
        TransferFilter filter = new TransferFilter();
        filter.setTimestampMin(timestampMin);
        filter.setLimit(limit);
        filter.setReversed(false);
        return client.queryTransfers(filter);
    }
}
```

### `SyncJob.java`

```java
public class SyncJob {
    private final AccountSyncer accountSyncer;
    private final TransferSyncer transferSyncer;
    private final SyncConfig config;

    public void run() throws InterruptedException {
        log.info("Sync job started, poll interval={}ms", config.pollIntervalMs());
        while (!Thread.currentThread().isInterrupted()) {
            try {
                boolean accountsBusy = accountSyncer.syncOneBatch();
                boolean transfersBusy = transferSyncer.syncOneBatch();

                if (!accountsBusy && !transfersBusy) {
                    Thread.sleep(config.pollIntervalMs());
                }
                // if either was busy (full batch), loop immediately
            } catch (Exception e) {
                log.error("Sync error, retrying after backoff", e);
                Thread.sleep(config.pollIntervalMs() * 2);
            }
        }
    }
}
```

### `AccountSyncer.java` / `TransferSyncer.java`

```java
// returns true if batch was full (more pages likely available)
public boolean syncOneBatch() {
    long cursor = cursorStore.get("accounts");
    List<AccountBatch> batch = reader.queryAccounts(cursor + 1, config.batchSize());

    if (batch.isEmpty()) return false;

    writer.upsertAccounts(batch);

    long newCursor = batch.get(batch.size() - 1).getTimestamp();
    cursorStore.set("accounts", newCursor);

    log.info("Synced {} accounts, cursor={}", batch.size(), newCursor);
    return batch.size() == config.batchSize();
}
```

---

## Configuration (`.env`)

```env
# TigerBeetle cluster
TB_ADDRESSES=localhost:3000
TB_CLUSTER_ID=0

# External database
DB_JDBC_URL=jdbc:postgresql://localhost:5432/mydb
DB_USER=sync_user
DB_PASSWORD=secret

# Sync tuning
SYNC_BATCH_SIZE=1000
SYNC_POLL_INTERVAL_MS=5000
```

---

## u128 Handling in Java

TigerBeetle uses `u128` for IDs. The Java client exposes them as two `long` values (high/low bits).
Convert to `BigInteger` for JDBC:

```java
// in DatabaseWriter.java
BigInteger toU128(long high, long low) {
    return BigInteger.valueOf(high).shiftLeft(64)
            .add(BigInteger.valueOf(low).and(BigInteger.valueOf(Long.MAX_VALUE).shiftLeft(1).add(BigInteger.ONE)));
}

// or use the helper from the TB Java client if available:
// UInt128.asBigInteger(high, low)
```

Store as `NUMERIC(39)` in PostgreSQL (fits full u128 range: 0 to 2^128−1).

---

## Deployment Options

| Option                 | Notes                                                          |
|------------------------|----------------------------------------------------------------|
| **Standalone JAR**     | `java -jar sync-job.jar` — simplest, run via systemd or Docker |
| **Docker container**   | Alongside manager-node containers on same host                 |
| **Kubernetes CronJob** | If you only need periodic sync, not continuous                 |
| **Spring Boot app**    | If you want `/health`, `/metrics` endpoints                    |

For continuous sync (recommended), run as a long-lived process — not a cron job — since
TB timestamps are monotonic and the polling loop is efficient (no duplicate work).

---

## What This Does NOT Need

- gRPC connection to manager-node — sync job is fully independent
- TigerBeetle WAL or data file access — uses the official client API only
- Awareness of checkpoint state or VSR protocol — TB client handles all of that transparently

---

## Related Docs

- [`tigerbeetle-replica-sync.md`](./tigerbeetle-replica-sync.md) — how TigerBeetle handles internal replica mismatch
- [`tigerbeetle-data-file-analysis.md`](./tigerbeetle-data-file-analysis.md) — on-disk format reference
