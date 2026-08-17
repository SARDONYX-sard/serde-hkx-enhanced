//! Import KF, FBX, and JSON animations into Havok HKX animations.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use jwalk::WalkDir;
use serde_hkx_features::{Format, error::Error};
use serde_spline::hkx::{Animation, AnimationAnnotation, Skeleton, ser::to_hkx};
use tokio::{fs, task::JoinSet};

use super::{invalid_input, write_file};

use crate::{AnyError, args::rig::Output};

const EXAMPLES: &str = color_print::cstr!(
    r#"Examples

- <blue!>Import a KF animation</blue!>
  <cyan!>hkxc importrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.kf <cyan!>-v</cyan!> amd64

- <blue!>Import an FBX animation</blue!>
  <cyan!>hkxc importrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.fbx <cyan!>-v</cyan!> amd64

- <blue!>Import a JSON animation</blue!>
  <cyan!>hkxc importrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.json <cyan!>-v</cyan!> amd64

- <blue!>Import a KF animation with annotations</blue!>
  <cyan!>hkxc importrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.kf <cyan!>-v</cyan!> amd64
  <cyan!>+</cyan!> ./idle.annotations.json

- <blue!>Import an FBX animation with annotations</blue!>
  <cyan!>hkxc importrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.fbx <cyan!>-v</cyan!> amd64
  <cyan!>+</cyan!> ./idle.annotations.json

- <blue!>Import all KF, FBX, and JSON animations in a directory</blue!>
  <cyan!>hkxc importrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations <cyan!>-v</cyan!> amd64

- <blue!>Import multiple animation formats</blue!>
  <cyan!>hkxc importrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.kf ./walk.fbx ./run.json <cyan!>-v</cyan!> amd64

- <blue!>Specify an output directory</blue!>
  <cyan!>hkxc importrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations <cyan!>-o</cyan!> ./out <cyan!>-v</cyan!> amd64

- <blue!>Specify an arbitrary output file name</blue!>
  <cyan!>hkxc importrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.kf <cyan!>-o</cyan!> ./idle.animation <cyan!>-v</cyan!> amd64

- <blue!>Import a JSON animation without specifying an animation stack alias</blue!>
  <cyan!>hkxc importrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.json <cyan!>-v</cyan!> amd64


Input behavior:
  - --animations accepts files and directories.
  - Directories are searched recursively for .kf, .fbx, and .json files.
  - Multiple files and directories may be specified.
  - .kf, .fbx, and .json inputs may be mixed.
  - KF and FBX inputs may use a neighboring .annotations.json file.
  - JSON inputs contain annotations in the Animation object itself.
  - Unknown file extensions are ignored when scanning directories.
  - Explicit input files must have a supported animation extension.

Output behavior:
  - Without --output, files are written to ./output/.
  - A single animation may use any explicit output file path.
  - Multiple animations require an output directory.
  - Directory inputs preserve their relative path structure.
"#
);

#[derive(Debug, clap::Args)]
#[clap(arg_required_else_help = true, after_long_help = EXAMPLES)]
pub(crate) struct Args {
    /// Input skeleton HKX file.
    #[clap(short = 's', long, value_name = "SKELETON")]
    pub skeleton: PathBuf,

    /// One or more KF, FBX, or JSON animation files or directories.
    ///
    /// Directories are searched recursively for supported animation files.
    #[clap(short = 'a', long, value_name = "ANIMATION", num_args = 1..)]
    pub animations: Vec<PathBuf>,

    /// Output directory or an explicit output file for a single animation.
    ///
    /// The extension of an explicit output file does not need to be .hkx.
    /// Defaults to ./output/.
    #[clap(short, long, value_name = "OUTPUT")]
    pub output: Option<PathBuf>,

    /// File format to output.
    #[clap(short = 'v', long, ignore_case = true, default_value = "amd64")]
    pub format: Format,

    /// Frames per second for sampling animations.
    ///
    /// This value is used by KF and FBX importers and when converting JSON
    /// animations back into HKX.
    #[clap(long, default_value = "30.0")]
    pub fps: f32,
}

