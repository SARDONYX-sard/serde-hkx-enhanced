//! Export Havok HKX animations to KF, FBX, or JSON.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::ValueEnum;
use jwalk::WalkDir;
use serde_hkx_features::error::Error;
use serde_spline::hkx::{Animation, Skeleton};
use tokio::{fs, task::JoinSet};

use crate::AnyError;

#[cfg(all(feature = "kf", feature = "fbx"))]
use super::nif_caster;
use super::{Output, invalid_input, is_serde_hkx_supported_extension, write_file};

const EXAMPLES: &str = color_print::cstr!(
    r#"Examples
- <blue!>Export a single animation to KF</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations/idle.hkx <cyan!>-v</cyan!> kf

- <blue!>Export a single animation + mesh to FBX</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations/idle.hkx --nif ./cow.nif <cyan!>-v</cyan!> fbx

- <blue!>Export a single animation to JSON</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations/idle.hkx <cyan!>-v</cyan!> json
- <blue!>Export an animation with annotations</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations/idle.hkx <cyan!>-v</cyan!> kf
  <cyan!>→</cyan!> ./output/idle.kf
  <cyan!>→</cyan!> ./output/idle.annotations.json

- <blue!>Export all animations in a directory</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations <cyan!>-v</cyan!> fbx
- <blue!>Export multiple animations</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.hkx ./walk.hkx <cyan!>-v</cyan!> kf

- <blue!>Specify an output directory</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations <cyan!>-o</cyan!> ./out <cyan!>-v</cyan!> fbx
- <blue!>Specify an output file for a single animation</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations/idle.hkx <cyan!>-o</cyan!> ./idle.kf <cyan!>-v</cyan!> kf

- <blue!>Use long options</blue!>
  <cyan!>hkxc exportrig --skeleton</cyan!> ./skeleton.hkx <cyan!>--animations</cyan!> ./animations/idle.hkx <cyan!>--format</cyan!> json

Output behavior:
  - Without --output, files are written to ./output/.
  - A single animation may use an explicit output file.
  - Multiple animations require an output directory.
  - A directory input is always exported into an output directory.
  - Directory inputs preserve their relative path structure.
  - A non-existing output path ending in .kf, .fbx, or .json is treated as a file.
  - Other non-existing output paths are treated as directories.
  - JSON output contains the complete Animation, including annotations.
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

    Json,
}

