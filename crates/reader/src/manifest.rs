//! Manifest log traversal.
//!
//! The manifest log is a linked list of grid blocks (type = 3, `manifest`).
//! Each block records a batch of `TableInfo` events — insert, update, or
//! remove — that describe the current state of the LSM forest's tables.
//!
//! We traverse the list **newest → oldest** (following
//! `previous_manifest_block_address` pointers) and collect the live set of
//! index-block addresses for a given LSM tree.
//!
//! ## Deduplication rule
//! Because we traverse newest-first, the first event we see for a given table
//! address is the most recent. We record whether it is live (insert/update)
//! or dead (remove) and skip subsequent events for the same address.
//!
//! ## ManifestNode::Metadata (inside `metadata_bytes` at header offset 128)
//!
//! | Offset in metadata | Field                         | Type |
//! |--------------------|-------------------------------|------|
//! | 0                  | prev_checksum                 | u128 |
//! | 16                 | prev_checksum_padding         | u128 |
//! | 32                 | **previous_manifest_block_address** | u64 |
//! | 40                 | **entry_count**               | u32  |
//!
//! ## TableInfo layout (128 bytes, in manifest block body after header)
//!
//! | Byte offset | Field      | Notes                                           |
//! |-------------|------------|-------------------------------------------------|
//! | 96          | address    | Index block address for this table              |
//! | 124         | tree_id    | 7 = Account objects tree, 18 = Transfer objects |
//! | 126         | label      | bits[5:0]=level, bits[7:6]=event (2-bit)        |

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};

use crate::block::{BLOCK_TYPE_MANIFEST, BlockHeader, HEADER_SIZE, OFF_METADATA};
use crate::error::ReaderError;
use crate::layout::TBConfig;
use crate::superblock::SuperblockInfo;
use crate::types::{ManifestEvent, TableInfo, read_u32, read_u64};

/// LSM tree ID for the Account objects tree (keyed by `timestamp`).
pub(crate) const ACCOUNT_OBJECTS_TREE_ID: u16 = 7;

/// LSM tree ID for the Transfer objects tree (keyed by `timestamp`).
pub(crate) const TRANSFER_OBJECTS_TREE_ID: u16 = 18;

const TABLE_INFO_SIZE: usize = 128;

// ManifestNode::Metadata field offsets relative to the start of metadata_bytes,
// i.e., absolute offsets within the block are `OFF_METADATA + X`.
const META_OFF_PREV_ADDRESS: usize = OFF_METADATA + 32;
const META_OFF_ENTRY_COUNT: usize = OFF_METADATA + 40;

/// Walk the manifest log and return the grid addresses of all **live** Account
/// index blocks (i.e., tables from tree_id == 7 whose last event is insert or
/// update).
///
/// Returns an empty `Vec` if the manifest log is empty (freshly formatted
/// file with no committed data).
pub(crate) fn collect_account_index_blocks(
    file: &mut (impl Read + Seek),
    config: &TBConfig,
    info: &SuperblockInfo,
) -> Result<Vec<u64>, ReaderError> {
    collect_index_blocks_for_tree(file, config, info, ACCOUNT_OBJECTS_TREE_ID)
}

/// Walk the manifest log and return the grid addresses of all **live** Transfer
/// index blocks (i.e., tables from tree_id == 18 whose last event is insert or
/// update).
///
/// Returns an empty `Vec` if the manifest log is empty (freshly formatted
/// file with no committed data).
pub(crate) fn collect_transfer_index_blocks(
    file: &mut (impl Read + Seek),
    config: &TBConfig,
    info: &SuperblockInfo,
) -> Result<Vec<u64>, ReaderError> {
    collect_index_blocks_for_tree(file, config, info, TRANSFER_OBJECTS_TREE_ID)
}

