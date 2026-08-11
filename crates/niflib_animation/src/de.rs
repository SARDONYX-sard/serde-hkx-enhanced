use rayon::iter::Either;
use rayon::prelude::*;
use serde_hkx_features::Format;
use serde_spline::hkx::{Animation, AnimationAnnotation, Skeleton, ser::to_hkx};
use std::path::Path;

use crate::{error::Error, ffi};

pub struct AnimationInput<'a> {
    /// .kf file bytes
    pub bytes: &'a [u8],
    /// .kf file path
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
pub fn from_kf_bytes_vec_to_hkx<P>(
    skeleton_bytes: &[u8],
    skeleton_path: P,
    kf_animations: Vec<AnimationInput<'_>>,
    fps: f32,
    format: Format,
) -> Result<Vec<Vec<u8>>, Error>
where
    P: AsRef<Path>,
{
    let skeleton = Skeleton::from_bytes(skeleton_bytes, skeleton_path.as_ref())?;

    let (kf_bytes_list, errors): (Vec<Vec<u8>>, Vec<Error>) =
        kf_animations.into_par_iter().partition_map(|animation| {
            match from_kf(&skeleton, animation, fps, format) {
                Ok(kf) => Either::Left(kf),
                Err(e) => Either::Right(e),
            }
        });

    if errors.is_empty() {
        return Ok(kf_bytes_list);
    }

    Err(Error::Errors { errors })
}

fn from_kf(
    skeleton: &Skeleton,
    anim_input: AnimationInput<'_>,
    fps: f32,
    format: Format,
) -> Result<Vec<u8>, Error> {
    let mut animation: Animation =
        ffi::convert_kf(anim_input.bytes, &ffi::Skeleton::from(skeleton), fps)
            .map_err(|error| Error::Niflib {
                message: error.to_string(),
            })?
            .into();

    animation.annotations = anim_input.annotations;

    Ok(to_hkx(skeleton, &animation, fps, format)?)
}
