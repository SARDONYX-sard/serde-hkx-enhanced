use serde_hkx_features::Format;
use serde_spline::hkx::{Animation, AnimationAnnotation, Skeleton, ser::to_hkx};

use crate::{error::Error, ffi};

/// Converts a skeleton and multiple Havok animations into a KF file.
///
/// # Errors
/// Returns [`Error`] when the skeleton or any animation cannot be decoded,
/// when spline data is invalid, or when the native niflib conversion fails.
pub fn to_kf(animation: Animation, skeleton: &Skeleton) -> Result<Vec<u8>, Error> {
    // cast to ffi types
    let animation = animation.into();
    let skeleton = skeleton.into();

    ffi::export_kf(&skeleton, &animation).map_err(|error| Error::Niflib {
        message: error.to_string(),
    })
}

#[derive(Debug)]
pub struct DeConfig {
    pub annotations: Vec<AnimationAnnotation>,
    pub fps: f32,
    pub format: Format,
}

impl Default for DeConfig {
    fn default() -> Self {
        Self {
            annotations: Default::default(),
            fps: 30.0,
            format: Format::Amd64,
        }
    }
}

/// Converts a Gamebryo KF animation into a Havok HKX animation.
///
/// # Errors
///
/// Returns [`Error`] if the skeleton cannot be decoded, the native KF
/// conversion fails, the animation is invalid, or the resulting HKX cannot
/// be encoded.
pub fn from_kf(bytes: &[u8], skeleton: &Skeleton, config: DeConfig) -> Result<Vec<u8>, Error> {
    let DeConfig {
        annotations,
        fps,
        format,
    } = config;

    let mut animation = {
        let skeleton = ffi::Skeleton::from(skeleton);

        let mut ffi_animation =
            ffi::convert_kf(bytes, &skeleton, fps).map_err(|error| Error::Niflib {
                message: error.to_string(),
            })?;
        ffi_animation.normalize_rotations();

        Animation::from(ffi_animation)
    };
    animation.annotations = annotations;

    Ok(to_hkx(skeleton, &animation, fps, format)?)
}