/// Count the total number of **live** index blocks across all LSM trees.
///
/// This walks the manifest log once and counts every table whose most-recent
/// event is insert or update (regardless of `tree_id`). The result is a lower
/// bound on grid block usage — it counts index blocks only, not their
/// associated value blocks, manifest blocks, or free-list blocks.
pub(crate) fn count_live_index_blocks(
    file: &mut (impl Read + Seek),
    config: &TBConfig,
    info: &SuperblockInfo,
) -> Result<u64, ReaderError> {
    if info.manifest_newest_address == 0 || info.manifest_block_count == 0 {
        return Ok(0);
    }

    let block_size = config.block_size as usize;
    let mut block = vec![0u8; block_size];
    let mut seen: HashMap<u64, bool> = HashMap::new();
    let mut current_address = info.manifest_newest_address;
    let mut blocks_visited: u32 = 0;

    while current_address != 0 {
        blocks_visited += 1;
        if blocks_visited > info.manifest_block_count + 1 {
            return Err(ReaderError::InvalidBlock(format!(
                "manifest chain longer than manifest_block_count={} (possible loop)",
                info.manifest_block_count
            )));
        }

        let offset = config.block_offset(current_address);
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut block)?;

        let header = BlockHeader::parse(&block)?;
        if header.block_type != BLOCK_TYPE_MANIFEST {
            return Err(ReaderError::InvalidBlock(format!(
                "expected manifest block at address {current_address}, \
                 got block_type={}",
                header.block_type
            )));
        }

        let prev_address = read_u64(&block, META_OFF_PREV_ADDRESS);
        let entry_count = read_u32(&block, META_OFF_ENTRY_COUNT) as usize;
        let max_entries = (block_size - HEADER_SIZE) / TABLE_INFO_SIZE;
        if entry_count > max_entries {
            return Err(ReaderError::InvalidBlock(format!(
                "manifest block at {current_address}: \
                 entry_count={entry_count} exceeds block capacity {max_entries}"
            )));
        }

        for i in 0..entry_count {
            let off = HEADER_SIZE + i * TABLE_INFO_SIZE;
            let Some(entry) = TableInfo::from_bytes(&block[off..off + TABLE_INFO_SIZE]) else {
                continue;
            };
            seen.entry(entry.address).or_insert(matches!(
                entry.event,
                ManifestEvent::Insert | ManifestEvent::Update
            ));
        }

        current_address = prev_address;
    }

    Ok(seen.values().filter(|&&live| live).count() as u64)
}

/// Walk the manifest log and collect live index-block addresses for the given `tree_id`.
///
/// Traverses newest → oldest, deduplicating by address (first-seen event wins).
fn collect_index_blocks_for_tree(
    file: &mut (impl Read + Seek),
    config: &TBConfig,
    info: &SuperblockInfo,
    tree_id: u16,
) -> Result<Vec<u64>, ReaderError> {
    if info.manifest_newest_address == 0 || info.manifest_block_count == 0 {
        return Ok(vec![]);
    }

    let block_size = config.block_size as usize;
    let mut block = vec![0u8; block_size];

    // Track the most-recent event per table address (first seen = most recent).
    let mut seen: HashMap<u64, bool> = HashMap::new();
    let mut current_address = info.manifest_newest_address;
    let mut blocks_visited: u32 = 0;

    while current_address != 0 {
        // Guard against corrupt pointer chains exceeding the declared block count.
        blocks_visited += 1;
        if blocks_visited > info.manifest_block_count + 1 {
            return Err(ReaderError::InvalidBlock(format!(
                "manifest chain longer than manifest_block_count={} (possible loop)",
                info.manifest_block_count
            )));
        }

        // Read and validate the manifest block.
        let offset = config.block_offset(current_address);
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut block)?;

        let header = BlockHeader::parse(&block)?;
        if header.block_type != BLOCK_TYPE_MANIFEST {
            return Err(ReaderError::InvalidBlock(format!(
                "expected manifest block at address {current_address}, \
                 got block_type={}",
                header.block_type
            )));
        }

        let prev_address = read_u64(&block, META_OFF_PREV_ADDRESS);
        let entry_count = read_u32(&block, META_OFF_ENTRY_COUNT) as usize;

        // Sanity-check: entry_count must fit in the block body.
        let max_entries = (block_size - HEADER_SIZE) / TABLE_INFO_SIZE;
        if entry_count > max_entries {
            return Err(ReaderError::InvalidBlock(format!(
                "manifest block at {current_address}: \
                 entry_count={entry_count} exceeds block capacity {max_entries}"
            )));
        }

        // Parse each TableInfo and record entries for the requested tree.
        for i in 0..entry_count {
            let off = HEADER_SIZE + i * TABLE_INFO_SIZE;
            let Some(entry) = TableInfo::from_bytes(&block[off..off + TABLE_INFO_SIZE]) else {
                continue;
            };

            if entry.tree_id != tree_id {
                continue;
            }

            // First time we encounter this table address = most recent event.
            seen.entry(entry.address).or_insert(matches!(
                entry.event,
                ManifestEvent::Insert | ManifestEvent::Update
            ));
        }

        current_address = prev_address;
    }

    // Collect all live table addresses.
    Ok(seen
        .into_iter()
        .filter_map(|(addr, live)| if live { Some(addr) } else { None })
        .collect())
}
