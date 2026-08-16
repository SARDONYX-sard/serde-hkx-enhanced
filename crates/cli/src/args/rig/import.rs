//! Import KF and FBX animations into Havok HKX animations.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde_hkx_features::{Format, error::Error};

use crate::AnyError;

#[cfg(feature = "fbx")]
use serde_fbx::de::{AnimationInput as FbxAnimationInput, fbx_to_hkx_bytes_vec};

#[cfg(feature = "kf")]
use niflib_animation::de::{AnimationInput as KfAnimationInput, from_kf_bytes_vec_to_hkx};

pub const EXAMPLES: &str = color_print::cstr!(
    r#"Examples

- <blue!>Import a KF animation</blue!>
  <cyan!>hkxc importrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.kf -v amd64

- <blue!>Import an FBX animation</blue!>
  <cyan!>hkxc importrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.fbx -v win32

- <blue!>Import all KF/FBX animations in a directory</blue!>
  <cyan!>hkxc importrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations -v xml

- <blue!>Import multiple animations</blue!>
  <cyan!>hkxc importrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.kf ./walk.kf -v amd64

- <blue!>Import KF and FBX animations together</blue!>
  <cyan!>hkxc importrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.kf ./walk.fbx -v amd64

- <blue!>Specify an output directory</blue!>
  <cyan!>hkxc importrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations -o ./out -v amd64

- <blue!>Specify an output file for a single animation</blue!>
  <cyan!>hkxc importrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.kf -o ./idle.hkx -v amd64

Input behavior:
  - --animations accepts files and directories.
  - Directories are searched recursively for .kf and .fbx files.
  - Multiple files and directories may be specified.
  - .kf and .fbx inputs may be mixed.
  - Unknown file extensions are ignored when scanning directories.
  - Explicit input files must have a .kf or .fbx extension.

Output behavior:
  - Without --output, files are written to ./output/.
  - A single animation may use an explicit .hkx output file.
  - Multiple animations require an output directory.
  - A non-existing output path ending in .hkx is treated as a file.
  - Other non-existing output paths are treated as directories.
  - --format controls the HKX output format (amd64, win32, xml).
  "#
);

#[derive(Debug, clap::Args)]
#[clap(arg_required_else_help = true, after_long_help = EXAMPLES)]
pub(crate) struct Args {
    /// Input skeleton HKX file.
    #[clap(short = 's', long, value_name = "SKELETON")]
    pub skeleton: PathBuf,

    /// One or more KF/FBX animation files or directories.
    #[clap(short = 'a', long, value_name = "ANIMATION", num_args = 1..)]
    pub animations: Vec<PathBuf>,

    /// Output directory, or an explicit .hkx path for a single animation.
    ///
    /// Defaults to ./output/.
    #[clap(short, long, value_name = "OUTPUT")]
    pub output: Option<PathBuf>,

    /// File format to output.
    #[clap(short = 'v', long, ignore_case = true, default_value = "amd64")]
    pub format: Format,

    /// Frames per second for sampling animations.
    #[clap(long, default_value = "30.0")]
    pub fps: f32,
}

#[derive(Debug)]
struct AnimationFile {
    path: PathBuf,
    relative_path: PathBuf,
    bytes: Vec<u8>,
}

#[derive(Debug, Default)]
struct AnimationInputs {
    kf: Vec<AnimationFile>,
    fbx: Vec<AnimationFile>,
}

impl AnimationInputs {
    const fn len(&self) -> usize {
        self.kf.len() + self.fbx.len()
    }

    const fn is_empty(&self) -> bool {
        self.kf.is_empty() && self.fbx.is_empty()
    }
}

#[derive(Debug)]
enum Output {
    File(PathBuf),
    Directory(PathBuf),
}

