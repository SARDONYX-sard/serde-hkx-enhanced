//! Read Skyrim LE (v104) and SE (v105) BSA archives.
//!
//! Only the index is loaded on [`Archive::open`]; file contents are read on
//! demand with a seek, so multi-gigabyte texture archives are cheap to open.
//!
//! Layout:
//!
//! ```text
//! header (36 bytes)
//! folder records      v104: hash u64, count u32, offset u32
//!                     v105: hash u64, count u32, pad u32, offset u64
//! file record blocks  per folder: bzstring name, count x (hash u64, size u32, offset u32)
//! file names          null-terminated, in file record order
//! file data           [bstring full path if embedded] [u32 original size + data if compressed]
//! ```
//!
//! ref: <https://en.uesp.net/wiki/Skyrim_Mod:Archive_File_Format>
mod data_dir;

pub use data_dir::DataDir;

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use snafu::{ResultExt as _, Snafu};

#[derive(Debug, Snafu)]
pub enum Error {
    /// I/O error on the archive file.
    #[snafu(display("{}: {source}", path.display()))]
    Io { path: PathBuf, source: std::io::Error },

    /// The file is not a supported BSA archive.
    #[snafu(display("{}: {message}", path.display()))]
    Invalid { path: PathBuf, message: String },

    /// A file could not be decompressed.
    #[snafu(display("{}: failed to decompress '{name}': {message}", path.display()))]
    Decompress {
        path: PathBuf,
        name: String,
        message: String,
    },
}

const FLAG_DIRECTORY_NAMES: u32 = 0x1;
const FLAG_FILE_NAMES: u32 = 0x2;
const FLAG_COMPRESSED: u32 = 0x4;
const FLAG_EMBED_NAMES: u32 = 0x100;

/// Toggles the archive-wide compression flag for one file.
const SIZE_COMPRESSION_TOGGLE: u32 = 0x4000_0000;
const SIZE_MASK: u32 = 0x3fff_ffff;

#[derive(Debug, Clone, Copy)]
struct Entry {
    offset: u64,
    size: u32,
    compressed: bool,
}

/// An opened BSA archive.
#[derive(Debug)]
pub struct Archive {
    path: PathBuf,
    version: u32,
    embed_names: bool,
    /// Normalized path (`lowercase`, `/` separated) -> entry.
    entries: HashMap<String, Entry>,
}

/// Normalizes an archive path: lowercase with `/` separators.
pub fn normalize(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches('/').to_lowercase()
}

impl Archive {
    /// Reads the index of a BSA archive.
    ///
    /// # Errors
    /// If the file cannot be read or is not a v104/v105 archive with
    /// directory and file names.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path).context(IoSnafu { path: &path })?;

        let mut header = [0u8; 36];
        file.read_exact(&mut header).context(IoSnafu { path: &path })?;
        let u32_at = |i: usize| u32::from_le_bytes([header[i], header[i + 1], header[i + 2], header[i + 3]]);

        let invalid = |message: &str| Error::Invalid {
            path: path.clone(),
            message: message.to_owned(),
        };
        if &header[..4] != b"BSA\0" {
            return Err(invalid("not a BSA archive"));
        }
        let version = u32_at(4);
        if version != 104 && version != 105 {
            return Err(invalid(&format!("unsupported BSA version {version}")));
        }
        let flags = u32_at(12);
        if flags & (FLAG_DIRECTORY_NAMES | FLAG_FILE_NAMES) != (FLAG_DIRECTORY_NAMES | FLAG_FILE_NAMES) {
            return Err(invalid("archives without directory/file names are not supported"));
        }
        let folder_count = u32_at(16) as usize;
        let file_count = u32_at(20) as usize;
        let total_file_name_length = u32_at(28) as usize;

        // Folder records.
        let record_size = if version == 105 { 24 } else { 16 };
        let mut folders = vec![0u8; folder_count * record_size];
        file.read_exact(&mut folders).context(IoSnafu { path: &path })?;
        let folder_file_counts: Vec<usize> = folders
            .chunks_exact(record_size)
            .map(|r| u32::from_le_bytes([r[8], r[9], r[10], r[11]]) as usize)
            .collect();

