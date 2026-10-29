// SPDX-FileCopyrightText: Copyright (c) 2011 figment
// SPDX-License-Identifier: BSD-3-Clause
//
// https://github.com/jgernandt/hkxcmd/tree/51260f796d1d19255936b2e35f33fa752391083a/Addins/ConvertKF.cpp

//! KF -> Havok animation.

use std::collections::HashMap;

use havok_types::QsTransform;
use serde_hkx_features::Format;
use serde_nif::kf::{
    INVALID_POSE, KeyGroup, KeyType, Quaternion, Rotations, Sequence, TransformInterpolator,
};
use serde_spline::hkx::{Animation, AnimationAnnotation, AnimationFrame, Skeleton, ser::to_hkx};

use crate::Error;

/// Key times closer than this are treated as equal (KF times are `f32` seconds).
const TIME_EPSILON: f32 = 1.0e-5;

#[derive(Debug)]
pub struct DeConfig {
    /// Overrides the KF text keys when non-empty (they carry no track index).
    pub annotations: Vec<AnimationAnnotation>,
    /// Sampling rate of the output animation.
    pub fps: f32,
    pub format: Format,
}

impl Default for DeConfig {
    fn default() -> Self {
        Self {
            annotations: Vec::new(),
            fps: 30.0,
            format: Format::Amd64,
        }
    }
}

/// Converts a KF animation into Havok HKX bytes.
///
/// # Errors
/// If the KF cannot be read or sampled, or the HKX cannot be encoded.
pub fn from_kf(bytes: &[u8], skeleton: &Skeleton, config: DeConfig) -> Result<Vec<u8>, Error> {
    let DeConfig {
        annotations,
        fps,
        format,
    } = config;

    let mut animation = kf_to_animation(bytes, skeleton, fps)?;
    if !annotations.is_empty() {
        animation.annotations = annotations;
    }
    Ok(to_hkx(skeleton, &animation, fps, format)?)
}

/// Samples a KF animation at `fps` into the skeleton's track order.
///
/// Bones without a controlled block keep their reference pose, so the result
/// always has one track per skeleton bone.
///
/// # Errors
/// If the input is empty, `fps` is not positive, or the KF is malformed.
pub fn kf_to_animation(bytes: &[u8], skeleton: &Skeleton, fps: f32) -> Result<Animation, Error> {
    if bytes.is_empty() {
        return Err(Error::invalid("KF input is empty"));
    }
    if skeleton.bones.is_empty() {
        return Err(Error::invalid("skeleton contains no bones"));
    }
    if !fps.is_finite() || fps <= 0.0 {
        return Err(Error::invalid(
            "KF sampling FPS must be finite and greater than zero",
        ));
    }

    let sequence = Sequence::from_bytes(bytes)?;
    let duration = sequence.stop_time - sequence.start_time;
    if !duration.is_finite() || duration < 0.0 {
        return Err(Error::invalid(
            "KF controller sequence has an invalid time range",
        ));
    }
    let num_frames = (duration * fps).round() as u32 + 1;

    let mut frames: Vec<_> = (0..num_frames)
        .map(|_| AnimationFrame {
            transforms: skeleton
                .bones
                .iter()
                .map(|bone| bone.reference_pose.clone())
                .collect(),
        })
        .collect();

    let bone_indices: HashMap<&str, usize> = skeleton
        .bones
        .iter()
        .enumerate()
        .map(|(index, bone)| (bone.name.as_str(), index))
        .collect();
    for block in &sequence.controlled_blocks {
        let (Some(&bone), Some(interpolator)) =
            (bone_indices.get(&*block.node_name), &block.interpolator)
        else {
            continue;
        };
        for (frame, transforms) in frames.iter_mut().enumerate() {
            // Key times are absolute; the clip starts at `start_time`, not 0.
            let time = (frame as f32 / fps + sequence.start_time).min(sequence.stop_time);
            sample(interpolator, time, &mut transforms.transforms[bone]);
        }
    }

    Ok(Animation {
        duration,
        num_frames,
        num_tracks: skeleton.bones.len() as u32,
        frames,
        annotations: sequence
            .text_keys
            .iter()
            .map(|key| AnimationAnnotation {
                track_index: 0,
                time: key.time - sequence.start_time,
                text: key.text.to_string(),
            })
            .collect(),
    })
}

/// Overwrites the channels of `out` that the interpolator animates.
///
/// A channel without keys falls back to the interpolator's pose, and to the
/// skeleton's reference pose (already in `out`) when that pose is unset.
fn sample(interpolator: &TransformInterpolator, time: f32, out: &mut QsTransform) {
    let data = interpolator.data.as_ref();

    let translation = data
        .and_then(|data| sample_keys(&data.translations, time, lerp3))
        .or_else(|| valid_pose(interpolator.translation));
    if let Some(translation) = translation {
        [out.transition.x, out.transition.y, out.transition.z] = translation;
    }

    let rotation = data
        .and_then(|data| sample_rotation(&data.rotations, time))
        .or_else(|| {
            let q = interpolator.rotation;
            valid_pose([q.x, q.y, q.z, q.w]).map(|_| q)
        });
    if let Some(q) = rotation {
        // The spline compressor reconstructs the dropped component of a unit
        // quaternion, so anything else would distort the rotation.
        let q = normalize(q);
        let out = &mut out.quaternion;
        (out.x, out.y, out.z, out.scaler) = (q.x, q.y, q.z, q.w);
    }

    let scale = data
        .and_then(|data| sample_keys(&data.scales, time, lerp))
        .or_else(|| valid_pose([interpolator.scale]).map(|[s]| s));
    if let Some(s) = scale {
        (out.scale.x, out.scale.y, out.scale.z) = (s, s, s);
    }
}

