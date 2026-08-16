//! Export Havok HKX animations to KF or FBX.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use clap::ValueEnum;
use serde_hkx_features::error::Error;

#[cfg(feature = "fbx")]
use serde_fbx::ser::{AnimationInput as FbxAnimationInput, export_fbx};

#[cfg(feature = "kf")]
use niflib_animation::ser::{AnimationInput as KfAnimationInput, to_kf_bytes_vec};

use crate::AnyError;

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
}

impl Format {
    const fn extension(self) -> &'static str {
        match self {
            #[cfg(feature = "kf")]
            Self::Kf => "kf",
            #[cfg(feature = "fbx")]
            Self::Fbx => "fbx",
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
    let animation_paths = resolve_animation_paths(&args.animations, !args.no_recursive)?;

    if animation_paths.is_empty() {
        return Err(invalid_input("no HKX animation files were found").into());
    }

    let output = resolve_output(args.output.as_deref(), &animation_paths, args.format)?;

    let skeleton_bytes = fs::read(&args.skeleton).map_err(|source| Error::FailedReadFile {
        source,
        path: args.skeleton.clone(),
    })?;

    let animation_bytes = read_animations(&animation_paths)?;

    match args.format {
        #[cfg(feature = "kf")]
        Format::Kf => export_kf(
            &skeleton_bytes,
            &args.skeleton,
            &animation_paths,
            &animation_bytes,
            &output,
        )?,

        #[cfg(feature = "fbx")]
        Format::Fbx => {
            if !args.fps.is_finite() || args.fps <= 0.0 {
                return Err(invalid_input("--fps must be a finite value greater than zero").into());
            }

            export_fbx_format(
                &skeleton_bytes,
                &args.skeleton,
                &animation_paths,
                &animation_bytes,
                &output,
                args.fps,
            )?;
        }
    }

    Ok(())
}

fn resolve_animation_paths(inputs: &[PathBuf], recursive: bool) -> Result<Vec<PathBuf>, Error> {
    let mut paths = Vec::new();

    for input in inputs {
        if input.is_file() {
            if !is_hkx(input) {
                return Err(invalid_input(format!(
                    "animation input is not an HKX file: {}",
                    input.display()
                )));
            }

            paths.push(input.clone());
            continue;
        }

        if input.is_dir() {
            collect_animations(input, recursive, &mut paths)?;
            continue;
        }

        return Err(invalid_input(format!(
            "animation path does not exist or is not a file/directory: {}",
            input.display()
        )));
    }

    paths.sort();
    paths.dedup();

    Ok(paths)
}

fn collect_animations(
    directory: &Path,
    recursive: bool,
    output: &mut Vec<PathBuf>,
) -> Result<(), Error> {
    let entries = fs::read_dir(directory).map_err(|source| Error::IoError { source })?;

    for entry in entries {
        let entry = entry.map_err(|source| Error::IoError { source })?;
        let path = entry.path();

        let file_type = entry
            .file_type()
            .map_err(|source| Error::IoError { source })?;

        if file_type.is_dir() {
            if recursive {
                collect_animations(&path, true, output)?;
            }
        } else if is_hkx(&path) {
            output.push(path);
        }
    }

    Ok(())
}

fn is_hkx(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("hkx"))
}

fn read_animations(paths: &[PathBuf]) -> Result<Vec<Vec<u8>>, Error> {
    paths
        .iter()
        .map(|path| {
            fs::read(path).map_err(|source| Error::FailedReadFile {
                source,
                path: path.clone(),
            })
        })
        .collect()
}

#[derive(Debug)]
enum Output {
    File(PathBuf),
    Directory(PathBuf),
}

fn resolve_output(
    output: Option<&Path>,
    animations: &[PathBuf],
    format: Format,
) -> Result<Output, Error> {
    let output = output.unwrap_or_else(|| Path::new("output"));

    if output.exists() {
        if output.is_file() {
            if animations.len() != 1 {
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

    if animations.len() == 1 && has_format_extension(output, format) {
        return Ok(Output::File(output.to_owned()));
    }

    Ok(Output::Directory(output.to_owned()))
}

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

fn has_format_extension(path: &Path, format: Format) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(format.extension()))
}

fn animation_output_path(directory: &Path, animation: &Path, format: Format) -> PathBuf {
    let stem = animation
        .file_stem()
        .unwrap_or_else(|| std::ffi::OsStr::new("animation"));

    directory.join(stem).with_extension(format.extension())
}

#[cfg(feature = "kf")]
fn export_kf(
    skeleton_bytes: &[u8],
    skeleton_path: &Path,
    animation_paths: &[PathBuf],
    animation_bytes: &[Vec<u8>],
    output: &Output,
) -> Result<(), AnyError> {
    let inputs = animation_paths
        .iter()
        .zip(animation_bytes)
        .map(|(path, bytes)| KfAnimationInput {
            bytes,
            path: path.as_path(),
        })
        .collect::<Vec<_>>();

    let kf_bytes = to_kf_bytes_vec(skeleton_bytes, skeleton_path, &inputs)?;

    if kf_bytes.len() != animation_paths.len() {
        return Err(Box::new(invalid_data(
            "conversion result count does not match animation count",
        )));
    }

    Ok(write_outputs(
        output,
        animation_paths,
        kf_bytes,
        Format::Kf,
    )?)
}

#[cfg(feature = "fbx")]
fn export_fbx_format(
    skeleton_bytes: &[u8],
    skeleton_path: &Path,
    animation_paths: &[PathBuf],
    animation_bytes: &[Vec<u8>],
    output: &Output,
    fps: f32,
) -> Result<(), AnyError> {
    let inputs = animation_paths
        .iter()
        .zip(animation_bytes)
        .map(|(path, bytes)| FbxAnimationInput {
            bytes,
            path: path.as_path(),
        })
        .collect::<Vec<_>>();

    let fbx_bytes = export_fbx(skeleton_bytes, skeleton_path, &inputs, fps)?;

    if fbx_bytes.len() != animation_paths.len() {
        return Err(Box::new(invalid_data(
            "conversion result count does not match animation count",
        )));
    }

    Ok(write_outputs(
        output,
        animation_paths,
        fbx_bytes,
        Format::Fbx,
    )?)
}

fn write_outputs(
    output: &Output,
    animation_paths: &[PathBuf],
    converted: Vec<Vec<u8>>,
    format: Format,
) -> Result<(), Error> {
    match output {
        Output::File(path) => {
            if animation_paths.len() != 1 || converted.len() != 1 {
                return Err(invalid_input(
                    "an output file can only be used with a single animation",
                ));
            }

            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|source| Error::IoError { source })?;
            }

            fs::write(path, &converted[0]).map_err(|source| Error::IoError { source })?;

            tracing::info!("Exported '{}'", path.display());
        }

        Output::Directory(directory) => {
            fs::create_dir_all(directory).map_err(|source| Error::IoError { source })?;

            for (animation_path, bytes) in animation_paths.iter().zip(converted) {
                let output_path = animation_output_path(directory, animation_path, format);

                fs::write(&output_path, bytes).map_err(|source| Error::IoError { source })?;

                tracing::info!("Exported '{}'", output_path.display());
            }
        }
    }

    Ok(())
}

fn invalid_input(message: impl Into<String>) -> Error {
    Error::IoError {
        source: io::Error::new(io::ErrorKind::InvalidInput, message.into()),
    }
}

fn invalid_data(message: impl Into<String>) -> Error {
    Error::IoError {
        source: io::Error::new(io::ErrorKind::InvalidData, message.into()),
    }
}