        // File record blocks.
        let mut records = Vec::with_capacity(file_count);
        let mut reader = std::io::BufReader::new(&mut file);
        for &count in &folder_file_counts {
            let mut len = [0u8; 1];
            reader.read_exact(&mut len).context(IoSnafu { path: &path })?;
            let mut name = vec![0u8; len[0] as usize];
            reader.read_exact(&mut name).context(IoSnafu { path: &path })?;
            let folder = String::from_utf8_lossy(name.strip_suffix(&[0]).unwrap_or(&name)).into_owned();

            let mut block = vec![0u8; count * 16];
            reader.read_exact(&mut block).context(IoSnafu { path: &path })?;
            for r in block.as_chunks::<16>().0 {
                let size = u32::from_le_bytes([r[8], r[9], r[10], r[11]]);
                let offset = u32::from_le_bytes([r[12], r[13], r[14], r[15]]);
                records.push((folder.clone(), size, offset));
            }
        }

        // File names.
        let mut names = vec![0u8; total_file_name_length];
        reader.read_exact(&mut names).context(IoSnafu { path: &path })?;
        let names: Vec<&[u8]> = names.split(|&b| b == 0).collect();
        if names.len() < records.len() {
            return Err(invalid("file name block is shorter than the file count"));
        }

        let default_compressed = flags & FLAG_COMPRESSED != 0;
        let entries = records
            .into_iter()
            .zip(names)
            .map(|((folder, size, offset), name)| {
                let name = String::from_utf8_lossy(name);
                let entry = Entry {
                    offset: u64::from(offset),
                    size: size & SIZE_MASK,
                    compressed: default_compressed != (size & SIZE_COMPRESSION_TOGGLE != 0),
                };
                (normalize(&format!("{folder}/{name}")), entry)
            })
            .collect();

        Ok(Self {
            path,
            version,
            embed_names: flags & FLAG_EMBED_NAMES != 0,
            entries,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Normalized paths of every file (`lowercase`, `/` separated).
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    pub fn contains(&self, path: &str) -> bool {
        self.entries.contains_key(&normalize(path))
    }

    /// Reads and decompresses one file, or `Ok(None)` if it is not in the archive.
    ///
    /// # Errors
    /// If reading or decompression fails.
    pub fn read(&self, path: &str) -> Result<Option<Vec<u8>>, Error> {
        let name = normalize(path);
        let Some(entry) = self.entries.get(&name).copied() else {
            return Ok(None);
        };

        let mut file = File::open(&self.path).context(IoSnafu { path: &self.path })?;
        file.seek(SeekFrom::Start(entry.offset))
            .context(IoSnafu { path: &self.path })?;
        let mut data = vec![0u8; entry.size as usize];
        file.read_exact(&mut data).context(IoSnafu { path: &self.path })?;

        let mut data = data.as_slice();
        if self.embed_names {
            let len = data.first().copied().unwrap_or(0) as usize;
            data = data.get(1 + len..).unwrap_or_default();
        }
        if !entry.compressed {
            return Ok(Some(data.to_vec()));
        }

        let decompress_error = |message: String| Error::Decompress {
            path: self.path.clone(),
            name: name.clone(),
            message,
        };
        let (size, compressed) = data
            .split_first_chunk::<4>()
            .ok_or_else(|| decompress_error("missing original size".to_owned()))?;
        let original_size = u32::from_le_bytes(*size) as usize;

        let bytes = if self.version == 105 {
            let mut out = Vec::with_capacity(original_size);
            lz4_flex::frame::FrameDecoder::new(compressed)
                .read_to_end(&mut out)
                .map_err(|e| decompress_error(e.to_string()))?;
            out
        } else {
            miniz_oxide::inflate::decompress_to_vec_zlib(compressed)
                .map_err(|e| decompress_error(format!("{e:?}")))?
        };
        Ok(Some(bytes))
    }
}

/// Several archives searched in order (later archives override earlier ones,
/// as the game loads them).
#[derive(Debug, Default)]
pub struct ArchiveSet {
    archives: Vec<Archive>,
}

impl ArchiveSet {
    /// Opens every archive in `paths`.
    ///
    /// # Errors
    /// If an archive cannot be opened.
    pub fn open<P: AsRef<Path>>(paths: impl IntoIterator<Item = P>) -> Result<Self, Error> {
        Ok(Self {
            archives: paths.into_iter().map(Archive::open).collect::<Result<_, _>>()?,
        })
    }

    pub fn archives(&self) -> &[Archive] {
        &self.archives
    }

    /// Normalized paths of every file in all archives (may contain duplicates).
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.archives.iter().flat_map(Archive::paths)
    }

    /// Reads `path` from the last archive that contains it.
    ///
    /// # Errors
    /// If reading or decompression fails.
    pub fn read(&self, path: &str) -> Result<Option<Vec<u8>>, Error> {
        for archive in self.archives.iter().rev() {
            if archive.contains(path) {
                return archive.read(path);
            }
        }
        Ok(None)
    }
}
