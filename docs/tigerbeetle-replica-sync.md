# TigerBeetle Replica Sync — How Replicas Detect and Fix Mismatches

> Source: `src/vsr/replica.zig`, `src/vsr/superblock.zig`, `src/vsr/sync.zig`
> TigerBeetle v0.16.x (VSR — Viewstamped Replication)

---

## Overview

TigerBeetle uses **VSR (Viewstamped Replication)** for consensus. Replica divergence
is handled by two distinct mechanisms depending on how far behind a replica is:

| Gap size                           | Mechanism      | What gets transferred                          |
|------------------------------------|----------------|------------------------------------------------|
| Small (within WAL window)          | **WAL repair** | Missing prepare messages from the journal      |
| Large (across checkpoint boundary) | **State sync** | Full checkpoint snapshot via grid block repair |

---

## 1. How Replicas Detect a Mismatch

### 1.1 Via `checkpoint_id` on every Prepare message

Every prepare message (from primary to backups) carries a `checkpoint_id` — a BLAKE3
hash of the current superblock checkpoint state. When a backup receives a prepare:

```zig
// replica.zig:2175
if (message.header.checkpoint_id != self.superblock.working.checkpoint_id() and
    message.header.checkpoint_id !=
        self.superblock.working.vsr_state.checkpoint.parent_checkpoint_id)
{
    // Checkpoint hash chain is broken — replica's state diverged from the cluster.
    log.err("{}: on_prepare: checkpoint diverged ...", ...);
    @panic("checkpoint diverged");
}
```

This is the **hard divergence check** — if a replica's checkpoint ID doesn't match the
primary's (or the parent checkpoint ID), it's a determinism violation and crashes immediately.

### 1.2 Via `StartView` messages during view changes

When a view change occurs, the new primary sends a `StartView` message containing:

- `commit_max` — the highest committed op in the cluster
- `checkpoint_op` — which checkpoint the cluster is currently on

A backup compares its own `op_checkpoint` against the SV message:

```zig
// on_start_view_set_checkpoint — how a replica detects it needs state sync
fn on_start_view_set_checkpoint(self: *Replica, message: *Message.StartView) bool {
    // Is the cluster's checkpoint at least 1 checkpoint ahead, and is that checkpoint durable?
    const far_behind = vsr.Checkpoint.durable(
        self.op_checkpoint_next() + constants.vsr_checkpoint_ops,
        message.header.commit_max
    );

    // Is WAL repair stuck (can't make progress via journal repair alone)?
    const likely_stuck = self.syncing == .idle and self.repair_stuck();

    if (!far_behind and !likely_stuck) return false;

    // → Trigger state sync: replace this replica's checkpoint with the cluster's
    self.sync_start_from_committing();
    ...
}
```

### 1.3 Via `repair_stuck()` — detecting WAL repair failure

```zig
fn repair_stuck(self: *const Replica) bool {
    if (self.commit_min == self.commit_max) return false;   // already caught up
    if (self.status == .recovering_head) return false;       // still recovering

    // "Stuck" if:
    // - Hash chain is broken (can't verify log continuity)
    const stuck_header = !self.valid_hash_chain(...);

    // - The next prepare to commit is dirty/missing in the journal
    const stuck_prepare = (commit_next_slot == null or self.journal.dirty.bit(commit_next_slot.?));

    // - Grid reads are queued but not completing
    const stuck_grid = !self.grid.read_global_queue.empty();

    return (stuck_header or stuck_prepare or stuck_grid);
}
```

---

## 2. State Comparison Fields

Replicas compare these fields to determine who is ahead:

| Field                         | Location                          | Description                                         |
|-------------------------------|-----------------------------------|-----------------------------------------------------|
| `checkpoint_id`               | Superblock + every Prepare header | BLAKE3 hash of checkpoint state; must match cluster |
| `op_checkpoint`               | Replica state                     | Op number of the last durable checkpoint            |
| `commit_max`                  | Replica state + SV messages       | Highest op committed across the cluster             |
| `commit_min`                  | Replica state                     | Highest op this replica has committed locally       |
| `view`                        | Replica state                     | Current VSR view number                             |
| `log_view`                    | Replica state                     | View at which the current log was established       |
| `sync_op_min` / `sync_op_max` | Superblock `vsr_state`            | Op range skipped by the last state sync             |

The **canonical** state is always the primary's in the current view.

---

## 3. Two Repair Paths

### Path A: WAL Repair (small gap, same checkpoint)

When a backup is a few ops behind but within the same checkpoint:

1. Backup sends `request_headers` to ask for missing WAL entries.
2. Peers respond with `headers` messages containing the prepare headers.
3. Backup sends `request_prepare` for each dirty/missing slot.
4. Primary (or any replica that has it) sends the full prepare body.
5. Backup writes prepares to its journal, then commits them in order.

```
Backup                     Primary
  |--- request_headers --->|
  |<--- headers -----------|
  |--- request_prepare --->|  (for each missing op)
  |<--- prepare -----------|
  | (write to journal, commit)
```

### Path B: State Sync (large gap, checkpoint behind)

When a replica is ≥1 checkpoint behind and WAL repair can't help (the WAL wraps around
and older prepares have been overwritten):

#### Step 1 — Detect need for sync

