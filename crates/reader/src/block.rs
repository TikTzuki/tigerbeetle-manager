//! Grid block header parsing.
//!
//! Every block in TigerBeetle's grid zone begins with a 256-byte header
//! (`vsr.Header.Block` in the Zig source). The layout is:
//!
//! | Offset | Size | Field           | Notes                          |
//! |--------|------|-----------------|--------------------------------|
//! | 0      | 16   | checksum        | SipHash-128 of rest of header  |
//! | 16     | 16   | checksum_padding|                                |
//! | 32     | 16   | checksum_body   | SipHash-128 of block body      |
//! | 48     | 16   | checksum_body_padding |                          |
//! | 64     | 16   | nonce_reserved  |                                |
//! | 80     | 16   | cluster         | Cluster ID                     |
//! | 96     | 4    | size            | Total bytes used (header+body) |
//! | 100    | 4    | epoch           |                                |
//! | 104    | 4    | view            | Always 0 for blocks            |
//! | 108    | 4    | release         |                                |
//! | 112    | 2    | protocol        |                                |
//! | 114    | 1    | **command**     | Must be **20** (`block`)       |
//! | 115    | 1    | replica         | Always 0 for blocks            |
//! | 116    | 12   | reserved_frame  |                                |
//! | 128    | 96   | **metadata_bytes** | Block-type-specific schema  |
//! | 224    | 8    | **address**     | 1-indexed block address        |
//! | 232    | 8    | snapshot        |                                |
//! | 240    | 1    | **block_type**  | 3=manifest, 4=index, 5=value   |
//! | 241    | 15   | reserved_block  |                                |

use crate::error::ReaderError;
use crate::types::{read_u32, read_u64};

/// Size of the block header in bytes.
pub(crate) const HEADER_SIZE: usize = 256;

/// `Command` enum value for grid blocks (`Command.block = 20`).
pub(crate) const COMMAND_BLOCK: u8 = 20;

/// `BlockType` for manifest log blocks.
pub(crate) const BLOCK_TYPE_MANIFEST: u8 = 3;
/// `BlockType` for LSM index blocks.
pub(crate) const BLOCK_TYPE_INDEX: u8 = 4;
/// `BlockType` for LSM value blocks (contain actual records).
pub(crate) const BLOCK_TYPE_VALUE: u8 = 5;

/// Byte offset of `Header.Block.size` within the block.
pub(crate) const OFF_SIZE: usize = 96;
/// Byte offset of `Header.Block.command` within the block.
pub(crate) const OFF_COMMAND: usize = 114;
/// Byte offset of `Header.Block.metadata_bytes` within the block.
pub(crate) const OFF_METADATA: usize = 128;
/// Byte offset of `Header.Block.address` within the block.
pub(crate) const OFF_ADDRESS: usize = 224;
/// Byte offset of `Header.Block.block_type` within the block.
pub(crate) const OFF_BLOCK_TYPE: usize = 240;

/// Key fields extracted from a grid block header.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BlockHeader {
    /// Total bytes used by this block (header + body data, excluding padding).
    pub(crate) size: u32,
    /// Block type: 3 = manifest, 4 = index, 5 = value.
    pub(crate) block_type: u8,
}

impl BlockHeader {
    /// Parse and lightly validate a block header from the first 256 bytes of `block`.
    pub(crate) fn parse(block: &[u8]) -> Result<Self, ReaderError> {
        if block.len() < HEADER_SIZE {
            return Err(ReaderError::InvalidBlock(format!(
                "block too short: {} < {HEADER_SIZE}",
                block.len()
            )));
        }

        let command = block[OFF_COMMAND];
        if command != COMMAND_BLOCK {
            return Err(ReaderError::InvalidBlock(format!(
                "expected command=block({COMMAND_BLOCK}), got {command}"
            )));
        }

        let size = read_u32(block, OFF_SIZE);
        if (size as usize) < HEADER_SIZE {
            return Err(ReaderError::InvalidBlock(format!(
                "block header.size={size} < HEADER_SIZE={HEADER_SIZE}"
            )));
        }
        if (size as usize) > block.len() {
            return Err(ReaderError::InvalidBlock(format!(
                "block header.size={size} > buffer len {}",
                block.len()
            )));
        }

        let address = read_u64(block, OFF_ADDRESS);
        if address == 0 {
            return Err(ReaderError::InvalidBlock(
                "block address is 0 (null sentinel)".into(),
            ));
        }

        Ok(BlockHeader {
            size,
            block_type: block[OFF_BLOCK_TYPE],
        })
    }

    /// Number of bytes of body data (header.size minus the 256-byte header).
    pub(crate) fn body_size(&self) -> usize {
        (self.size as usize).saturating_sub(HEADER_SIZE)
    }
}
