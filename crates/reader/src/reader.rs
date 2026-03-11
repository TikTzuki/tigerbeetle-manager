//! High-level `DataFileReader` — the main entry point for applications.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::marker::PhantomData;
use std::path::Path;

use crate::block::{BLOCK_TYPE_INDEX, BLOCK_TYPE_VALUE, BlockHeader, HEADER_SIZE, OFF_METADATA};
use crate::error::ReaderError;
use crate::layout::TBConfig;
use crate::manifest::{
    collect_account_index_blocks, collect_transfer_index_blocks, count_live_index_blocks,
};
use crate::superblock::read_superblock;
use crate::types::{Account, Transfer, read_u32, read_u64};

const RECORD_SIZE: usize = 128;

/// Data file capacity statistics.
#[derive(Debug, Clone)]
pub struct CapacityStats {
    /// Total file size in bytes (= formatted capacity, since the file is pre-allocated).
    pub data_file_size_bytes: u64,
    /// Total number of grid blocks that fit in the data file.
    pub grid_blocks_total: u64,
    /// Number of grid blocks occupied by live LSM tables (index blocks only).
    /// This is a lower bound — it does not count value blocks, manifest blocks,
    /// or free-list blocks.
    pub grid_blocks_used: u64,
}

// WAL message header field offsets (within a 256-byte header slot).
const WAL_HDR_SIZE: usize = 96; // u32: total message size (header + body)
const WAL_HDR_OP: usize = 224; // u64: operation sequence number
const WAL_HDR_OPERATION: usize = 252; // u8:  operation code

// TableIndex::Metadata offsets within metadata_bytes (at header offset OFF_METADATA).
//
// | Offset in metadata | Field                 | Type |
// |--------------------|-----------------------|------|
// | 0                  | value_block_count     | u32  |
// | 4                  | value_block_count_max | u32  |
// | 8                  | key_size              | u32  |
const META_OFF_VALUE_BLOCK_COUNT: usize = OFF_METADATA;
const META_OFF_VALUE_BLOCK_COUNT_MAX: usize = OFF_METADATA + 4;
const META_OFF_KEY_SIZE: usize = OFF_METADATA + 8;

// ---------------------------------------------------------------------------
// TBObject trait — implemented by all record types that live in value blocks
// ---------------------------------------------------------------------------

/// Implemented by types that can be decoded from a 128-byte value-block slot.
///
/// This trait is sealed (all impls are in this crate) and is not part of the
/// public API.
pub(crate) trait TBObject: Sized {
    /// Decode from a 128-byte little-endian slice.
    fn from_bytes(b: &[u8]) -> Self;
    /// Return `true` for padding/sentinel slots that should be skipped.
    fn is_padding(&self) -> bool;
    /// WAL operation code for this record type.
    /// `create_accounts = 138`, `create_transfers = 139`.
    const WAL_OPERATION: u8;
}

impl TBObject for Account {
    fn from_bytes(b: &[u8]) -> Self {
        Account::from_bytes(b)
    }
    fn is_padding(&self) -> bool {
        // id=0: padding slot. id=u128::MAX: internal LSM sentinel key.
        self.id == 0 || self.id == u128::MAX
    }
    const WAL_OPERATION: u8 = 138;
}

impl TBObject for Transfer {
    fn from_bytes(b: &[u8]) -> Self {
        Transfer::from_bytes(b)
    }
    fn is_padding(&self) -> bool {
        self.id == 0 || self.id == u128::MAX
    }
    const WAL_OPERATION: u8 = 139;
}

// ---------------------------------------------------------------------------
// DataFileReader
// ---------------------------------------------------------------------------

