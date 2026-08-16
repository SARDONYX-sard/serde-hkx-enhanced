//! Export Havok HKX animations to KF or FBX.

use std::{
    fs,
    path::{Path, PathBuf},
};

use clap::ValueEnum;
use rayon::prelude::*;
use serde_hkx_features::error::Error;

#[cfg(feature = "kf")]
use niflib_animation::ser::{AnimationInput as KfAnimationInput, to_kf_bytes_vec};
#[cfg(feature = "fbx")]
use serde_fbx::ser::{AnimationInput as FbxAnimationInput, export_fbx};

use crate::AnyError;

use super::{
    AnimationFile, Output, invalid_data, invalid_input, is_extension,
    is_serde_hkx_supported_extension, output_path, relative_path, write_file,
};

pub const EXAMPLES: &str = color_print::cstr!(
    r#"Examples

- <blue!>export a single animation to KF</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations/idle.hkx <cyan!>-v</cyan!> kf
- <blue!>export a single animation to FBX</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations/idle.hkx <cyan!>-v</cyan!> fbx

- <blue!>export all animations in a directory</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations <cyan!>-v</cyan!> fbx

- <blue!>export multiple animations</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.hkx ./walk.hkx <cyan!>-v</cyan!> kf
- <blue!>specify an output directory</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations <cyan!>-o</cyan!> ./out <cyan!>-v</cyan!> fbx

- <blue!>specify an output file for a single animation</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations/idle.hkx <cyan!>-o</cyan!> ./idle.kf <cyan!>-v</cyan!> kf
Output behavior:
  - Without --output, files are written to ./output/.
  - A single animation may use an explicit .kf or .fbx output file.
  - Multiple animations require an output directory.
  - A directory input is always exported into an output directory.
  - Directory inputs preserve their relative path structure.
  - A non-existing output path ending in .kf or .fbx is treated as a file.
  - Other non-existing output paths are treated as directories.
  - The output file extension must match --format.
"#
);

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Format {
    #[cfg(feature = "kf")]
    Kf,

    #[cfg(feature = "fbx")]
    Fbx,
    #[cfg(feature = "fbx")]
    FbxAscii,
}

impl Format {
    /// Returns the file extension associated with the output format.
    const fn extension(self) -> &'static str {
        match self {
            #[cfg(feature = "kf")]
            Self::Kf => "kf",

            #[cfg(feature = "fbx")]
            Self::Fbx | Self::FbxAscii => "fbx",
        }
    }
}

#[derive(Debug, clap::Args)]
#[clap(arg_required_else_help = true, after_long_help = EXAMPLES)]
pub(crate) struct Args {
    /// Skeleton HKX path.
    #[clap(short = 's', long, value_name = "SKELETON")]
    pub skeleton: PathBuf,

    /// One or more animation HKX files or directories.
    ///
    /// Directories are searched for HKX files. Multiple files and directories
    /// may be specified.
    #[clap(short = 'a', long, value_name = "ANIMATION", num_args = 1..)]
    pub animations: Vec<PathBuf>,

    /// Output directory or an explicit .kf/.fbx file for a single animation.
    ///
    /// Defaults to ./output/.
    #[clap(short, long, value_name = "OUTPUT")]
    pub output: Option<PathBuf>,

    /// Output format.
    #[clap(short = 'v', long, value_enum, value_name = "FORMAT")]
    pub format: Format,

    /// Do not recurse into animation directories.
    #[clap(short = 'n', long)]
    pub no_recursive: bool,

    /// Frames per second used when exporting FBX.
    #[cfg(feature = "fbx")]
    #[clap(long, default_value = "30.0")]
    pub fps: f32,
}