#[derive(Debug, Clone)]
struct AnimationInput {
    path: PathBuf,
    root: Option<PathBuf>,
}

/// Imports KF, FBX, and JSON animations into Havok HKX animations.
///
/// The skeleton is decoded once and shared by all animation imports.
/// Each animation is read, decoded, converted, and written independently.
///
/// # Errors
///
/// Returns [`AnyError`] if the skeleton cannot be read or decoded, an
/// animation path is invalid, an animation cannot be read or converted,
/// annotations cannot be read or decoded, the output configuration is invalid,
/// or an output file cannot be written.
pub async fn importrig(args: &Args) -> Result<(), AnyError> {
    if args.animations.is_empty() {
        return Err(invalid_input("at least one --animations path is required").into());
    }

    if !args.fps.is_finite() || args.fps <= 0.0 {
        return Err(invalid_input("--fps must be a finite value greater than zero").into());
    }

    let (animations, has_directory_input) = resolve_animations(&args.animations)?;

    if animations.is_empty() {
        return Err(invalid_input("no .kf, .fbx, or .json animation files were found").into());
    }

    let output = resolve_output(
        args.output.as_deref(),
        animations.len(),
        has_directory_input,
    )?;

    let skeleton_bytes =
        fs::read(&args.skeleton)
            .await
            .map_err(|source| Error::FailedReadFile {
                source,
                path: args.skeleton.clone(),
            })?;

    // NOTE: `Skeleton::from_bytes` is intended for `hkx` and XML.
    let skeleton = Arc::new(Skeleton::from_bytes(
        &skeleton_bytes,
        args.skeleton.as_path(),
    )?);

    if let Output::Directory(directory) = &output {
        fs::create_dir_all(directory)
            .await
            .map_err(|source| Error::IoError { source })?;
    }

    let mut tasks = JoinSet::new();

    for animation in animations {
        let skeleton = Arc::clone(&skeleton);
        let output = output.clone();
        let fps = args.fps;
        let format = args.format;

        tasks.spawn(
            async move { import_animation(&animation, &skeleton, &output, fps, format).await },
        );
    }

    while let Some(result) = tasks.join_next().await {
        result??;
    }

    Ok(())
}

/// Resolves animation files from explicit files and directories.
///
/// Explicit files are represented without a root directory. Files discovered
/// inside a directory retain that directory so their relative paths can be
/// preserved in the output.
///
/// # Errors
///
/// Returns [`Error`] if an input path does not exist, an explicit file does
/// not have a supported animation extension, or a directory cannot be
/// traversed.
fn resolve_animations(inputs: &[PathBuf]) -> Result<(Vec<AnimationInput>, bool), Error> {
    let mut animations = Vec::new();
    let mut has_directory_input = false;

    for input in inputs {
        if input.is_file() {
            if !is_animation_extension(input) {
                return Err(invalid_input(format!(
                    "animation input must be a .kf, .fbx, or .json file: {}",
                    input.display()
                )));
            }

            animations.push(AnimationInput {
                path: input.clone(),
                root: None,
            });

            continue;
        }

        if input.is_dir() {
            has_directory_input = true;

            for entry in WalkDir::new(input).sort(true) {
                let entry = entry.map_err(|source| {
                    invalid_input(format!(
                        "failed to read animation directory '{}': {source}",
                        input.display()
                    ))
                })?;

                if entry.file_type().is_dir() {
                    continue;
                }

                let path = entry.path();

                if !is_animation_extension(&path) {
                    continue;
                }

                animations.push(AnimationInput {
                    path,
                    root: Some(input.clone()),
                });
            }

            continue;
        }

        return Err(invalid_input(format!(
            "animation path does not exist or is not a file/directory: {}",
            input.display()
        )));
    }

    animations.sort_by(|a, b| a.path.cmp(&b.path));

    Ok((animations, has_directory_input))
}

/// Checks whether a path has a supported animation extension.
fn is_animation_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("kf")
                || extension.eq_ignore_ascii_case("fbx")
                || extension.eq_ignore_ascii_case("json")
        })
}

