pub(crate) mod decoder;

use rayon::iter::Either;
use rayon::prelude::*;
use std::path::Path;

use crate::{
    error::Error,
    export::decoder::decode_skeleton_from_bytes,
    ffi::{self, Skeleton},
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
pub fn export_kf(
    skeleton_bytes: &[u8],
    skeleton_path: &Path,
    animations: &[AnimationInput<'_>],
) -> Result<Vec<Vec<u8>>, Error> {
    let skeleton = decode_skeleton_from_bytes(skeleton_bytes, skeleton_path)?;

    let (kf_bytes_list, errors): (Vec<Vec<u8>>, Vec<Error>) =
        animations
            .par_iter()
            .partition_map(|animation| match decode(&skeleton, animation) {
                Ok(kf) => Either::Left(kf),
                Err(e) => Either::Right(e),
            });

    if errors.is_empty() {
        return Ok(kf_bytes_list);
    }

    Err(Error::Errors { errors })
}

fn decode(skeleton: &Skeleton, animation: &AnimationInput<'_>) -> Result<Vec<u8>, Error> {
    let kf = decoder::decode(skeleton, animation)?;
    ffi::export_kf(&kf).map_err(|error| Error::Niflib {
        message: error.to_string(),
    })
}