/// Exports Havok HKX animations to KF or FBX.
///
/// # Errors
///
/// Returns [`AnyError`] if an input path is invalid, an animation cannot be
/// read or converted, the output configuration is invalid, or an output file
/// cannot be written.
pub fn exportrig(args: &Args) -> Result<(), AnyError> {
    let (animations, has_directory_input) =
        resolve_animation_paths(&args.animations, !args.no_recursive)?;

    if animations.is_empty() {
        return Err(invalid_input("no HKX animation files were found").into());
    }

    let output = resolve_output(
        args.output.as_deref(),
        animations.len(),
        has_directory_input,
        args.format,
    )?;

    let skeleton_bytes = fs::read(&args.skeleton).map_err(|source| Error::FailedReadFile {
        source,
        path: args.skeleton.clone(),
    })?;

    match args.format {
        #[cfg(feature = "kf")]
        Format::Kf => export_kf(&skeleton_bytes, &args.skeleton, &animations, &output)?,

        #[cfg(feature = "fbx")]
        Format::Fbx | Format::FbxAscii => {
            if !args.fps.is_finite() || args.fps <= 0.0 {
                return Err(invalid_input("--fps must be a finite value greater than zero").into());
            }

            let format = match args.format {
                Format::Fbx => serde_fbx::ser::Format::FbxBin,
                Format::FbxAscii => serde_fbx::ser::Format::FbxAscii,
                Format::Kf => unreachable!(),
            };

            export_fbx_format(
                &skeleton_bytes,
                &args.skeleton,
                &animations,
                &output,
                args.fps,
                format,
            )?;
        }
    }

    Ok(())
}