/// Opens and reads a TigerBeetle data file to extract account and transfer records.
///
/// # LSM vs WAL
///
/// TigerBeetle stores committed data in two places:
///
/// - **LSM (checkpointed)** — flushed to the grid every ~960 ops. Records here
///   have up-to-date balance fields. Use [`iter_accounts`] / [`iter_transfers`]
///   or [`read_lsm_accounts`] / [`read_lsm_transfers`].
/// - **WAL (pre-checkpoint)** — ring buffer of the last 1024 prepares. Records
///   here have initial balance values as submitted by the client. Use
///   [`iter_wal_accounts`] / [`iter_wal_transfers`] or
///   [`read_wal_accounts`] / [`read_wal_transfers`].
///
/// [`iter_accounts`]: DataFileReader::iter_accounts
/// [`iter_transfers`]: DataFileReader::iter_transfers
/// [`read_lsm_accounts`]: DataFileReader::read_lsm_accounts
/// [`read_lsm_transfers`]: DataFileReader::read_lsm_transfers
/// [`iter_wal_accounts`]: DataFileReader::iter_wal_accounts
/// [`iter_wal_transfers`]: DataFileReader::iter_wal_transfers
/// [`read_wal_accounts`]: DataFileReader::read_wal_accounts
/// [`read_wal_transfers`]: DataFileReader::read_wal_transfers
#[derive(Debug)]
pub struct DataFileReader {
    file: File,
    config: TBConfig,
}

