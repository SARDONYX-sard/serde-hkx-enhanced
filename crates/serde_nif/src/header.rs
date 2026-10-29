//! File header (layout in the crate docs) and footer.

use std::borrow::Cow;

use crate::error::{Error, InvalidDataSnafu, InvalidHeaderSnafu, UnsupportedVersionSnafu};
use crate::reader::Reader;
use crate::writer::Writer;

/// 20.2.0.7: Skyrim LE/SE meshes and the KF files Skyrim tools exchange.
pub const VERSION: u32 = 0x1402_0007;
const HEADER_LINE: &str = "Gamebryo File Format, Version 20.2.0.7\n";

/// Everything in front of the first block that a reader needs.
#[derive(Debug, Default)]
pub struct Header<'a> {
    /// 12 for Skyrim meshes, 11 for KF animations written by niflib/hkxcmd.
    pub user_version: u32,
    /// Bethesda stream version: 83 = Skyrim LE, 100 = Skyrim SE. Several block
    /// layouts depend on it (e.g. SE packs vertices into `BSTriShape`).
    pub bs_version: u32,
    /// Type name of every block, e.g. `["BSFadeNode", "NiNode", "BSTriShape", ...]`.
    pub block_types: Vec<Cow<'a, str>>,
    /// Byte size of every block (stored since 20.2.0.5, so unknown blocks can be skipped).
    pub block_sizes: Vec<u32>,
    /// String table; blocks store `i32` indices into it (names of nodes, ...).
    pub strings: Vec<Cow<'a, str>>,
}

impl<'a> Header<'a> {
    /// Reads any 20.2.0.7 header; callers check `user_version`/`bs_version`
    /// for the block layouts they understand.
    pub fn read(reader: &mut Reader<'a>) -> Result<Self, Error> {
        let mut line = Vec::new();
        loop {
            match reader.u8() {
                Ok(b'\n') => break,
                Ok(byte) if line.len() < 128 => line.push(byte),
                _ => return InvalidHeaderSnafu.fail(),
            }
        }
        if !(line.starts_with(b"Gamebryo File Format")
            || line.starts_with(b"NetImmerse File Format"))
        {
            return InvalidHeaderSnafu.fail();
        }

        let version = reader.u32()?;
        let _endian = reader.u8()?;
        let user_version = reader.u32()?;
        let num_blocks = reader.u32()? as usize;
        let bs_version = reader.u32()?;
        if version != VERSION {
            return UnsupportedVersionSnafu {
                version,
                user_version,
                bs_version,
                expected: "20.2.0.7",
            }
            .fail();
        }

        // Export info: author, process script, export script (+ max filepath).
        let _author = reader.short_string()?;
        if bs_version > 130 {
            reader.u32()?;
        }
        if bs_version < 131 {
            let _process_script = reader.short_string()?;
        }
        let _export_script = reader.short_string()?;
        if bs_version >= 103 {
            let _max_filepath = reader.short_string()?;
        }

        let num_types = reader.u16()? as usize;
        let types = (0..num_types)
            .map(|_| reader.sized_string())
            .collect::<Result<Vec<_>, _>>()?;

        reader.check_len(num_blocks, 6)?;
        let block_types = (0..num_blocks)
            .map(|_| {
                // The high bit flags PhysX blocks; it is not part of the index.
                let index = (reader.u16()? & 0x7fff) as usize;
                types.get(index).cloned().ok_or_else(|| {
                    InvalidDataSnafu {
                        message: format!("block type index {index} out of range"),
                    }
                    .build()
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let block_sizes = (0..num_blocks)
            .map(|_| reader.u32())
            .collect::<Result<Vec<_>, _>>()?;

        let num_strings = reader.u32()? as usize;
        let _max_string_len = reader.u32()?;
        reader.check_len(num_strings, 4)?;
        let strings = (0..num_strings)
            .map(|_| reader.sized_string())
            .collect::<Result<Vec<_>, _>>()?;

        let num_groups = reader.u32()? as usize;
        reader.skip(num_groups.saturating_mul(4))?;

        Ok(Self {
            user_version,
            bs_version,
            block_types,
            block_sizes,
            strings,
        })
    }

    /// Decodes every block with `read`, each from a reader limited to the
    /// block's stored size, so a decoder that stops early cannot desync the rest.
    pub fn read_blocks<B>(
        &self,
        reader: &mut Reader<'a>,
        mut read: impl FnMut(&mut Reader<'a>, &str) -> Result<B, Error>,
    ) -> Result<Vec<B>, Error> {
        self.block_types
            .iter()
            .zip(&self.block_sizes)
            .enumerate()
            .map(|(index, (block_type, &size))| {
                let data = reader.take(size as usize)?;
                read(&mut Reader::new(data), block_type).map_err(|source| Error::Block {
                    index,
                    block_type: block_type.to_string(),
                    source: Box::new(source),
                })
            })
            .collect()
    }

    /// Writes the header; `block_types` and `block_sizes` must describe the
    /// blocks that follow. Type names are deduplicated in first-use order.
    pub fn write(&self, w: &mut Writer) {
        w.bytes(HEADER_LINE.as_bytes());
        w.u32(VERSION);
        w.u8(1); // little endian
        w.u32(self.user_version);
        w.u32(self.block_types.len() as u32);
        w.u32(self.bs_version);
        // Empty author / process script / export script.
        for _ in 0..3 {
            w.short_string("");
        }

        let mut types: Vec<&str> = Vec::new();
        let indices: Vec<u16> = self
            .block_types
            .iter()
            .map(|name| {
                let index = types.iter().position(|t| t == name).unwrap_or_else(|| {
                    types.push(name);
                    types.len() - 1
                });
                index as u16
            })
            .collect();
        w.u16(types.len() as u16);
        for t in types.iter() {
            w.sized_string(t);
        }
        for i in indices {
            w.u16(i);
        }
        self.block_sizes.iter().for_each(|&size| w.u32(size));

        w.u32(self.strings.len() as u32);
        w.u32(self.strings.iter().map(|s| s.len()).max().unwrap_or(0) as u32);
        self.strings.iter().for_each(|s| w.sized_string(s));
        w.u32(0); // groups
    }
}

/// Footer: the root block indices, falling back to block 0 when absent.
pub fn read_roots(reader: &mut Reader<'_>) -> Vec<usize> {
    reader
        .index_list()
        .ok()
        .map(|roots| roots.into_iter().flatten().collect::<Vec<_>>())
        .filter(|roots| !roots.is_empty())
        .unwrap_or_else(|| vec![0])
}

pub fn write_roots(w: &mut Writer, roots: &[usize]) {
    w.u32(roots.len() as u32);
    for &root in roots.iter() {
        w.index(Some(root));
    }
}