/// Imports KF and FBX animations into Havok HKX animations.
///
/// KF and FBX inputs are collected independently and converted by their
/// respective importers. The resulting HKX files are then written together
/// using the requested [`Format`].
///
/// # Errors
///
/// Returns [`AnyError`] if the skeleton cannot be read, an animation path is
/// invalid, an animation cannot be read or converted, the output configuration
/// is invalid, or an output file cannot be written.
pub async fn importrig(args: &Args) -> Result<(), AnyError> {
    if args.animations.is_empty() {
        return Err(invalid_input("at least one --animations path is required").into());
    }

    if !args.fps.is_finite() || args.fps <= 0.0 {
        return Err(invalid_input("--fps must be a finite value greater than zero").into());
    }

    let inputs = resolve_animations(&args.animations)?;

    if inputs.is_empty() {
        return Err(invalid_input("no .kf or .fbx animation files were found").into());
    }

    let output = resolve_output(args.output.as_deref(), inputs.len())?;

    let skeleton_bytes = fs::read(&args.skeleton).map_err(|source| Error::IoError { source })?;

    let mut outputs = Vec::with_capacity(inputs.len());

    #[cfg(feature = "kf")]
    if !inputs.kf.is_empty() {
        let bytes = import_kf(
            &skeleton_bytes,
            &args.skeleton,
            &inputs.kf,
            args.fps,
            args.format,
        )?;

        outputs.extend(
            inputs
                .kf
                .iter()
                .zip(bytes)
                .map(|(input, bytes)| ConvertedAnimation {
                    relative_path: input.relative_path.clone(),
                    bytes,
                }),
        );
    }

    #[cfg(not(feature = "kf"))]
    if !inputs.kf.is_empty() {
        return Err(invalid_input("KF input was specified, but the KF feature is disabled").into());
    }

    #[cfg(feature = "fbx")]
    if !inputs.fbx.is_empty() {
        let bytes = import_fbx(
            &skeleton_bytes,
            &args.skeleton,
            &inputs.fbx,
            args.fps,
            args.format,
        )?;

        outputs.extend(
            inputs
                .fbx
                .iter()
                .zip(bytes)
                .map(|(input, bytes)| ConvertedAnimation {
                    relative_path: input.relative_path.clone(),
                    bytes,
                }),
        );
    }

    #[cfg(not(feature = "fbx"))]
    if !inputs.fbx.is_empty() {
        return Err(
            invalid_input("FBX input was specified, but the FBX feature is disabled").into(),
        );
    }

    if outputs.len() != inputs.len() {
        return Err(invalid_data("conversion result count does not match animation count").into());
    }

    write_outputs(&output, outputs)?;

    Ok(())
}

#[derive(Debug)]
struct ConvertedAnimation {
    relative_path: PathBuf,
    bytes: Vec<u8>,
}

fn resolve_animations(inputs: &[PathBuf]) -> Result<AnimationInputs, Error> {
    let mut animations = AnimationInputs::default();

    for input in inputs {
        if input.is_file() {
            collect_file(input, &mut animations)?;
            continue;
        }

        if input.is_dir() {
            collect_directory(input, input, &mut animations)?;
            continue;
        }

        return Err(invalid_input(format!(
            "animation path does not exist or is not a file/directory: {}",
            input.display()
        )));
    }

    animations.kf.sort_by(|a, b| a.path.cmp(&b.path));
    animations.fbx.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(animations)
}

fn collect_file(path: &Path, animations: &mut AnimationInputs) -> Result<(), Error> {
    let extension = path.extension().and_then(|value| value.to_str());

    let animation = AnimationFile {
        path: path.to_owned(),
        relative_path: path.file_name().map(PathBuf::from).ok_or_else(|| {
            invalid_input(format!(
                "animation path has no file name: {}",
                path.display()
            ))
        })?,
        bytes: fs::read(path).map_err(|source| Error::IoError { source })?,
    };

    match extension {
        Some(extension) if extension.eq_ignore_ascii_case("kf") => {
            animations.kf.push(animation);
        }
        Some(extension) if extension.eq_ignore_ascii_case("fbx") => {
            animations.fbx.push(animation);
        }
        _ => {
            return Err(invalid_input(format!(
                "animation input must be a .kf or .fbx file: {}",
                path.display()
            )));
        }
    }

    Ok(())
}

