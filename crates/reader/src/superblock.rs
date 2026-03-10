//! Superblock parsing.
//!
//! TigerBeetle maintains 4 redundant copies of its superblock at the start of
//! every data file. Each copy is `superblock_copy_size` bytes wide (24,576 B
//! for the default production config). We read all copies, validate each, and
//! return the one with the highest `sequence` number.
//!
//! ## SuperBlockHeader layout (key fields only)
//!
//! | Abs. offset | Field                          | Type |
//! |-------------|--------------------------------|------|
//! | 32          | copy                           | u16  |
//! | 34          | version                        | u16  |
//! | 40          | **sequence**                   | u64  |
//! | 48          | cluster                        | u128 |
//! | 96..2144    | vsr_state (VSRState, 2048 B)   |      |
//! |   └ 96..1120 | checkpoint (CheckpointState, 1024 B) | |
//! |       └ 648 | **manifest_oldest_address**    | u64  |
//! |       └ 656 | **manifest_newest_address**    | u64  |
//! |       └ 704 | **manifest_block_count**       | u32  |

use std::io::{Read, Seek, SeekFrom};

use crate::error::ReaderError;
use crate::layout::TBConfig;
use crate::types::{read_u32, read_u64};

// SuperBlockHeader field offsets (absolute, within each copy's header bytes).
const OFF_SEQUENCE: usize = 40;
const OFF_MANIFEST_NEWEST_ADDRESS: usize = 656; // 96 (vsr_state) + 560 (CheckpointState)
const OFF_MANIFEST_BLOCK_COUNT: usize = 704; // 96 (vsr_state) + 608 (CheckpointState)
// vsr_state.checkpoint.header.op: vsr_state@96 + CheckpointState.header@0 + Header.op@224
const OFF_CHECKPOINT_OP: usize = 320;

/// The SuperBlockHeader struct is 8,192 bytes for the default production config.
/// We only need to read this much from each copy.
const SUPERBLOCK_HEADER_SIZE: usize = 8_192;

/// Information extracted from the best (highest-sequence) superblock copy.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SuperblockInfo {
    /// Superblock sequence number (increments on each superblock write).
    /// A value > 0 confirms the cluster has started.
    pub(crate) sequence: u64,
    /// Grid block address of the newest manifest log block (linked-list head).
    pub(crate) manifest_newest_address: u64,
    /// Total number of manifest log blocks in the chain.
    pub(crate) manifest_block_count: u32,
    /// The `op` of the last committed checkpoint.
    /// WAL slots with `op > checkpoint_op` have not been checkpointed to LSM yet.
    /// Zero for clusters that have never checkpointed.
    pub(crate) checkpoint_op: u64,
}

/// Read all superblock copies and return info from the one with the highest
/// valid `sequence` number.
///
/// A copy is considered valid if its `sequence` is non-zero. Checksum
/// verification is not performed; the sequence number alone is used to elect
/// the best copy.
pub(crate) fn read_superblock(
    file: &mut (impl Read + Seek),
    config: &TBConfig,
) -> Result<SuperblockInfo, ReaderError> {
    let read_size = SUPERBLOCK_HEADER_SIZE.min(config.superblock_copy_size as usize);
    let mut buf = vec![0u8; read_size];

    let mut best_sequence = 0u64;
    let mut best_info: Option<SuperblockInfo> = None;

    for copy_idx in 0..config.superblock_copies {
        let offset = config.superblock_copy_offset(copy_idx);
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut buf)?;

        let sequence = read_u64(&buf, OFF_SEQUENCE);
        if sequence == 0 {
            continue; // uninitialized or blank copy
        }

        if sequence > best_sequence {
            best_sequence = sequence;
            best_info = Some(SuperblockInfo {
                sequence,
                manifest_newest_address: read_u64(&buf, OFF_MANIFEST_NEWEST_ADDRESS),
                manifest_block_count: read_u32(&buf, OFF_MANIFEST_BLOCK_COUNT),
                checkpoint_op: read_u64(&buf, OFF_CHECKPOINT_OP),
            });
        }
    }

    best_info.ok_or_else(|| {
        ReaderError::InvalidSuperblock(
            "all 4 superblock copies have sequence=0; \
             the file may be unformatted or corrupted"
                .into(),
        )
    })
}
