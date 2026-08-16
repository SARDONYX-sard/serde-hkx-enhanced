//! Shared types and helpers used by rig import and export.
pub mod export;
pub mod import;

use std::{
    io,
    path::{Path, PathBuf},
};

use serde_hkx_features::error::Error;

#[derive(Debug)]
/// Represents where converted animation files should be written.
enum Output {
    /// Writes the converted animation to a single file.
    File(PathBuf),

    /// Writes converted animations to a directory.
    Directory(PathBuf),
}

#[derive(Debug)]
/// Represents an animation file and its path relative to the input root.
struct AnimationFile {
    /// Original input file path.
    pub path: PathBuf,

    /// Path relative to the input root, used to preserve the directory structure.
    pub relative_path: PathBuf,

    /// Raw animation file contents.
    pub bytes: Vec<u8>,
}

/// Creates an invalid-input error for CLI argument validation.
fn invalid_input(message: impl Into<String>) -> Error {
    Error::IoError {
        source: io::Error::new(io::ErrorKind::InvalidInput, message.into()),
    }
}

/// Creates an invalid-data error for malformed or incompatible animation data.
fn invalid_data(message: impl Into<String>) -> Error {
    Error::IoError {
        source: io::Error::new(io::ErrorKind::InvalidData, message.into()),
    }
}

/// Checks whether a path has the specified file extension.
fn is_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
}

/// Returns the path relative to the specified input root.
fn relative_path(root: &Path, path: &Path) -> Result<PathBuf, Error> {
    path.strip_prefix(root).map(Path::to_owned).map_err(|_| {
        invalid_data(format!(
            "failed to determine relative path: {}",
            path.display()
        ))
    })
}

/// Builds an output path while preserving the input directory structure.
fn output_path(directory: &Path, relative_path: &Path, extension: &str) -> PathBuf {
    directory.join(relative_path).with_extension(extension)
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

/// Writes converted animation bytes to the specified output path.
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    create_parent(path)?;

    std::fs::write(path, bytes).map_err(|source| Error::IoError { source })
}
