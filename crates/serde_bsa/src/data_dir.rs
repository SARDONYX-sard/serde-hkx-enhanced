//! A Skyrim `Data` directory as one virtual file system.
//!
//! Loose files override archives, as in the game. Archives are opened in an
//! approximation of the game's load order: the base game archives first,
//! then the official DLCs, then every other archive alphabetically.
//!
//! Extra loose directories laid out like `Data` (e.g. a cache of extracted
//! files) can be layered on top with [`DataDir::with_overlay`].

use std::path::{Path, PathBuf};

use crate::{ArchiveSet, Error, IoSnafu, normalize};
use snafu::ResultExt as _;

#[derive(Debug)]
pub struct DataDir {
    /// Loose file roots, highest priority first.
    roots: Vec<PathBuf>,
    archives: ArchiveSet,
}

/// Base game / DLC archive name prefixes, in load order.
const LOAD_ORDER: &[&str] = &["skyrim - ", "update", "dawnguard", "hearthfires", "dragonborn"];

fn load_order_key(name: &str) -> (usize, String) {
    let lower = name.to_lowercase();
    let rank = LOAD_ORDER
        .iter()
        .position(|prefix| lower.starts_with(prefix))
        .unwrap_or(LOAD_ORDER.len());
    (rank, lower)
}

impl DataDir {
    /// Opens `root` (a `Data` directory) and every `*.bsa` directly in it.
    ///
    /// # Errors
    /// If the directory cannot be listed or an archive cannot be opened.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, Error> {
        let root = root.as_ref().to_path_buf();
        let mut archives: Vec<PathBuf> = std::fs::read_dir(&root)
            .context(IoSnafu { path: &root })?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("bsa")))
            .collect();
        archives.sort_by_key(|path| load_order_key(&path.file_name().unwrap_or_default().to_string_lossy()));

        Ok(Self {
            archives: ArchiveSet::open(&archives)?,
            roots: vec![root],
        })
    }

    /// Adds a loose directory that takes priority over everything else.
    #[must_use]
    pub fn with_overlay(mut self, root: impl Into<PathBuf>) -> Self {
        self.roots.insert(0, root.into());
        self
    }

    /// The `Data` directory (lowest priority loose root).
    pub fn root(&self) -> &Path {
        self.roots.last().map_or_else(|| Path::new(""), PathBuf::as_path)
    }

    pub const fn archives(&self) -> &ArchiveSet {
        &self.archives
    }

    /// Reads a file (`meshes/actors/cow/...`), loose files first.
    ///
    /// # Errors
    /// If reading or decompression fails.
    pub fn read(&self, path: &str) -> Result<Option<Vec<u8>>, Error> {
        let relative = normalize(path);
        for root in &self.roots {
            let loose = root.join(&relative);
            if loose.is_file() {
                return std::fs::read(&loose).map(Some).context(IoSnafu { path: loose });
            }
        }
        self.archives.read(path)
    }

    /// Normalized paths of every file under `prefix` (e.g. `meshes/actors/`),
    /// from archives and loose files, sorted and deduplicated.
    pub fn paths_under(&self, prefix: &str) -> Vec<String> {
        let prefix = normalize(prefix);
        let mut paths: Vec<String> = self
            .archives
            .paths()
            .filter(|p| p.starts_with(&prefix))
            .map(str::to_owned)
            .collect();

        for root in &self.roots {
            let mut stack = vec![root.join(&prefix)];
            while let Some(dir) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for entry in entries.filter_map(Result::ok) {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if let Ok(relative) = path.strip_prefix(root) {
                        paths.push(normalize(&relative.to_string_lossy()));
                    }
                }
            }
        }

        paths.sort_unstable();
        paths.dedup();
        paths
    }
}