fn collect_directory(
    root: &Path,
    directory: &Path,
    animations: &mut AnimationInputs,
) -> Result<(), Error> {
    for entry in fs::read_dir(directory).map_err(|source| Error::IoError { source })? {
        let entry = entry.map_err(|source| Error::IoError { source })?;
        let path = entry.path();

        if path.is_dir() {
            collect_directory(root, &path, animations)?;
            continue;
        }

        match path.extension().and_then(|value| value.to_str()) {
            Some(extension) if extension.eq_ignore_ascii_case("kf") => {
                let relative_path = path
                    .strip_prefix(root)
                    .map_err(|_| {
                        invalid_data(format!(
                            "failed to determine relative animation path: {}",
                            path.display()
                        ))
                    })?
                    .to_owned();

                animations.kf.push(AnimationFile {
                    bytes: fs::read(&path).map_err(|source| Error::IoError { source })?,
                    path,
                    relative_path,
                });
            }
            Some(extension) if extension.eq_ignore_ascii_case("fbx") => {
                let relative_path = path
                    .strip_prefix(root)
                    .map_err(|_| {
                        invalid_data(format!(
                            "failed to determine relative animation path: {}",
                            path.display()
                        ))
                    })?
                    .to_owned();

                animations.fbx.push(AnimationFile {
                    bytes: fs::read(&path).map_err(|source| Error::IoError { source })?,
                    path,
                    relative_path,
                });
            }
            _ => {}
        }
    }

    Ok(())
}

fn resolve_output(output: Option<&Path>, animation_count: usize) -> Result<Output, Error> {
    let output = output.unwrap_or_else(|| Path::new("output"));

    if output.exists() {
        if output.is_file() {
            if animation_count != 1 {
                return Err(invalid_input(
                    "an output file can only be used with a single animation",
                ));
            }

            if !has_hkx_extension(output) {
                return Err(invalid_input("an output file must have the .hkx extension"));
            }

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

    if animation_count == 1 && has_hkx_extension(output) {
        return Ok(Output::File(output.to_owned()));
    }

    Ok(Output::Directory(output.to_owned()))
}

fn has_hkx_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("hkx"))
}

#[cfg(feature = "kf")]
fn import_kf(
    skeleton_bytes: &[u8],
    skeleton_path: &Path,
    animations: &[AnimationFile],
    fps: f32,
    format: Format,
) -> Result<Vec<Vec<u8>>, AnyError> {
    let inputs = animations
        .iter()
        .map(|animation| KfAnimationInput {
            bytes: &animation.bytes,
            path: animation.path.as_path(),
            annotations: Vec::new(),
        })
        .collect::<Vec<_>>();

    Ok(from_kf_bytes_vec_to_hkx(
        skeleton_bytes,
        skeleton_path,
        inputs,
        fps,
        format,
    )?)
}

#[cfg(feature = "fbx")]
fn import_fbx(
    skeleton_bytes: &[u8],
    skeleton_path: &Path,
    animations: &[AnimationFile],
    fps: f32,
    format: Format,
) -> Result<Vec<Vec<u8>>, AnyError> {
    let inputs = animations
        .iter()
        .map(|animation| FbxAnimationInput {
            bytes: &animation.bytes,
            path: animation.path.as_path(),
            animation_stack: None,
            annotations: Vec::new(),
        })
        .collect::<Vec<_>>();

    Ok(fbx_to_hkx_bytes_vec(
        skeleton_bytes,
        skeleton_path,
        inputs,
        fps,
        format,
    )?)
}

fn write_outputs(output: &Output, animations: Vec<ConvertedAnimation>) -> Result<(), Error> {
    match output {
        Output::File(path) => {
            let animation = animations
                .into_iter()
                .next()
                .ok_or_else(|| invalid_data("no converted animation is available"))?;

            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent).map_err(|source| Error::IoError { source })?;
            }

            fs::write(path, animation.bytes).map_err(|source| Error::IoError { source })?;

            tracing::info!("Written '{}'", path.display());
        }

        Output::Directory(directory) => {
            fs::create_dir_all(directory).map_err(|source| Error::IoError { source })?;

            for animation in animations {
                let output_path = directory.join(animation.relative_path.with_extension("hkx"));
                fs::write(&output_path, animation.bytes)
                    .map_err(|source| Error::IoError { source })?;

                tracing::info!("Written '{}'", output_path.display());
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
