//! Shared types and helpers used by rig import and export.
pub mod actor;
pub mod export;
pub mod export_all;
pub mod import;

use std::{
    io,
    path::{Path, PathBuf},
};

use serde_hkx_features::error::Error;

#[derive(Debug, Clone, PartialEq)]
/// Represents where converted animation files should be written.
enum Output {
    /// Writes the converted animation to a single file.
    File(PathBuf),

    /// Writes converted animations to a directory.
    Directory(PathBuf),
}

/// Creates an invalid-input error for CLI argument validation.
fn invalid_input(message: impl Into<String>) -> Error {
    Error::IoError {
        source: io::Error::new(io::ErrorKind::InvalidInput, message.into()),
    }
}

/// Is this a file extension supported by serde-hkx?
///
/// `.hkx`, `.xml`
fn is_serde_hkx_supported_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| serde_hkx_features::Format::from_extension(ext).is_ok())
}

/// Creates the parent directory required for an output file.
fn create_parent(path: &Path) -> Result<(), Error> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|source| Error::IoError { source })?;
    }

    Ok(())
}

/// Writes a file through a temporary file and a rename, so that threads
/// writing the same file concurrently never leave a partial file behind.
fn write_file_atomic(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    create_parent(path)?;

    let mut temp = path.as_os_str().to_owned();
    temp.push(format!(".{:?}.tmp", std::thread::current().id()).replace(['(', ')'], ""));
    let temp = PathBuf::from(temp);

    std::fs::write(&temp, bytes).map_err(|source| Error::IoError { source })?;
    std::fs::rename(&temp, path).map_err(|source| {
        let _ = std::fs::remove_file(&temp);
        Error::IoError { source }
    })
}

/// Writes converted animation bytes to the specified output path.
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    create_parent(path)?;

    std::fs::write(path, bytes).map_err(|source| Error::IoError { source })
}