/// Resolves animation files and preserves each directory input's relative paths.
fn resolve_animation_paths(
    inputs: &[PathBuf],
    recursive: bool,
) -> Result<(Vec<AnimationFile>, bool), Error> {
    let results = inputs
        .par_iter()
        .map(|input| {
            if input.is_file() {
                if !is_serde_hkx_supported_extension(input.as_path()) {
                    return Err(invalid_input(format!(
                        "animation input is not an HKX file: {}",
                        input.display()
                    )));
                }

                return collect_file(input);
            }

            if input.is_dir() {
                return collect_directory(input, recursive);
            }

            Err(invalid_input(format!(
                "animation path does not exist or is not a file/directory: {}",
                input.display()
            )))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let has_directory_input = results.iter().any(|(_, is_directory)| *is_directory);

    let mut animations = results
        .into_iter()
        .flat_map(|(animations, _)| animations)
        .collect::<Vec<_>>();

    animations.sort_by(|a, b| a.path.cmp(&b.path));
    animations.dedup_by(|a, b| a.path == b.path);

    Ok((animations, has_directory_input))
}

/// Collects a single explicitly specified HKX animation.
fn collect_file(path: &Path) -> Result<(Vec<AnimationFile>, bool), Error> {
    let bytes = fs::read(path).map_err(|source| Error::FailedReadFile {
        source,
        path: path.to_owned(),
    })?;

    let relative_path = path.file_name().map(PathBuf::from).ok_or_else(|| {
        invalid_input(format!(
            "animation path has no file name: {}",
            path.display()
        ))
    })?;

    Ok((
        vec![AnimationFile {
            path: path.to_owned(),
            relative_path,
            bytes,
        }],
        false,
    ))
}

/// Recursively discovers HKX animations and preserves paths relative to the input directory.
fn collect_directory(root: &Path, recursive: bool) -> Result<(Vec<AnimationFile>, bool), Error> {
    let mut paths = Vec::new();

    collect_paths(root, recursive, &mut paths)?;

    let animations = paths
        .into_par_iter()
        .map(|path| {
            let relative_path = relative_path(root, &path)?;

            let bytes = fs::read(&path).map_err(|source| Error::FailedReadFile {
                source,
                path: path.clone(),
            })?;

            Ok(AnimationFile {
                path,
                relative_path,
                bytes,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;

    Ok((animations, true))
}

/// Recursively collects HKX paths without reading their contents.
fn collect_paths(directory: &Path, recursive: bool, paths: &mut Vec<PathBuf>) -> Result<(), Error> {
    let entries = fs::read_dir(directory).map_err(|source| Error::IoError { source })?;

    for entry in entries {
        let entry = entry.map_err(|source| Error::IoError { source })?;
        let path = entry.path();

        let file_type = entry
            .file_type()
            .map_err(|source| Error::IoError { source })?;

        if file_type.is_dir() {
            if recursive {
                collect_paths(&path, true, paths)?;
            }
        } else if is_serde_hkx_supported_extension(path.as_path()) {
            paths.push(path);
        }
    }

    Ok(())
}

/// Resolves whether the output represents a file or directory.
fn resolve_output(
    output: Option<&Path>,
    animation_count: usize,
    has_directory_input: bool,
    format: Format,
) -> Result<Output, Error> {
    let output = output.unwrap_or_else(|| Path::new("output"));

    if output.exists() {
        if output.is_file() {
            if animation_count != 1 {
                return Err(invalid_input(
                    "an output file can only be used with a single animation",
                ));
            }

            validate_output_extension(output, format)?;

            return Ok(Output::File(output.to_owned()));
        }

        if output.is_dir() {
            return Ok(Output::Directory(output.to_owned()));
        }

        return Err(invalid_input(format!(
            "output path is neither a file nor a directory: {}",
            output.display()
        )));
    }

    if !has_directory_input && animation_count == 1 && has_format_extension(output, format) {
        return Ok(Output::File(output.to_owned()));
    }

    Ok(Output::Directory(output.to_owned()))
}

/// Validates that an explicit output file has the requested format extension.
fn validate_output_extension(path: &Path, format: Format) -> Result<(), Error> {
    if has_format_extension(path, format) {
        return Ok(());
    }

    Err(invalid_input(format!(
        "output file must have the .{} extension: {}",
        format.extension(),
        path.display()
    )))
}

/// Checks whether a path has the output format's file extension.
fn has_format_extension(path: &Path, format: Format) -> bool {
    is_extension(path, format.extension())
}

#[cfg(feature = "kf")]
/// Converts HKX animations to KF bytes.
fn export_kf(
    skeleton_bytes: &[u8],
    skeleton_path: &Path,
    animations: &[AnimationFile],
    output: &Output,
) -> Result<(), AnyError> {
    let inputs = animations
        .iter()
        .map(|animation| KfAnimationInput {
            bytes: &animation.bytes,
            path: animation.path.as_path(),
        })
        .collect::<Vec<_>>();

    let kf_bytes = to_kf_bytes_vec(skeleton_bytes, skeleton_path, &inputs)?;

    if kf_bytes.len() != animations.len() {
        return Err(Box::new(invalid_data(
            "conversion result count does not match animation count",
        )));
    }

    write_outputs(output, animations, kf_bytes, Format::Kf)?;

    Ok(())
}

#[cfg(feature = "fbx")]
/// Converts HKX animations to FBX bytes.
fn export_fbx_format(
    skeleton_bytes: &[u8],
    skeleton_path: &Path,
    animations: &[AnimationFile],
    output: &Output,
    fps: f32,
    format: serde_fbx::ser::Format,
) -> Result<(), AnyError> {
    let inputs = animations
        .iter()
        .map(|animation| FbxAnimationInput {
            bytes: &animation.bytes,
            path: animation.path.as_path(),
        })
        .collect::<Vec<_>>();

    let fbx_bytes = export_fbx(skeleton_bytes, skeleton_path, &inputs, fps, format)?;

    if fbx_bytes.len() != animations.len() {
        return Err(Box::new(invalid_data(
            "conversion result count does not match animation count",
        )));
    }

    write_outputs(output, animations, fbx_bytes, Format::Fbx)?;

    Ok(())
}

/// Writes converted animations while preserving their relative paths.
fn write_outputs(
    output: &Output,
    animations: &[AnimationFile],
    converted: Vec<Vec<u8>>,
    format: Format,
) -> Result<(), Error> {
    if animations.len() != converted.len() {
        return Err(invalid_data(
            "conversion result count does not match animation count",
        ));
    }

    match output {
        Output::File(path) => {
            if animations.len() != 1 {
                return Err(invalid_input(
                    "an output file can only be used with a single animation",
                ));
            }

            write_file(path, &converted[0])?;

            tracing::info!("Exported '{}'", path.display());
        }

        Output::Directory(directory) => {
            fs::create_dir_all(directory).map_err(|source| Error::IoError { source })?;

            animations
                .par_iter()
                .zip(converted.into_par_iter())
                .map(|(animation, bytes)| {
                    let output_path =
                        output_path(directory, &animation.relative_path, format.extension());

                    write_file(&output_path, &bytes)?;

                    tracing::info!("Exported '{}'", output_path.display());

                    Ok::<(), Error>(())
                })
                .collect::<Result<Vec<_>, _>>()?;
        }
    }

    Ok(())
}