impl DataFileReader {
    /// Open a TigerBeetle data file using the **default production** layout.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ReaderError> {
        Self::open_with_config(path, TBConfig::default())
    }

    /// Open a TigerBeetle data file with a custom layout configuration.
    pub fn open_with_config(path: impl AsRef<Path>, config: TBConfig) -> Result<Self, ReaderError> {
        let file = File::open(path.as_ref())
            .map_err(|e| ReaderError::Io(format!("cannot open {:?}: {e}", path.as_ref())))?;
        Ok(DataFileReader { file, config })
    }

    /// Return capacity statistics for this data file.
    ///
    /// Reports the total file size, total grid block slots, and the number of
    /// grid blocks occupied by live LSM tables.
    pub fn capacity_stats(&mut self) -> Result<CapacityStats, ReaderError> {
        let file_size = self
            .file
            .metadata()
            .map_err(|e| ReaderError::Io(format!("metadata: {e}")))?
            .len();

        let grid_bytes = file_size.saturating_sub(self.config.grid_zone_start);
        let grid_blocks_total = grid_bytes / self.config.block_size;

        let sb = read_superblock(&mut self.file, &self.config)?;
        let grid_blocks_used = count_live_index_blocks(&mut self.file, &self.config, &sb)?;

        Ok(CapacityStats {
            data_file_size_bytes: file_size,
            grid_blocks_total,
            grid_blocks_used,
        })
    }

    // -----------------------------------------------------------------------
    // LSM (checkpointed) iterators
    // -----------------------------------------------------------------------

    /// Lazy streaming iterator over checkpointed accounts (LSM only).
    ///
    /// Returns `Err(NotCheckpointed)` if the cluster has started but has not
    /// yet triggered its first LSM checkpoint (~960 committed operations).
    /// Use [`iter_wal_accounts`] to read pre-checkpoint accounts from the WAL.
    ///
    /// [`iter_wal_accounts`]: DataFileReader::iter_wal_accounts
    pub fn iter_accounts(&mut self) -> Result<AccountIter<'_>, ReaderError> {
        let sb = read_superblock(&mut self.file, &self.config)?;
        if sb.manifest_block_count == 0 {
            if sb.sequence > 0 {
                return Err(ReaderError::NotCheckpointed {
                    sequence: sb.sequence,
                });
            }
            return Ok(ObjectIter::new(&mut self.file, &self.config, vec![]));
        }
        let index_addrs = collect_account_index_blocks(&mut self.file, &self.config, &sb)?;
        Ok(ObjectIter::new(&mut self.file, &self.config, index_addrs))
    }

    /// Lazy streaming iterator over checkpointed transfers (LSM only).
    ///
    /// Returns `Err(NotCheckpointed)` if no checkpoint has occurred yet.
    pub fn iter_transfers(&mut self) -> Result<TransferIter<'_>, ReaderError> {
        let sb = read_superblock(&mut self.file, &self.config)?;
        if sb.manifest_block_count == 0 {
            if sb.sequence > 0 {
                return Err(ReaderError::NotCheckpointed {
                    sequence: sb.sequence,
                });
            }
            return Ok(ObjectIter::new(&mut self.file, &self.config, vec![]));
        }
        let index_addrs = collect_transfer_index_blocks(&mut self.file, &self.config, &sb)?;
        Ok(ObjectIter::new(&mut self.file, &self.config, index_addrs))
    }

    /// Return a page of LSM accounts (0-based `page`, up to `limit` records).
    ///
    /// Streams the iterator — no full intermediate `Vec` is built.
    /// Returns an empty `Vec` on a page beyond the last record, or if no
    /// checkpoint has occurred yet.
    pub fn read_lsm_accounts(
        &mut self,
        page: usize,
        limit: usize,
    ) -> Result<Vec<Account>, ReaderError> {
        match self.iter_accounts() {
            Ok(iter) => iter
                .skip(page * limit)
                .take(limit)
                .collect::<Result<Vec<_>, _>>(),
            Err(ReaderError::NotCheckpointed { .. }) => Ok(vec![]),
            Err(e) => Err(e),
        }
    }

    /// Return a page of LSM transfers (0-based `page`, up to `limit` records).
    pub fn read_lsm_transfers(
        &mut self,
        page: usize,
        limit: usize,
    ) -> Result<Vec<Transfer>, ReaderError> {
        match self.iter_transfers() {
            Ok(iter) => iter
                .skip(page * limit)
                .take(limit)
                .collect::<Result<Vec<_>, _>>(),
            Err(ReaderError::NotCheckpointed { .. }) => Ok(vec![]),
            Err(e) => Err(e),
        }
    }

    // -----------------------------------------------------------------------
    // WAL (pre-checkpoint) iterators
    // -----------------------------------------------------------------------

    /// Lazy streaming iterator over WAL accounts not yet flushed to the LSM.
    ///
    /// Scans WAL header slots for `create_accounts` prepares with
    /// `op > checkpoint_op`. Records carry initial balance values as submitted
    /// by the client (not current balances — those are in the LSM).
    pub fn iter_wal_accounts(&mut self) -> Result<WalAccountIter<'_>, ReaderError> {
        let sb = read_superblock(&mut self.file, &self.config)?;
        Ok(WalObjectIter::new(
            &mut self.file,
            &self.config,
            sb.checkpoint_op,
        ))
    }

    /// Lazy streaming iterator over WAL transfers not yet flushed to the LSM.
    pub fn iter_wal_transfers(&mut self) -> Result<WalTransferIter<'_>, ReaderError> {
        let sb = read_superblock(&mut self.file, &self.config)?;
        Ok(WalObjectIter::new(
            &mut self.file,
            &self.config,
            sb.checkpoint_op,
        ))
    }

    /// Return a page of WAL accounts (0-based `page`, up to `limit` records).
    ///
    /// Streams the WAL iterator — no full intermediate `Vec` is built.
    pub fn read_wal_accounts(
        &mut self,
        page: usize,
        limit: usize,
    ) -> Result<Vec<Account>, ReaderError> {
        self.iter_wal_accounts()?
            .skip(page * limit)
            .take(limit)
            .collect::<Result<Vec<_>, _>>()
    }

    /// Return a page of WAL transfers (0-based `page`, up to `limit` records).
    pub fn read_wal_transfers(
        &mut self,
        page: usize,
        limit: usize,
    ) -> Result<Vec<Transfer>, ReaderError> {
        self.iter_wal_transfers()?
            .skip(page * limit)
            .take(limit)
            .collect::<Result<Vec<_>, _>>()
    }
}

// ---------------------------------------------------------------------------
// LSM streaming iterator (ObjectIter)
// ---------------------------------------------------------------------------