Triggered by `on_start_view_set_checkpoint()` when:

- `far_behind`: cluster is ≥2 checkpoints ahead of this replica, OR
- `likely_stuck`: repair is stuck and cluster is 1 checkpoint ahead

#### Step 2 — Cancel in-flight work

```zig
fn sync_start_from_committing(self: *Replica) void {
    // Transition through sync stages:
    // .idle → .canceling_commit → .canceling_grid → .updating_checkpoint → .idle
    self.sync_dispatch(.canceling_commit);
}
```

Ongoing grid reads, compaction, and commits are cancelled before sync begins.

#### Step 3 — Update superblock with target checkpoint

The `StartView` message contains the full checkpoint state (manifest addresses, free set,
VSR state). The lagging replica writes this directly into its superblock:

```zig
// sync.zig
.updating_checkpoint: vsr.CheckpointState  // target checkpoint from SV message
```

The superblock is updated with:

- New `checkpoint_id`
- New `manifest_oldest_address` / `manifest_newest_address`
- New `free_set` block address
- New `commit_max`, `op_checkpoint`, `sync_op_min`, `sync_op_max`

**`sync_op_min` / `sync_op_max`** mark the op range that was skipped — this tells the
replica (and its peers) that it doesn't have WAL entries for those ops and shouldn't try
to repair them.

#### Step 4 — Grid block repair (fetch actual data)

After the superblock is updated, the replica's grid is empty (it has checkpoint metadata
but no actual LSM data blocks). It must fetch all blocks from peers:

1. Reads the manifest blocks (linked list from `manifest_oldest_address`)
2. For each manifest entry: identifies which index + value blocks it's missing
3. Sends `request_blocks` messages to peers for each missing block
4. Peers respond with the actual 512 KiB block data
5. Replica writes blocks to its grid, verifying BLAKE3 checksums

```
Lagging Replica              Any peer
  |--- request_blocks ------> (up to grid_repair_request_max blocks per msg)
  |<--- blocks -------------- (512 KiB each, with checksum in header)
  | (write to grid, verify checksum)
  | (repeat until all blocks are local)
```

This continues until the `GridBlocksMissing` tracker shows zero missing blocks.

#### Step 5 — Resume normal operation

Once all grid blocks are local:

- Sync stage transitions back to `.idle`
- Replica resumes WAL repair for any remaining op gap (within the new checkpoint's WAL window)
- Replica participates in consensus normally

---

## 4. `checkpoint_id` Hash Chain

Checkpoints form a **linked hash chain** in the superblock:

```
superblock.checkpoint_id()          = BLAKE3(current checkpoint state)
superblock.parent_checkpoint_id     = checkpoint_id of the previous checkpoint
superblock.grandparent_checkpoint_id = parent_checkpoint_id of the parent
```

Every prepare carries the `checkpoint_id` of the checkpoint active when it was prepared.
This allows backups to detect if they've somehow ended up on a different fork of history —
a storage determinism bug that would cause an immediate panic.

---

## 5. `sync_op_min` / `sync_op_max` — The WAL Hole

After state sync, the superblock records the op range that was skipped:

```zig
// Stored in superblock.vsr_state:
sync_op_min: u64  // first op covered by the synced checkpoint
sync_op_max: u64  // last op that was skipped (not present in WAL)
```

On startup, if `sync_op_max != 0`, the replica knows it has a "hole" in its WAL and
treats ops in `[sync_op_min, sync_op_max]` as if they were already committed — it will
never try to WAL-repair them since the checkpoint already encodes their effects.

---

## 6. Full Sequence: Replica Far Behind

```
Lagging Replica (2 checkpoints behind)       Rest of cluster

1. Receives StartView in view change
   - Sees commit_max is far ahead of its op_checkpoint
   - on_start_view_set_checkpoint() → sync needed

2. sync_start_from_committing()
   - .idle → .canceling_commit → .canceling_grid
   - Cancels all in-flight grid ops

3. sync_superblock_update_start()
   - .canceling_grid → .updating_checkpoint
   - Writes target checkpoint into superblock (from SV message)
   - Records sync_op_min, sync_op_max

4. Superblock written with new checkpoint
   - .updating_checkpoint → .idle
   - Grid is now "empty" (has checkpoint metadata but no blocks)

5. Grid repair begins
   ←── request_blocks (missing manifest, index, value blocks)
   ──► blocks (512 KiB each)
   (repeat for all missing blocks, verified by checksum)

6. All blocks present → repair complete
   - WAL repair (if any remaining gap)
   - Resume normal commit + consensus
```

---

## 7. Key Invariants

1. **Checkpoint ID must match** — any replica receiving a prepare with a mismatched checkpoint ID panics. There is no
   recovery from determinism violations.
2. **Grid blocks are content-addressed** — every block's BLAKE3 checksum is in its header. A repaired block that fails
   its checksum is rejected and re-requested.
3. **State sync skips WAL entirely** — when syncing, the replica gets the LSM state directly. It does NOT replay
   individual operations.
4. **Sync is checkpoint-to-checkpoint** — a replica always syncs to a complete checkpoint boundary, never to a mid-WAL
   position.
5. **`sync_op_min/max` persists across restarts** — stored in the superblock so the replica knows on re-open that it has
   a WAL hole and shouldn't attempt to repair those ops.
