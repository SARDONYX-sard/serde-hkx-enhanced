//! Export Havok HKX animations to KF, FBX, or JSON.

#[cfg(feature = "fbx")]
use std::collections::HashMap;
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

use super::actor::{EXPECTED_TREE, EXTRACT_DIR_NAME, RigArgs, RigResolver};
#[cfg(feature = "fbx")]
use super::actor::{
    NifSource, Rig, TextureMode, externalize_textures, filter_nifs, load_meshes, relative_textures,
};
use super::{Output, invalid_input, is_serde_hkx_supported_extension, write_file};

const EXAMPLES: &str = r#"Examples

- <blue!>Export a custom animation using the vanilla skeleton, meshes and textures of an actor</blue!>
  <cyan!>hkxc exportrig --actor</cyan!> cow <cyan!>-a</cyan!> ./myanim.hkx <cyan!>-v</cyan!> fbx

- <blue!>Export a mod's animations, inferring the actor from each path</blue!>
  <cyan!>hkxc exportrig --auto -a</cyan!> ./MyMod/meshes/actors <cyan!>-o</cyan!> ./out <cyan!>-v</cyan!> fbx

- <blue!>Export a single animation to KF</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations/idle.hkx <cyan!>-v</cyan!> kf

- <blue!>Export a single animation to FBX</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations/idle.hkx <cyan!>-v</cyan!> fbx

- <blue!>Export a single animation to JSON</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations/idle.hkx <cyan!>-v</cyan!> json

- <blue!>Export an animation with annotations</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations/idle.hkx <cyan!>-v</cyan!> kf
  <cyan!>→</cyan!> ./output/idle.kf
  <cyan!>→</cyan!> ./output/idle.annotations.json

- <blue!>Export to FBX with a NIF mesh skinned to the skeleton</blue!>
  <cyan!>hkxc exportrig -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations/idle.hkx <cyan!>-v</cyan!> fbx <cyan!>--nif</cyan!> ./meshes/body.nif

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
"#;

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
    pub(super) const fn extension(self) -> &'static str {
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
#[clap(arg_required_else_help = true, after_long_help = crate::ui::paint(&format!("{EXAMPLES}\n{EXPECTED_TREE}")))]
pub(crate) struct Args {
    /// Skeleton HKX path. Not needed with --actor or --auto.
    #[clap(short = 's', long, value_name = "SKELETON", required_unless_present_any = ["actor", "auto"])]
    pub skeleton: Option<PathBuf>,

    #[clap(flatten)]
    pub rig: RigArgs,

    /// One or more animation HKX files or directories.
    ///
    /// Directories are searched for HKX files. Multiple files and directories
    /// may be specified.
    #[clap(short = 'a', long, value_name = "ANIMATION", num_args = 1..)]
    pub animations: Vec<PathBuf>,

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

    /// Scene frame rate written to FBX.
    ///
    /// Defaults to the frame rate of each HKX animation, which keeps every key
    /// on an integer frame in Blender.
    #[cfg(feature = "fbx")]
    #[clap(long)]
    pub fps: Option<f32>,

    /// NIF mesh whose shapes are added to every exported FBX, skinned to the
    /// skeleton (Skyrim LE/SE). With --actor/--auto the actor's meshes are
    /// used by default.
    #[cfg(feature = "fbx")]
    #[clap(long, value_name = "NIF")]
    pub nif: Option<PathBuf>,

    /// Do not add meshes to FBX.
    #[cfg(feature = "fbx")]
    #[clap(long)]
    pub no_mesh: bool,

    /// Only use the actor's NIFs whose file name contains this text
    /// (repeatable), e.g. `--mesh femalebody --mesh femalehands`.
    #[cfg(feature = "fbx")]
    #[clap(long, value_name = "NAME")]
    pub mesh: Vec<String>,

    /// How FBX meshes get their textures.
    #[cfg(feature = "fbx")]
    #[clap(long, value_enum, default_value = "embed")]
    pub textures: TextureMode,
}