/// A lazy streaming iterator over object records in the LSM (checkpointed data).
///
/// Created by [`DataFileReader::iter_accounts`] (as [`AccountIter`]) or
/// [`DataFileReader::iter_transfers`] (as [`TransferIter`]).
/// Only one 512 KiB block buffer is held in memory at any time.
pub struct ObjectIter<'r, T> {
    file: &'r mut File,
    config: &'r TBConfig,

    /// Reused 512 KiB buffer — holds whichever block is currently being processed.
    block: Vec<u8>,

    /// Remaining index-block addresses (live LSM tables for this tree).
    index_addrs: std::vec::IntoIter<u64>,

    /// Value-block addresses extracted from the current index block.
    value_addrs: Vec<u64>,
    /// Next position in `value_addrs` to read.
    value_idx: usize,

    /// Number of records in the current value block (`header.body_size / 128`).
    record_count: usize,
    /// Next record slot to decode within the current value block.
    record_idx: usize,

    /// Set after the first error; causes `next()` to return `None` afterwards.
    done: bool,

    _phantom: PhantomData<T>,
}

/// Lazy streaming iterator over checkpointed [`Account`] records (LSM).
pub type AccountIter<'r> = ObjectIter<'r, Account>;

/// Lazy streaming iterator over checkpointed [`Transfer`] records (LSM).
pub type TransferIter<'r> = ObjectIter<'r, Transfer>;

impl<T> ObjectIter<'_, T> {
    fn new<'r>(
        file: &'r mut File,
        config: &'r TBConfig,
        index_addrs: Vec<u64>,
    ) -> ObjectIter<'r, T> {
        ObjectIter {
            file,
            config,
            block: vec![0u8; config.block_size as usize],
            index_addrs: index_addrs.into_iter(),
            value_addrs: Vec::new(),
            value_idx: 0,
            record_count: 0,
            record_idx: 0,
            done: false,
            _phantom: PhantomData,
        }
    }
}

impl<T> std::fmt::Debug for ObjectIter<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectIter")
            .field("record_idx", &self.record_idx)
            .field("record_count", &self.record_count)
            .field("value_idx", &self.value_idx)
            .field(
                "value_addrs_remaining",
                &(self.value_addrs.len() - self.value_idx),
            )
            .field("done", &self.done)
            .finish()
    }
}

impl<T: TBObject> Iterator for ObjectIter<'_, T> {
    type Item = Result<T, ReaderError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        loop {
            // --- Yield the next record from the current value block ---
            while self.record_idx < self.record_count {
                let off = HEADER_SIZE + self.record_idx * RECORD_SIZE;
                self.record_idx += 1;
                let record = T::from_bytes(&self.block[off..off + RECORD_SIZE]);
                if !record.is_padding() {
                    return Some(Ok(record));
                }
            }

            // --- Load the next value block ---
            if self.value_idx < self.value_addrs.len() {
                let addr = self.value_addrs[self.value_idx];
                self.value_idx += 1;
                match self.load_value_block(addr) {
                    Ok(()) => continue,
                    Err(e) => {
                        self.done = true;
                        return Some(Err(e));
                    }
                }
            }

            // --- Load the next index block ---
            let Some(index_addr) = self.index_addrs.next() else {
                return None;
            };
            match self.load_index_block(index_addr) {
                Ok(()) => continue,
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
            }
        }
    }
}