/// Resolves an animation input to its output path.
///
/// For an explicitly specified single file, `root` is `None` and `output`
/// is returned unchanged. This allows arbitrary output file names and
/// extensions.
///
/// For directory inputs, the path relative to `root` is preserved and the
/// input extension is replaced with `.hkx`.
///
/// # Errors
///
/// Returns [`Error`] if the animation path cannot be made relative to its
/// input directory.
fn get_rel_path(
    input: &Path,
    root: Option<&Path>,
    output: &Path,
    format: Format,
) -> Result<PathBuf, Error> {
    match root {
        None => Ok(output.to_owned()),

        Some(root) => {
            let mut relative = input
                .strip_prefix(root)
                .map_err(|_| {
                    invalid_input(format!(
                        "animation path is not relative to its input directory: {}",
                        input.display()
                    ))
                })?
                .to_owned();

            relative.set_extension(format.as_extension());

            Ok(output.join(relative))
        }
    }
}

/// Resolves whether the output represents a file or directory.
///
/// A single explicit animation may use any output path regardless of its
/// extension. Multiple animations and directory inputs require an output
/// directory.
///
/// # Errors
///
/// Returns [`Error`] if an existing output path has an invalid type or a file
/// output is requested for multiple animations.
fn resolve_output(
    output: Option<&Path>,
    animation_count: usize,
    has_directory_input: bool,
) -> Result<Output, Error> {
    let output = output.unwrap_or_else(|| Path::new("output"));

    if output.exists() {
        if output.is_file() {
            if animation_count != 1 {
                return Err(invalid_input(
                    "an output file can only be used with a single animation",
                ));
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

    if animation_count == 1 && !has_directory_input {
        return Ok(Output::File(output.to_owned()));
    }

    Ok(Output::Directory(output.to_owned()))
}

/// Imports and writes one animation.
///
/// KF and FBX annotations are loaded from a neighboring
/// `<animation>.annotations.json` file. JSON animations already contain their
/// annotations as part of the serialized [`Animation`] and therefore do not
/// load a separate annotation file.
///
/// # Errors
///
/// Returns [`AnyError`] if the animation cannot be read, annotations cannot be
/// read or decoded, conversion fails, or the resulting HKX cannot be written.
async fn import_animation(
    input: &AnimationInput,
    skeleton: &Skeleton,
    output: &Output,
    fps: f32,
    format: Format,
) -> Result<(), AnyError> {
    let bytes = fs::read(&input.path)
        .await
        .map_err(|source| Error::FailedReadFile {
            source,
            path: input.path.clone(),
        })?;

    let output_path = match output {
        Output::File(path) => get_rel_path(&input.path, input.root.as_deref(), path, format)?,
        Output::Directory(directory) => {
            get_rel_path(&input.path, input.root.as_deref(), directory, format)?
        }
    };

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .await
            .map_err(|source| Error::IoError { source })?;
    }

    let extension = input
        .path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);

    let converted = match extension.as_deref() {
        #[cfg(feature = "kf")]
        Some("kf") => {
            let config = niflib_animation::serde_kf::DeConfig {
                annotations: read_annotations(&input.path).await?,
                fps,
                format,
            };
            niflib_animation::serde_kf::from_kf(&bytes, skeleton, config)?
        }

        #[cfg(not(feature = "kf"))]
        Some("kf") => {
            return Err(
                invalid_input("KF input was specified, but the KF feature is disabled").into(),
            );
        }

        #[cfg(feature = "fbx")]
        Some("fbx") => {
            let config = serde_fbx::de::Config {
                annotations: read_annotations(&input.path).await?,
                fps,
                format,
                animation_stack: None,
            };
            serde_fbx::de::from_fbx(&bytes, skeleton, config)?
        }

        #[cfg(not(feature = "fbx"))]
        Some("fbx") => {
            return Err(
                invalid_input("FBX input was specified, but the FBX feature is disabled").into(),
            );
        }

        Some("json") => {
            let animation: Animation = sonic_rs::from_slice(&bytes)?;

            to_hkx(skeleton, &animation, fps, format)?
        }

        _ => {
            return Err(invalid_input(format!(
                "unsupported animation extension: {}",
                input.path.display()
            ))
            .into());
        }
    };

    write_file(&output_path, &converted)?;

    tracing::info!("Imported '{}'", output_path.display());

    Ok(())
}

/// Reads optional animation annotations for KF and FBX inputs.
///
/// The annotation file uses the `<animation>.annotations.json` naming
/// convention. A missing annotation file is treated as an empty annotation
/// list.
///
/// # Errors
///
/// Returns [`AnyError`] if the annotation file exists but cannot be read or
/// deserialized.
async fn read_annotations(animation_path: &Path) -> Result<Vec<AnimationAnnotation>, AnyError> {
    let parent = animation_path.parent().unwrap_or_else(|| Path::new(""));

    let stem = animation_path
        .file_stem()
        .ok_or_else(|| invalid_input("animation input path has no file stem"))?;

    let path = parent.join(format!("{}.annotations.json", stem.to_string_lossy()));

    if !path.exists() {
        return Ok(Vec::new());
    }

    let bytes = fs::read(&path)
        .await
        .map_err(|source| Error::FailedReadFile {
            source,
            path: path.clone(),
        })?;

    Ok(sonic_rs::from_slice(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_animation_extension_accepts_kf() {
        assert!(is_animation_extension(Path::new("idle.kf")));
        assert!(is_animation_extension(Path::new("idle.KF")));
    }

    #[test]
    fn is_animation_extension_accepts_fbx() {
        assert!(is_animation_extension(Path::new("idle.fbx")));
        assert!(is_animation_extension(Path::new("idle.FBX")));
    }

    #[test]
    fn is_animation_extension_accepts_json() {
        assert!(is_animation_extension(Path::new("idle.json")));
        assert!(is_animation_extension(Path::new("idle.JSON")));
    }

    #[test]
    fn is_animation_extension_rejects_unknown_extension() {
        assert!(!is_animation_extension(Path::new("idle.hkx")));
        assert!(!is_animation_extension(Path::new("idle.txt")));
    }

    #[test]
    fn get_rel_path_uses_exact_output_for_single_file() {
        let input = Path::new("animations/idle.kf");
        let output = Path::new("custom.animation");
        let format = Format::Amd64;

        let result = get_rel_path(input, None, output, format).unwrap();

        assert_eq!(result, PathBuf::from("custom.animation"));
    }

    #[test]
    fn get_rel_path_preserves_directory_structure() {
        let input = Path::new("animations/actors/human/idle.kf");
        let root = Path::new("animations");
        let output = Path::new("output");
        let format = Format::Xml;

        let result = get_rel_path(input, Some(root), output, format).unwrap();

        assert_eq!(result, PathBuf::from("output/actors/human/idle.xml"));
    }

    #[test]
    fn get_rel_path_changes_json_extension_to_hkx() {
        let input = Path::new("animations/actors/human/idle.json");
        let root = Path::new("animations");
        let output = Path::new("output");
        let format = Format::Win32;

        let result = get_rel_path(input, Some(root), output, format).unwrap();

        assert_eq!(result, PathBuf::from("output/actors/human/idle.hkx"));
    }

    #[test]
    fn get_rel_path_rejects_path_outside_root() {
        let input = Path::new("other/idle.kf");
        let root = Path::new("animations");
        let output = Path::new("output");
        let format = Format::Win32;

        let result = get_rel_path(input, Some(root), output, format);

        assert!(result.is_err());
    }

    #[test]
    fn resolve_output_allows_arbitrary_single_file_extension() {
        let output = Path::new("custom.animation");

        let result = resolve_output(Some(output), 1, false).unwrap();

        assert_eq!(result, Output::File(output.to_owned()));
    }

    #[test]
    fn resolve_output_uses_directory_for_multiple_files() {
        let output = Path::new("custom.animation");

        let result = resolve_output(Some(output), 2, false).unwrap();

        assert_eq!(result, Output::Directory(output.to_owned()));
    }
}
