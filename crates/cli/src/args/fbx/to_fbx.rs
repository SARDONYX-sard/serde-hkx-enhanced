//! Convert Havok HKX animations to FBX scenes.

use serde_fbx::de::{AnimationInput, export_fbx};
use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_hkx_features::error::Error;

use crate::AnyError;

pub const EXAMPLES: &str = color_print::cstr!(
    r#"Examples

- <blue!>skeleton + animation -> fbx</blue!>
  <cyan!>hkxc to-fbx -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations/idle.hkx <cyan!>-o</cyan!> ./idle.fbx

- <blue!>multiple animations -> fbx</blue!>
  <cyan!>hkxc to-fbx -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.hkx ./walk.hkx <cyan!>-o</cyan!> ./out/

- <blue!>project directory -> fbx</blue!>
  <cyan!>hkxc to-fbx</cyan!> ./characters/defaultmale/
  "#
);

#[derive(Debug, clap::Args)]
#[clap(arg_required_else_help = true, after_long_help = EXAMPLES)]
pub(crate) struct Args {
    /// Project directory (auto-discovers skeleton.hkx + animations/*.hkx),
    /// OR explicit skeleton path when used with --anim.
    #[clap(value_name = "DIR_OR_SKEL")]
    pub input: Option<PathBuf>,

    /// Skeleton HKX path (required when --anim is specified).
    #[clap(short = 's', long, value_name = "SKELETON")]
    pub skeleton: Option<PathBuf>,

    /// One or more animation HKX files to export.
    #[clap(short = 'a', long, value_name = "ANIM", num_args = 1..)]
    pub anim: Vec<PathBuf>,

    /// Output directory, or an explicit .fbx path for a single animation.
    #[clap(short, long)]
    pub output: Option<PathBuf>,

    /// Do not recurse into subdirectories.
    #[clap(short = 'n', long)]
    pub no_recursive: bool,

    /// Frames per second used when sampling the animations.
    #[clap(long, default_value = "30.0")]
    pub fps: f32,
}

/// Converts Havok HKX animations into FBX scenes.
///
/// The skeleton HKX provides the skeleton while each animation HKX provides
/// the animation data. Each input animation produces one FBX file.
///
/// # Errors
///
/// Returns [`Error`] if an input file cannot be read, the HKX data cannot be
/// converted, the FBX scene cannot be exported, or an output file cannot be
/// written.
pub fn to_fbx(args: &Args) -> Result<(), AnyError> {
    if let Some(skeleton) = &args.skeleton {
        return convert_explicit(skeleton, &args.anim, args.output.as_deref(), args.fps);
    }

    convert_project(args)
}

fn convert_explicit(
    skeleton_path: &Path,
    animation_paths: &[PathBuf],
    output: Option<&Path>,
    fps: f32,
) -> Result<(), AnyError> {
    if animation_paths.is_empty() {
        return Err(Error::IoError {
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "--skeleton requires at least one --anim path",
            ),
        }
        .into());
    }

    if !fps.is_finite() || fps <= 0.0 {
        return Err(Error::IoError {
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "--fps must be a finite value greater than zero",
            ),
        }
        .into());
    }

    let skeleton_bytes = fs::read(skeleton_path).map_err(|source| Error::FailedReadFile {
        source,
        path: skeleton_path.to_path_buf(),
    })?;

    let mut owned_animations = Vec::with_capacity(animation_paths.len());

    for path in animation_paths {
        let bytes = fs::read(path).map_err(|source| Error::FailedReadFile {
            source,
            path: path.clone(),
        })?;

        owned_animations.push((path, bytes));
    }

    let inputs = owned_animations
        .iter()
        .map(|(path, bytes)| AnimationInput {
            bytes,
            path: path.as_path(),
        })
        .collect::<Vec<_>>();

    let fbx_bytes = export_fbx(&skeleton_bytes, skeleton_path, &inputs, fps)?;

    Ok(write_outputs(output, animation_paths, fbx_bytes)?)
}

fn write_outputs(
    output: Option<&Path>,
    animation_paths: &[PathBuf],
    fbx_bytes: Vec<Vec<u8>>,
) -> Result<(), Error> {
    if fbx_bytes.len() != animation_paths.len() {
        return Err(Error::IoError {
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "conversion result count does not match animation count",
            ),
        });
    }

    if animation_paths.len() == 1 {
        let output_path = match output {
            Some(path) if path.extension().is_some() => path.to_owned(),
            Some(path) => path
                .join(
                    animation_paths[0]
                        .file_stem()
                        .unwrap_or_else(|| std::ffi::OsStr::new("animation")),
                )
                .with_extension("fbx"),
            None => animation_paths[0].with_extension("fbx"),
        };

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::IoError { source })?;
        }

        fs::write(&output_path, &fbx_bytes[0]).map_err(|source| Error::IoError { source })?;

        tracing::info!("Exported '{}'", output_path.display());

        return Ok(());
    }

    let output_dir = output.map_or_else(
        || {
            animation_paths[0]
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_owned()
        },
        Path::to_owned,
    );

    fs::create_dir_all(&output_dir).map_err(|source| Error::IoError { source })?;

    for (animation_path, bytes) in animation_paths.iter().zip(fbx_bytes) {
        let output_path = output_path(&output_dir, animation_path);

        fs::write(&output_path, bytes).map_err(|source| Error::IoError { source })?;

        tracing::info!("Exported '{}'", output_path.display());
    }

    Ok(())
}

fn output_path(output_dir: &Path, animation_path: &Path) -> PathBuf {
    let stem = animation_path
        .file_stem()
        .unwrap_or_else(|| std::ffi::OsStr::new("animation"));

    output_dir.join(stem).with_extension("fbx")
}

fn convert_project(args: &Args) -> Result<(), AnyError> {
    let input = match &args.input {
        Some(path) => path,
        None => {
            return Err(Error::IoError {
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Provide a project directory or use --skeleton / --anim",
                ),
            }
            .into());
        }
    };

    let skeleton_path = input.join("skeleton.hkx");
    let animation_dir = input.join("animations");

    let animation_paths = discover_animations(&animation_dir, !args.no_recursive)?;

    convert_explicit(
        &skeleton_path,
        &animation_paths,
        args.output.as_deref(),
        args.fps,
    )
}

fn discover_animations(directory: &Path, recursive: bool) -> Result<Vec<PathBuf>, Error> {
    let mut paths = Vec::new();

    collect_animations(directory, recursive, &mut paths)?;

    paths.sort();

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

            continue;
        }

        let is_hkx = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("hkx"));

        if is_hkx {
            output.push(path);
        }
    }

    Ok(())
}
