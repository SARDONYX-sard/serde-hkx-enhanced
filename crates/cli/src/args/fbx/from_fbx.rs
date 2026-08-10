//! Apply FBX animation to Havok HKX animation.

use serde_fbx::ser::{AnimationInput, fbx_to_hkx_bytes_vec};
use serde_hkx_features::error::Error;
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::AnyError;

pub const EXAMPLES: &str = color_print::cstr!(
    r#"Examples

- <blue!>Apply fbx to hkx</blue!>
  <cyan!>hkxc from-fbx -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.fbx <cyan!>-o</cyan!> ./idle.hkx

- <blue!>Convert multiple fbx animations</blue!>
  <cyan!>hkxc from-fbx -s</cyan!> ./skeleton.hkx <cyan!>-a</cyan!> ./idle.fbx ./walk.fbx <cyan!>-o</cyan!> ./out/
  "#
);

#[derive(Debug, clap::Args)]
#[clap(arg_required_else_help = true, after_long_help = EXAMPLES)]
pub(crate) struct Args {
    /// Input skeleton HKX file.
    #[clap(short = 's', long, value_name = "SKELETON")]
    pub skeleton: PathBuf,

    /// One or more FBX animation files to convert.
    #[clap(short = 'a', long, value_name = "FBX", num_args = 1..)]
    pub anim: Vec<PathBuf>,

    /// Output directory, or an explicit .hkx path for a single animation.
    #[clap(short, long)]
    pub output: Option<PathBuf>,

    /// Frames per second for sampling.
    #[clap(long, default_value = "30.0")]
    pub fps: f32,
}

/// Converts FBX animations into Havok HKX animations.
///
/// The input HKX is used as the skeleton source. Each FBX animation is
/// converted independently and the resulting HKX files are written to the
/// output path.
///
/// # Errors
///
/// Returns [`serde_hkx_features::error::Error`] if the input files cannot be
/// read, the FBX animation cannot be converted, or an output file cannot be
/// written.
pub async fn from_fbx(args: &Args) -> Result<(), AnyError> {
    if args.anim.is_empty() {
        return Err(serde_hkx_features::error::Error::IoError {
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "at least one --anim path is required",
            ),
        }
        .into());
    }

    if !args.fps.is_finite() || args.fps <= 0.0 {
        return Err(serde_hkx_features::error::Error::IoError {
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "--fps must be a finite value greater than zero",
            ),
        }
        .into());
    }

    let skeleton_bytes = fs::read(&args.skeleton)
        .map_err(|source| serde_hkx_features::error::Error::IoError { source })?;

    let mut animation_bytes = Vec::with_capacity(args.anim.len());

    for path in &args.anim {
        let bytes = fs::read(path)
            .map_err(|source| serde_hkx_features::error::Error::IoError { source })?;

        animation_bytes.push((path, bytes));
    }

    let animations = animation_bytes
        .iter()
        .map(|(path, bytes)| AnimationInput {
            bytes,
            path: path.as_path(),
            animation_stack: None,
            annotations: Vec::new(),
        })
        .collect::<Vec<_>>();

    let hkx_bytes = fbx_to_hkx_bytes_vec(&skeleton_bytes, &args.skeleton, &animations, args.fps)?;

    let output = args.output.as_deref();

    write_outputs(output, &args.anim, hkx_bytes)?;

    Ok(())
}

fn write_outputs(
    output: Option<&Path>,
    animation_paths: &[PathBuf],
    hkx_bytes: Vec<Vec<u8>>,
) -> Result<(), Error> {
    if hkx_bytes.len() != animation_paths.len() {
        return Err(serde_hkx_features::error::Error::IoError {
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
                .with_extension("hkx"),
            None => animation_paths[0].with_extension("hkx"),
        };

        fs::write(&output_path, &hkx_bytes[0])
            .map_err(|source| serde_hkx_features::error::Error::IoError { source })?;

        tracing::info!("Written '{}'", output_path.display());

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

    fs::create_dir_all(&output_dir)
        .map_err(|source| serde_hkx_features::error::Error::IoError { source })?;

    for (animation_path, bytes) in animation_paths.iter().zip(hkx_bytes) {
        let output_path = output_dir
            .join(
                animation_path
                    .file_stem()
                    .unwrap_or_else(|| std::ffi::OsStr::new("animation")),
            )
            .with_extension("hkx");

        fs::write(&output_path, bytes)
            .map_err(|source| serde_hkx_features::error::Error::IoError { source })?;

        tracing::info!("Written '{}'", output_path.display());
    }

    Ok(())
}
