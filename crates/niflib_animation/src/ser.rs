use rayon::iter::Either;
use rayon::prelude::*;
use serde_spline::hkx::{Animation, Skeleton};
use std::path::Path;

use crate::{
    error::Error,
    ffi::{self, Kf},
};

pub struct AnimationInput<'a> {
    pub bytes: &'a [u8],
    pub path: &'a Path,
}

/// Converts a skeleton and multiple Havok animations into a KF file.
///
/// # Errors
///
/// Returns [`Error`] when the skeleton or any animation cannot be decoded,
/// when spline data is invalid, or when the native niflib conversion fails.
pub fn to_kf_bytes_vec(
    skeleton_bytes: &[u8],
    skeleton_path: &Path,
    animations: &[AnimationInput<'_>],
) -> Result<Vec<Vec<u8>>, Error> {
    let skeleton = Skeleton::from_bytes(skeleton_bytes, skeleton_path)?;

    let (kf_bytes_list, errors): (Vec<Vec<u8>>, Vec<Error>) =
        animations
            .par_iter()
            .partition_map(|animation| match to_kf(&skeleton, animation) {
                Ok(kf) => Either::Left(kf),
                Err(e) => Either::Right(e),
            });

    if errors.is_empty() {
        return Ok(kf_bytes_list);
    }

    Err(Error::Errors { errors })
}

fn to_kf(skeleton: &Skeleton, animation: &AnimationInput<'_>) -> Result<Vec<u8>, Error> {
    let input = Kf {
        animation: Animation::from_bytes(skeleton, animation.bytes, animation.path)?.into(),
        skeleton: skeleton.into(),
    };
    ffi::export_kf(&input).map_err(|error| Error::Niflib {
        message: error.to_string(),
    })
}