/// Exports Havok HKX animations to KF, FBX, or JSON.
///
/// The skeleton is decoded once and shared by all animation exports.
/// Each animation is read, decoded, converted, serialized, and written
/// independently.
///
/// # Errors
///
/// Returns [`AnyError`] if an input path is invalid, the skeleton or an
/// animation cannot be read or decoded, conversion or serialization fails,
/// the output configuration is invalid, or an output file cannot be written.
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
        && args.fps.is_some_and(|fps| !fps.is_finite() || fps <= 0.0)
    {
        return Err(invalid_input("--fps must be a finite value greater than zero").into());
    }

    let mut resolver = RigResolver::new(&args.rig, args.skeleton.as_deref())?;
    if args.skeleton.is_none() {
        resolver = resolver.with_extract_dir(output_root(args).join(EXTRACT_DIR_NAME));
    }

    if let Output::Directory(directory) = &output {
        fs::create_dir_all(directory)
            .await
            .map_err(|source| Error::IoError { source })?;
    }

    // Skeleton (and meshes) per rig, shared by its animations.
    #[cfg(feature = "fbx")]
    let mut meshes_by_rig: HashMap<String, Arc<Vec<serde_fbx::ser::Mesh>>> = HashMap::new();
    let mut tasks = JoinSet::new();
    let mut skeleton_json_written = false;
    let bar = crate::ui::progress_bar(animations.len() as u64, "Exporting");

    for (path, root) in animations {
        bar.set_message(format!("resolving skeleton: {}", path.display()));
        let rig = resolver.resolve(&path)?;
        let skeleton = Arc::clone(&rig.skeleton);
        let output = output.clone();
        let format = args.format;

        if matches!(args.format, Format::Json) && !skeleton_json_written {
            export_skeleton_json(&skeleton, &output).await?;
            skeleton_json_written = true;
        }

        #[cfg(feature = "fbx")]
        let fps = args.fps;
        #[cfg(feature = "fbx")]
        let texture_root =
            (args.textures == TextureMode::External).then(|| texture_root(args, &resolver));
        #[cfg(feature = "fbx")]
        let meshes = match meshes_by_rig.get(&rig.id) {
            Some(meshes) => Arc::clone(meshes),
            None => {
                bar.set_message(format!("loading meshes and textures: {}", rig.id));
                let meshes = Arc::new(rig_meshes(args, &resolver, &rig)?);
                meshes_by_rig.insert(rig.id.clone(), Arc::clone(&meshes));
                meshes
            }
        };

        tasks.spawn(async move {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            export_animation(
                &path,
                root.as_deref(),
                &skeleton,
                &output,
                format,
                #[cfg(feature = "fbx")]
                fps,
                #[cfg(feature = "fbx")]
                &meshes,
                #[cfg(feature = "fbx")]
                texture_root.as_deref(),
            )
            .await
            .map(|()| name)
        });
    }

    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(name)) => {
                bar.set_message(name);
                bar.inc(1);
            }
            Ok(Err(error)) => {
                bar.abandon();
                return Err(error);
            }
            Err(error) => {
                bar.abandon();
                return Err(error.into());
            }
        }
    }
    bar.finish_and_clear();
    crate::ui::info(&format!(
        "<green>Exported {} animations</green>",
        bar.position()
    ));

    Ok(())
}