impl<T> ObjectIter<'_, T> {
    fn load_index_block(&mut self, address: u64) -> Result<(), ReaderError> {
        seek_and_read(self.file, self.config, address, &mut self.block)?;

        let header = BlockHeader::parse(&self.block)?;
        if header.block_type != BLOCK_TYPE_INDEX {
            return Err(ReaderError::InvalidBlock(format!(
                "expected index block at address {address}, got block_type={}",
                header.block_type
            )));
        }

        let value_count = read_u32(&self.block, META_OFF_VALUE_BLOCK_COUNT) as usize;
        let value_count_max = read_u32(&self.block, META_OFF_VALUE_BLOCK_COUNT_MAX) as usize;
        let key_size = read_u32(&self.block, META_OFF_KEY_SIZE) as usize;

        if value_count > value_count_max {
            return Err(ReaderError::InvalidBlock(format!(
                "index block {address}: value_count={value_count} \
                 exceeds value_count_max={value_count_max}"
            )));
        }

        let checksum_section = value_count_max.checked_mul(32).ok_or_else(|| {
            ReaderError::InvalidBlock("index block: checksum section overflow".into())
        })?;
        let keys_section = value_count_max.checked_mul(key_size).ok_or_else(|| {
            ReaderError::InvalidBlock("index block: keys section overflow".into())
        })?;
        let addr_section_offset = HEADER_SIZE + checksum_section + keys_section + keys_section;
        let addr_section_end = addr_section_offset
            .checked_add(value_count * 8)
            .ok_or_else(|| {
                ReaderError::InvalidBlock("index block: address section end overflow".into())
            })?;

        if addr_section_end > self.block.len() {
            return Err(ReaderError::InvalidBlock(format!(
                "index block {address}: address section end ({addr_section_end}) \
                 exceeds block size ({})",
                self.block.len()
            )));
        }

        self.value_addrs.clear();
        self.value_addrs.reserve(value_count);
        for i in 0..value_count {
            let addr = read_u64(&self.block, addr_section_offset + i * 8);
            if addr != 0 {
                self.value_addrs.push(addr);
            }
        }
        self.value_idx = 0;
        self.record_count = 0;
        self.record_idx = 0;
        Ok(())
    }

    fn load_value_block(&mut self, address: u64) -> Result<(), ReaderError> {
        seek_and_read(self.file, self.config, address, &mut self.block)?;

        let header = BlockHeader::parse(&self.block)?;
        if header.block_type != BLOCK_TYPE_VALUE {
            return Err(ReaderError::InvalidBlock(format!(
                "expected value block at address {address}, got block_type={}",
                header.block_type
            )));
        }

        self.record_count = header.body_size() / RECORD_SIZE;
        self.record_idx = 0;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// WAL streaming iterator (WalObjectIter)
// ---------------------------------------------------------------------------

/// A lazy streaming iterator over WAL records not yet flushed to the LSM.
///
/// Created by [`DataFileReader::iter_wal_accounts`] (as [`WalAccountIter`]) or
/// [`DataFileReader::iter_wal_transfers`] (as [`WalTransferIter`]).
///
/// Scans WAL header slots sequentially and reads matching slot bodies on demand.
/// No large intermediate `Vec` is held — one body buffer is reused per slot.
pub struct WalObjectIter<'r, T> {
    file: &'r mut File,
    config: &'r TBConfig,
    checkpoint_op: u64,

    /// Next WAL slot index to examine (0..journal_slot_count).
    slot: usize,

    /// Reused body buffer for the currently loaded slot.
    body: Vec<u8>,
    /// Number of 128-byte records in `body`.
    record_count: usize,
    /// Next record to yield from `body`.
    record_idx: usize,

    /// Set after the first I/O error; causes subsequent `next()` to return `None`.
    done: bool,

    _phantom: PhantomData<T>,
}

/// Lazy streaming iterator over WAL [`Account`] records (pre-checkpoint).
pub type WalAccountIter<'r> = WalObjectIter<'r, Account>;

/// Lazy streaming iterator over WAL [`Transfer`] records (pre-checkpoint).
pub type WalTransferIter<'r> = WalObjectIter<'r, Transfer>;

impl<T> WalObjectIter<'_, T> {
    fn new<'r>(
        file: &'r mut File,
        config: &'r TBConfig,
        checkpoint_op: u64,
    ) -> WalObjectIter<'r, T> {
        WalObjectIter {
            file,
            config,
            checkpoint_op,
            slot: 0,
            body: Vec::new(),
            record_count: 0,
            record_idx: 0,
            done: false,
            _phantom: PhantomData,
        }
    }
}

