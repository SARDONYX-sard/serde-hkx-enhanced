mod encoder;
mod ser_builder;

use rayon::iter::Either;
use rayon::prelude::*;
use std::path::Path;

use crate::{
    error::Error,
    export::decoder::decode_skeleton_from_bytes,
    ffi::{self, AnimationAnnotation, Skeleton},
};

pub struct AnimationInput<'a> {
    pub bytes: &'a [u8],
    pub path: &'a Path,
    pub annotations: Vec<AnimationAnnotation>,
}

/// Converts a Gamebryo KF animation into a Havok HKX animation.
///
/// The KF data is decoded by the native NIFLib implementation, while the
/// skeleton is decoded using the existing Rust HKX API. The resulting
/// animation is spline-compressed and serialized as an HKX byte buffer.
///
/// This function performs no file I/O.
///
/// # Errors
///
/// Returns [`Error`] if the skeleton cannot be decoded, the native KF
/// conversion fails, the animation is invalid, or the resulting HKX cannot
/// be encoded.
pub fn convert_kf<P>(
    skeleton_bytes: &[u8],
    skeleton_path: P,
    kf_animations: &[AnimationInput<'_>],
    fps: f32,
) -> Result<Vec<Vec<u8>>, Error>
where
    P: AsRef<Path>,
{
    let skeleton = decode_skeleton_from_bytes(skeleton_bytes, skeleton_path.as_ref())?;

    let (kf_bytes_list, errors): (Vec<Vec<u8>>, Vec<Error>) = kf_animations
        .par_iter()
        .partition_map(|animation| match encode(&skeleton, animation, fps) {
            Ok(kf) => Either::Left(kf),
            Err(e) => Either::Right(e),
        });

    if errors.is_empty() {
        return Ok(kf_bytes_list);
    }

    Err(Error::Errors { errors })
}

fn encode(
    skeleton: &Skeleton,
    anim_input: &AnimationInput<'_>,
    fps: f32,
) -> Result<Vec<u8>, Error> {
    let animation =
        ffi::convert_kf(anim_input.bytes, skeleton, fps).map_err(|error| Error::Niflib {
            message: error.to_string(),
        })?;

    encoder::encode(skeleton, &animation, fps, &anim_input.annotations)
}
