mod fbx;
mod fbx_error;

use rayon::iter::Either;
use rayon::prelude::*;
use serde_spline::hkx::Skeleton;
use std::path::Path;

use crate::error::Error;

pub struct AnimationInput<'a> {
    pub bytes: &'a [u8],
    pub path: &'a Path,
}

/// Converts a skeleton and multiple Havok animations into FBX files.
///
/// Each input animation is exported independently and produces one FBX byte
/// buffer. The order of the returned buffers matches the order of `animations`.
///
/// # Errors
///
/// Returns [`Error`] when the skeleton or any animation cannot be decoded,
/// when the animation data is invalid, or when FBX serialization fails.
pub fn export_fbx(
    skeleton_bytes: &[u8],
    skeleton_path: &Path,
    animations: &[AnimationInput<'_>],
    fps: f32,
) -> Result<Vec<Vec<u8>>, Error> {
    let skeleton = Skeleton::from_bytes(skeleton_bytes, skeleton_path)?;

    let (fbx_bytes_list, errors): (Vec<Vec<u8>>, Vec<Error>) =
        animations.par_iter().partition_map(|animation| {
            match fbx::export_fbx(&skeleton, animation, fps) {
                Ok(fbx) => Either::Left(fbx),
                Err(error) => Either::Right(error),
            }
        });

    if errors.is_empty() {
        return Ok(fbx_bytes_list);
    }

    Err(Error::Errors { errors })
}