/// Gamebryo writes `-FLT_MAX` into pose components that are not set.
fn valid_pose<const N: usize>(values: [f32; N]) -> Option<[f32; N]> {
    // An exact sentinel, so compare bits rather than with a tolerance.
    values
        .iter()
        .all(|v| v.to_bits() != INVALID_POSE.to_bits())
        .then_some(values)
}

/// The value at `time`, or `None` without keys.
///
/// Every key type is interpolated linearly, as hkxcmd does: quadratic/TBC
/// tangents only matter between keys, and exported KF files carry one key
/// per frame.
fn sample_keys<T: Copy>(group: &KeyGroup<T>, time: f32, lerp: fn(T, T, f32) -> T) -> Option<T> {
    let keys = &group.keys;
    let next = keys.partition_point(|key| key.time <= time);
    let Some(previous) = next.checked_sub(1).map(|i| &keys[i]) else {
        return keys.first().map(|key| key.value);
    };
    let Some(next) = keys.get(next) else {
        return Some(previous.value);
    };

    let interval = next.time - previous.time;
    if group.key_type == KeyType::Constant
        || interval < TIME_EPSILON
        || time - previous.time < TIME_EPSILON
    {
        return Some(previous.value);
    }
    if next.time - time < TIME_EPSILON {
        return Some(next.value);
    }
    Some(lerp(
        previous.value,
        next.value,
        (time - previous.time) / interval,
    ))
}

fn sample_rotation(rotations: &Rotations, time: f32) -> Option<Quaternion> {
    match rotations {
        Rotations::Quaternion(group) => sample_keys(group, time, nlerp),
        Rotations::Euler(axes) => {
            if axes.iter().all(|axis| axis.keys.is_empty()) {
                return None;
            }
            let [x, y, z] = [0, 1, 2].map(|i| sample_keys(&axes[i], time, lerp).unwrap_or(0.0));
            Some(euler_xyz(x, y, z))
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    (b - a).mul_add(t, a)
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    core::array::from_fn(|i| lerp(a[i], b[i], t))
}

/// Normalized lerp along the shorter arc (`q` and `-q` are the same rotation;
/// without the flip, interpolating between them would pass through zero).
fn nlerp(a: Quaternion, b: Quaternion, t: f32) -> Quaternion {
    let dot =
        a.w.mul_add(b.w, a.z.mul_add(b.z, a.x.mul_add(b.x, a.y * b.y)));
    let sign = if dot < 0.0 { -1.0 } else { 1.0 };
    normalize(Quaternion {
        x: lerp(a.x, sign * b.x, t),
        y: lerp(a.y, sign * b.y, t),
        z: lerp(a.z, sign * b.z, t),
        w: lerp(a.w, sign * b.w, t),
    })
}

fn normalize(q: Quaternion) -> Quaternion {
    let length_sq =
        q.w.mul_add(q.w, q.z.mul_add(q.z, q.x.mul_add(q.x, q.y * q.y)));
    if length_sq <= 1.0e-12 {
        return q;
    }
    let inv = length_sq.sqrt().recip();
    Quaternion {
        x: q.x * inv,
        y: q.y * inv,
        z: q.z * inv,
        w: q.w * inv,
    }
}

/// Euler angles applied X, then Y, then Z (`Rz * Ry * Rx`), the order the
/// Blender NIF importer uses for `XYZ_ROTATION_KEY` data.
fn euler_xyz(x: f32, y: f32, z: f32) -> Quaternion {
    let (sx, cx) = (x * 0.5).sin_cos();
    let (sy, cy) = (y * 0.5).sin_cos();
    let (sz, cz) = (z * 0.5).sin_cos();
    Quaternion {
        x: (sx * cy).mul_add(cz, -(cx * sy * sz)),
        y: (cx * sy).mul_add(cz, sx * cy * sz),
        z: (cx * cy).mul_add(sz, -(sx * sy * cz)),
        w: (cx * cy).mul_add(cz, sx * sy * sz),
    }
}

#[cfg(test)]
mod tests {
    use serde_nif::kf::Key;

    use super::*;

    #[test]
    fn samples_between_keys() {
        let group = KeyGroup {
            key_type: KeyType::Linear,
            keys: vec![Key::new(0.0, 0.0), Key::new(1.0, 10.0)],
        };
        assert_eq!(sample_keys(&group, -1.0, lerp), Some(0.0));
        assert_eq!(sample_keys(&group, 0.25, lerp), Some(2.5));
        assert_eq!(sample_keys(&group, 2.0, lerp), Some(10.0));

        let constant = KeyGroup {
            key_type: KeyType::Constant,
            ..group
        };
        assert_eq!(sample_keys(&constant, 0.75, lerp), Some(0.0));
        assert_eq!(sample_keys(&KeyGroup::<f32>::default(), 0.5, lerp), None);
    }

    #[test]
    fn euler_order_is_x_then_y_then_z() {
        use std::f32::consts::FRAC_PI_2;
        // X by 90° then Z by 90° must equal the product qz * qx.
        let q = euler_xyz(FRAC_PI_2, 0.0, FRAC_PI_2);
        let h = FRAC_PI_2 * 0.5;
        let (s, c) = h.sin_cos();
        // qz * qx = (c + s k)(c + s i) = c² + cs i + cs k + s² j
        let expected = [c * s, s * s, c * s, c * c];
        for (a, b) in [q.x, q.y, q.z, q.w].into_iter().zip(expected) {
            assert!((a - b).abs() < 1.0e-6, "{q:?}");
        }
    }
}