impl Format {
    /// Returns the file extension associated with the output format.
    const fn extension(self) -> &'static str {
        match self {
            #[cfg(feature = "kf")]
            Self::Kf => "kf",

            #[cfg(feature = "fbx")]
            Self::Fbx | Self::FbxAscii => "fbx",
            Self::Json => "json",
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

    /// Optional NIF path used when exporting FBX.
    ///
    /// The NIF provides scene, mesh, skin, material, and texture data.
    #[cfg(all(feature = "kf", feature = "fbx"))]
    #[clap(long, value_name = "NIF")]
    pub nif: Option<PathBuf>,

    /// Output directory or an explicit output file for a single animation.
    ///
    /// The extension of an explicit output file does not need to match the
    /// selected format. Defaults to ./output/.
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

/// Exports Havok HKX animations to KF, FBX, or JSON.
///
/// The skeleton is decoded once and shared by all animation exports.
/// When an NIF is specified, it is loaded and converted once, then shared by
/// all FBX exports.
///
/// # Errors
///
/// Returns [`AnyError`] if an input path is invalid, the skeleton, NIF, or an
/// animation cannot be read or decoded, conversion or serialization fails, the
/// output configuration is invalid, or an output file cannot be written.
pub async fn exportrig(args: &Args) -> Result<(), AnyError> {
    let (animations, has_directory_input) =
        collect_animation_paths(&args.animations, !args.no_recursive)?;

    if animations.is_empty() {
        return Err(invalid_input("no HKX animation files were found").into());
    }

    let output = resolve_output(
        args.output.as_deref(),
        animations.len(),
        has_directory_input,
        args.format,
    )?;

    #[cfg(feature = "fbx")]
    if matches!(args.format, Format::Fbx | Format::FbxAscii)
        && (!args.fps.is_finite() || args.fps <= 0.0)
    {
        return Err(invalid_input("--fps must be a finite value greater than zero").into());
    }

    let skeleton_bytes =
        fs::read(&args.skeleton)
            .await
            .map_err(|source| Error::FailedReadFile {
                source,
                path: args.skeleton.clone(),
            })?;

    let skeleton = Arc::new(Skeleton::from_bytes(
        &skeleton_bytes,
        args.skeleton.as_path(),
    )?);

    #[cfg(all(feature = "kf", feature = "fbx"))]
    let nif = match args.nif.as_deref() {
        Some(path) => {
            let scene = niflib_animation::ffi::load_nif(&path.to_string_lossy())?;
            Some(Arc::new(nif_caster::cast(scene)))
        }
        None => None,
    };

    if matches!(args.format, Format::Json) {
        export_skeleton_json(&skeleton, &output).await?;
    }

    if let Output::Directory(directory) = &output {
        fs::create_dir_all(directory)
            .await
            .map_err(|source| Error::IoError { source })?;
    }

    let mut tasks = JoinSet::new();

    for (path, root) in animations {
        let skeleton = Arc::clone(&skeleton);
        let output = output.clone();
        let format = args.format;

        #[cfg(feature = "fbx")]
        let fps = args.fps;

        #[cfg(all(feature = "kf", feature = "fbx"))]
        let nif = nif.as_ref().map(Arc::clone);

        tasks.spawn(async move {
            export_animation(
                &path,
                root.as_deref(),
                &skeleton,
                &output,
                format,
                #[cfg(feature = "fbx")]
                fps,
                #[cfg(all(feature = "kf", feature = "fbx"))]
                nif.as_deref(),
            )
            .await
        });
    }

    while let Some(result) = tasks.join_next().await {
        result??;
    }

    Ok(())
}

/// Collects animation files from explicit files and directories.
///
/// Each returned entry contains the animation path and, for directory inputs,
/// the directory against which the animation path should be made relative.
///
/// No animation data is read at this stage.
///
/// # Errors
///
/// Returns [`Error`] if an input path does not exist, an explicit file is not
/// an HKX file, or a directory cannot be traversed.
#[expect(clippy::type_complexity)]
fn collect_animation_paths(
    inputs: &[PathBuf],
    recursive: bool,
) -> Result<(Vec<(PathBuf, Option<PathBuf>)>, bool), Error> {
    let mut animations = Vec::new();
    let mut has_directory_input = false;

    for input in inputs {
        let is_dir = input.is_dir();

        if !is_dir {
            if !is_serde_hkx_supported_extension(input) {
                return Err(invalid_input(format!(
                    "animation input is not an HKX file: {}",
                    input.display()
                )));
            }

            animations.push((input.clone(), None));
        } else {
            has_directory_input = true;

            let mut walker = WalkDir::new(input).sort(true);

            if !recursive {
                walker = walker.max_depth(1);
            }

            for entry in walker {
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

                if !is_serde_hkx_supported_extension(&path) {
                    continue;
                }

                animations.push((path, Some(input.clone())));
            }
        }
    }

    animations.sort_by(|a, b| a.0.cmp(&b.0));

    Ok((animations, has_directory_input))
}

/// Resolves an animation input to its final output path.
///
/// An explicit output file is used as-is. An output directory receives the
/// animation file name, preserving the relative directory structure when the
/// animation was discovered from a directory input.
///
/// # Errors
///
/// Returns [`Error`] if the input path has no file name or cannot be made
/// relative to its input directory.
fn resolve_animation_output(
    input: &Path,
    root: Option<&Path>,
    output: &Output,
    format: Format,
) -> Result<PathBuf, Error> {
    match output {
        Output::File(path) => Ok(path.to_owned()),
        Output::Directory(directory) => {
            let mut relative = match root {
                Some(root) => input
                    .strip_prefix(root)
                    .map_err(|_| {
                        invalid_input(format!(
                            "animation path is not relative to its input directory: {}",
                            input.display()
                        ))
                    })?
                    .to_owned(),
                None => input
                    .file_name()
                    .ok_or_else(|| invalid_input("animation input path has no file name"))?
                    .into(),
            };

            relative.set_extension(format.extension());

            Ok(directory.join(relative))
        }
    }
}

/// Resolves whether the output represents a file or directory.
///
/// JSON always requires an output directory because `skeleton.json` is
/// generated in addition to the animation JSON files.
///
/// For a single explicit animation file, an explicitly supplied output path
/// is always treated as a file regardless of its extension.
///
/// # Errors
///
/// Returns [`Error`] if an existing output path has an invalid type or an
/// output file is requested for multiple animations.
fn resolve_output(
    output: Option<&Path>,
    animation_count: usize,
    has_directory_input: bool,
    format: Format,
) -> Result<Output, Error> {
    let output = output.unwrap_or_else(|| Path::new("output"));

    if matches!(format, Format::Json) {
        if output.exists() && !output.is_dir() {
            return Err(invalid_input("JSON output requires an output directory"));
        }

        return Ok(Output::Directory(output.to_owned()));
    }

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

/// Exports the decoded skeleton to `skeleton.json`.
///
/// # Errors
///
/// Returns [`AnyError`] if JSON serialization fails or the output file cannot
/// be written.
async fn export_skeleton_json(skeleton: &Skeleton, output: &Output) -> Result<(), AnyError> {
    let directory = match output {
        Output::Directory(directory) => directory,
        Output::File(_) => {
            return Err(invalid_input("JSON output requires an output directory").into());
        }
    };

    fs::create_dir_all(directory)
        .await
        .map_err(|source| Error::IoError { source })?;

    let path = directory.join("skeleton.json");
    let json = sonic_rs::to_string_pretty(skeleton)?;
    write_file(&path, json.as_bytes())?;

    tracing::info!("Exported '{}'", path.display());

    Ok(())
}

/// Reads, decodes, converts, serializes, and writes one animation.
///
/// This function is shared by explicit animation files and animations
/// discovered inside directories.
///
/// # Errors
///
/// Returns [`AnyError`] if the animation cannot be read or decoded, conversion
/// or serialization fails, or an output file cannot be written.
async fn export_animation(
    input: &Path,
    root: Option<&Path>,
    skeleton: &Skeleton,
    output: &Output,
    format: Format,
    #[cfg(feature = "fbx")] fps: f32,
    #[cfg(all(feature = "kf", feature = "fbx"))] nif: Option<&serde_fbx::ser::nif_compat::Scene>,
) -> Result<(), AnyError> {
    let bytes = fs::read(input)
        .await
        .map_err(|source| Error::FailedReadFile {
            source,
            path: input.to_owned(),
        })?;

    let output_path = resolve_animation_output(input, root, output, format)?;

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|source| Error::IoError { source })?;
    }

    let animation = Animation::from_bytes(skeleton, &bytes, input)?;

    match format {
        #[cfg(feature = "kf")]
        Format::Kf => {
            write_annotations(&animation, &output_path)?;

            let bytes = niflib_animation::serde_kf::to_kf(animation, skeleton)?;

            write_file(&output_path, &bytes)?;
        }

        #[cfg(feature = "fbx")]
        Format::Fbx | Format::FbxAscii => {
            let config = serde_fbx::ser::Config {
                format: match format {
                    Format::Fbx => serde_fbx::ser::Format::FbxBin,
                    Format::FbxAscii => serde_fbx::ser::Format::FbxAscii,
                    Format::Json => unreachable!(),
                    #[cfg(feature = "kf")]
                    Format::Kf => unreachable!(),
                },
                fps,
            };

            let bytes = serde_fbx::ser::to_fbx(&animation, skeleton, nif, config)?;

            write_file(&output_path, &bytes)?;

            write_annotations(&animation, &output_path)?;
        }

        Format::Json => {
            let json = sonic_rs::to_string_pretty(&animation)?;

            write_file(&output_path, json.as_bytes())?;
        }
    }

    tracing::info!("Exported '{}'", output_path.display());

    Ok(())
}

/// Writes animation annotations next to a KF or FBX animation.
///
/// # Errors
///
/// Returns [`AnyError`] if JSON serialization fails or the annotation file
/// cannot be written.
fn write_annotations(animation: &Animation, animation_path: &Path) -> Result<(), AnyError> {
    let parent = animation_path.parent().unwrap_or_else(|| Path::new(""));

    let stem = animation_path
        .file_stem()
        .ok_or_else(|| invalid_input("animation output path has no file stem"))?;

    let path = parent.join(format!("{}.annotations.json", stem.to_string_lossy()));
    let json = sonic_rs::to_string_pretty(&animation.annotations)?;
    write_file(&path, json.as_bytes())?;

    tracing::info!("Exported '{}'", path.display());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_animation_output_uses_input_name_for_single_animation_directory() {
        let input = Path::new("animations/idle.hkx");
        let output = Output::Directory(PathBuf::from("output"));

        let result = resolve_animation_output(input, None, &output, Format::Fbx).unwrap();

        assert_eq!(result, PathBuf::from("output/idle.fbx"));
    }

    #[test]
    fn resolve_animation_output_preserves_directory_structure() {
        let input = Path::new("animations/actors/human/idle.hkx");
        let root = Path::new("animations");
        let output = Output::Directory(PathBuf::from("output"));

        let result = resolve_animation_output(input, Some(root), &output, Format::Fbx).unwrap();

        assert_eq!(result, PathBuf::from("output/actors/human/idle.fbx"));
    }

    #[test]
    fn resolve_animation_output_uses_explicit_file_unchanged() {
        let input = Path::new("animations/idle.hkx");
        let output = Output::File(PathBuf::from("custom.animation"));

        let result = resolve_animation_output(input, None, &output, Format::Fbx).unwrap();

        assert_eq!(result, PathBuf::from("custom.animation"));
    }

    #[test]
    fn resolve_output_allows_arbitrary_file_extension() {
        let output = Path::new("custom_animation.any");

        let result = resolve_output(Some(output), 1, false, Format::Kf).unwrap();

        assert_eq!(result, Output::File(output.to_owned()));
    }

    #[test]
    fn resolve_output_requires_directory_for_json() {
        let output = Path::new("animation.json");

        let result = resolve_output(Some(output), 1, false, Format::Json).unwrap();

        assert_eq!(result, Output::Directory(output.to_owned()));
    }
}