/// Meshes for FBX output: `--nif`, else the actor's meshes.
#[cfg(feature = "fbx")]
fn rig_meshes(
    args: &Args,
    resolver: &RigResolver,
    rig: &Rig,
) -> Result<Vec<serde_fbx::ser::Mesh>, Error> {
    if args.no_mesh || !matches!(args.format, Format::Fbx | Format::FbxAscii) {
        return Ok(Vec::new());
    }
    let textures = args.textures != TextureMode::None;

    let mut meshes = if let Some(nif) = &args.nif {
        // Textures: loose next to the NIF, then the Data directory if any.
        let data = if textures { resolver.data().ok() } else { None };
        load_meshes(NifSource::File(nif), data.as_deref(), textures)?
    } else {
        let nifs = filter_nifs(&rig.nifs, &args.mesh);
        if nifs.is_empty() {
            return Ok(Vec::new());
        }
        let data = resolver.data()?;
        let mut meshes = Vec::new();
        for nif in &nifs {
            if let Err(error) = resolver.read_data(nif) {
                tracing::warn!("skipping mesh {nif}: {error}");
                continue;
            }
            match load_meshes(NifSource::Data(nif), Some(&data), textures) {
                Ok(mut m) => meshes.append(&mut m),
                Err(error) => tracing::warn!("skipping mesh {nif}: {error}"),
            }
        }
        meshes
    };

    if args.textures == TextureMode::External {
        externalize_textures(&mut meshes, &texture_root(args, resolver))?;
    }
    Ok(meshes)
}

/// Directory receiving `textures/...` with `--textures external`:
/// the extract directory when used, else the output directory.
#[cfg(feature = "fbx")]
fn texture_root(args: &Args, resolver: &RigResolver) -> PathBuf {
    resolver
        .extract_dir()
        .map_or_else(|| output_root(args), Path::to_path_buf)
}

/// Output directory (the parent of an explicit output file).
fn output_root(args: &Args) -> PathBuf {
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from("output"));
    if output.extension().is_some() && !output.is_dir() {
        output.parent().map(Path::to_path_buf).unwrap_or_default()
    } else {
        output
    }
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
#[cfg_attr(
    feature = "fbx",
    expect(clippy::too_many_arguments, reason = "feature-gated FBX options")
)]
async fn export_animation(
    input: &Path,
    root: Option<&Path>,
    skeleton: &Skeleton,
    output: &Output,
    format: Format,
    #[cfg(feature = "fbx")] fps: Option<f32>,
    #[cfg(feature = "fbx")] meshes: &[serde_fbx::ser::Mesh],
    #[cfg(feature = "fbx")] texture_root: Option<&Path>,
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

    #[cfg(feature = "fbx")]
    let relative = texture_root.map(|root| relative_textures(meshes, &output_path, root));
    #[cfg(feature = "fbx")]
    let meshes = relative.as_deref().unwrap_or(meshes);

    write_animation(
        &bytes,
        input,
        skeleton,
        &output_path,
        format,
        #[cfg(feature = "fbx")]
        fps,
        #[cfg(feature = "fbx")]
        meshes,
    )
}

/// Decodes one HKX animation (`bytes`, named `input`) and writes it as `format`.
///
/// # Errors
///
/// Returns [`AnyError`] if decoding, conversion, or writing fails.
pub(super) fn write_animation(
    bytes: &[u8],
    input: &Path,
    skeleton: &Skeleton,
    output_path: &Path,
    format: Format,
    #[cfg(feature = "fbx")] fps: Option<f32>,
    #[cfg(feature = "fbx")] meshes: &[serde_fbx::ser::Mesh],
) -> Result<(), AnyError> {
    let animation = Animation::from_bytes(skeleton, bytes, input)?;

    match format {
        #[cfg(feature = "kf")]
        Format::Kf => {
            write_annotations(&animation, output_path)?;
            let bytes = niflib_animation::serde_kf::to_kf(animation, skeleton)?;
            write_file(output_path, &bytes)?;
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
                take_name: input.file_stem().map_or_else(
                    || "Take 001".into(),
                    |stem| stem.to_string_lossy().into_owned(),
                ),
                meshes: meshes.to_vec(),
            };

            let bytes = serde_fbx::ser::to_fbx(&animation, skeleton, config)?;
            write_file(output_path, &bytes)?;

            write_annotations(&animation, output_path)?;
        }

        Format::Json => {
            let json = sonic_rs::to_string_pretty(&animation)?;
            write_file(output_path, json.as_bytes())?;
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