impl<T> std::fmt::Debug for WalObjectIter<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WalObjectIter")
            .field("slot", &self.slot)
            .field("record_idx", &self.record_idx)
            .field("record_count", &self.record_count)
            .field("checkpoint_op", &self.checkpoint_op)
            .field("done", &self.done)
            .finish()
    }
}

impl<T: TBObject> Iterator for WalObjectIter<'_, T> {
    type Item = Result<T, ReaderError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        loop {
            // --- Yield the next record from the current slot body ---
            while self.record_idx < self.record_count {
                let off = self.record_idx * RECORD_SIZE;
                self.record_idx += 1;
                let record = T::from_bytes(&self.body[off..off + RECORD_SIZE]);
                if !record.is_padding() {
                    return Some(Ok(record));
                }
            }

            // --- Advance to the next matching WAL slot ---
            match self.load_next_matching_slot() {
                Ok(true) => continue,
                Ok(false) => return None,
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
            }
        }
    }
}

#[allow(private_bounds)]
impl<T: TBObject> WalObjectIter<'_, T> {
    /// Scan forward from `self.slot` until a slot matching `T::WAL_OPERATION`
    /// and `op > checkpoint_op` is found, then load its body.
    /// Returns `Ok(true)` on success, `Ok(false)` when all slots are exhausted.
    #[allow(private_bounds)]
    fn load_next_matching_slot(&mut self) -> Result<bool, ReaderError> {
        let slot_count = self.config.journal_slot_count as usize;
        let wal_headers_start = self.config.wal_headers_start();
        let wal_prepares_start = self.config.wal_prepares_start();
        let mut hdr = [0u8; 256];

        while self.slot < slot_count {
            let slot = self.slot;
            self.slot += 1;

            // Read the compact 256-byte header from the wal_headers zone.
            let hdr_offset = wal_headers_start + slot as u64 * 256;
            self.file
                .seek(SeekFrom::Start(hdr_offset))
                .map_err(|e| ReaderError::Io(format!("WAL header seek slot {slot}: {e}")))?;
            self.file
                .read_exact(&mut hdr)
                .map_err(|e| ReaderError::Io(format!("WAL header read slot {slot}: {e}")))?;

            if hdr[WAL_HDR_OPERATION] != T::WAL_OPERATION {
                continue;
            }

            let op = read_u64(&hdr, WAL_HDR_OP);
            if op == 0 || op <= self.checkpoint_op {
                continue;
            }

            let msg_size = read_u32(&hdr, WAL_HDR_SIZE) as usize;
            if msg_size <= 256 {
                continue;
            }
            let body_size = msg_size - 256;
            let record_count = body_size / RECORD_SIZE;
            if record_count == 0 {
                continue;
            }

            // Read the body from the wal_prepares zone.
            // Slot layout: [256-byte header][body...][padding to message_size_max].
            let body_offset = wal_prepares_start + slot as u64 * self.config.message_size_max + 256;
            self.file
                .seek(SeekFrom::Start(body_offset))
                .map_err(|e| ReaderError::Io(format!("WAL body seek slot {slot}: {e}")))?;
            self.body.resize(body_size, 0);
            self.file
                .read_exact(&mut self.body)
                .map_err(|e| ReaderError::Io(format!("WAL body read slot {slot}: {e}")))?;

            self.record_count = record_count;
            self.record_idx = 0;
            return Ok(true);
        }

        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// Shared I/O helper (LSM blocks)
// ---------------------------------------------------------------------------

fn seek_and_read(
    file: &mut File,
    config: &TBConfig,
    address: u64,
    block: &mut Vec<u8>,
) -> Result<(), ReaderError> {
    let offset = config.block_offset(address);
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| ReaderError::Io(format!("seek to block {address} (offset {offset}): {e}")))?;
    file.read_exact(block)
        .map_err(|e| ReaderError::Io(format!("read block {address}: {e}")))?;
    Ok(())
}
