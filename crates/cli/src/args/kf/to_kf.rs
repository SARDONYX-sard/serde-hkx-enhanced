//! Convert Havok HKX animation to Gamebryo KF animation.

use niflib_animation::{
    error::Error,
    export::{AnimationInput, export_kf},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const EXAMPLES: &str = color_print::cstr!(
    r#"Examples

- <blue!>skeleton + animation -> kf</blue!>
  <cyan!>hkxc to-kf -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./animations/idle.hkx <cyan!>-o</cyan!> ./out/
- <blue!>multiple animations -> kf</blue!>
  <cyan!>hkxc to-kf -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.hkx ./walk.hkx <cyan!>-o</cyan!> ./out/
- <blue!>project directory -> kf</blue!>
  <cyan!>hkxc to-kf</cyan!> ./characters/defaultmale/
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

    /// One or more animation HKX files to convert.
    #[clap(short = 'a', long, value_name = "ANIM", num_args = 1..)]
    pub anim: Vec<PathBuf>,

    /// Output directory.
    #[clap(short, long)]
    pub output: Option<PathBuf>,

    /// Export float tracks in addition to transform tracks.
    #[clap(long)]
    pub float_tracks: bool,

    /// Do not recurse into subdirectories.
    #[clap(short = 'n', long)]
    pub no_recursive: bool,

    /// NIF user version (default: 11 = Skyrim LE).
    #[clap(short = 'u', long, default_value = "11")]
    pub user_version: u32,
}

pub fn to_kf(args: &Args) -> Result<(), Error> {
    if let Some(skeleton) = &args.skeleton {
        return convert_explicit(skeleton, &args.anim, args.output.as_deref());
    }

    convert_project(args)
}

fn convert_explicit(
    skeleton_path: &Path,
    animation_paths: &[PathBuf],
    output: Option<&Path>,
) -> Result<(), Error> {
    if animation_paths.is_empty() {
        return Err(serde_hkx_features::error::Error::IoError {
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "--skeleton requires at least one --anim path",
            ),
        }
        .into());
    }

    let skeleton_bytes = fs::read(skeleton_path).map_err(|source| {
        serde_hkx_features::error::Error::FailedReadFile {
            source,
            path: skeleton_path.to_path_buf(),
        }
    })?;

    let mut owned_animations = Vec::with_capacity(animation_paths.len());
    for path in animation_paths {
        let bytes =
            fs::read(path).map_err(|source| serde_hkx_features::error::Error::FailedReadFile {
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

    let kf_bytes = export_kf(&skeleton_bytes, skeleton_path, &inputs)?;

    let output_dir = output.map_or_else(
        || {
            skeleton_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_owned()
        },
        Path::to_owned,
    );

    fs::create_dir_all(&output_dir)
        .map_err(|source| serde_hkx_features::error::Error::IoError { source })?;

    for (animation_path, bytes) in animation_paths.iter().zip(kf_bytes) {
        let output_path = output_path(&output_dir, animation_path);

        fs::write(&output_path, bytes)
            .map_err(|source| serde_hkx_features::error::Error::IoError { source })?;

        tracing::info!("Exported '{}'", output_path.display());
    }

    Ok(())
}

fn output_path(output_dir: &Path, animation_path: &Path) -> PathBuf {
    let stem = animation_path
        .file_stem()
        .unwrap_or_else(|| std::ffi::OsStr::new("animation"));

    output_dir.join(stem).with_extension("kf")
}

fn convert_project(args: &Args) -> Result<(), Error> {
    let input = match &args.input {
        Some(path) => path,
        None => {
            return Err(serde_hkx_features::error::Error::IoError {
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

    convert_explicit(&skeleton_path, &animation_paths, args.output.as_deref())
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
    let entries = fs::read_dir(directory)
        .map_err(|source| serde_hkx_features::error::Error::IoError { source })?;

    for entry in entries {
        let entry = entry.map_err(|source| serde_hkx_features::error::Error::IoError { source })?;

        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| serde_hkx_features::error::Error::IoError { source })?;

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
