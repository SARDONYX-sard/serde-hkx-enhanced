// SPDX-License-Identifier: GPL-3.0-or-later
//
// Reference:
// - https://github.com/BadDogSkyrim/PyNifly/blob/7fd4644f5a6416c1502983b7d49a853eb0d24509/io_scene_nifly/hkx/anim_skyrim.py

//! Rotation tracks: sign-continuous quaternions packed as THREECOMP40/48.

use rig_math::QuatA16;

use super::fit::{fit_bspline_quaternion, make_clamped_knots};
use super::{align_rotation, encode_knots};
use crate::error::Error;
use crate::spline::track::{
    QuantizationType, SplineDynamicTrackQuat, SplineTrackQuat, SplineTrackType, TransformMask,
    TransformTrack, TransformType,
};

/// Encodes a rotation track.
///
/// PyNifly supports THREECOMP40 and THREECOMP48 for newly generated
/// quaternion data. Uncompressed is also emitted directly because its binary
/// representation is unambiguous.
///
/// # Errors
///
/// Returns [`Error`] if the track representation is inconsistent with its
/// mask, if a quaternion is invalid, or if the quantization format is not
/// supported by this encoder.
pub(super) fn encode_rotation(
    mask: &TransformMask,
    track: &TransformTrack,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    let quantization = mask.rotation_quantization_type()?;
    let kind = mask.sub_track_type(TransformType::Rotation);

    match &track.rotation {
        SplineTrackQuat::Identity => {
            if kind != SplineTrackType::Identity {
                return Err(Error::InvalidData(
                    "rotation track is identity while its mask is not identity",
                ));
            }

            Ok(())
        }

        SplineTrackQuat::Static(static_track) => {
            if kind != SplineTrackType::Static {
                return Err(Error::InvalidData(
                    "rotation track is static while its mask is not static",
                ));
            }

            align_rotation(out, quantization);

            write_quaternion(out, static_track.value, quantization)
        }

        SplineTrackQuat::Dynamic(dynamic_track) => {
            if kind != SplineTrackType::Dynamic {
                return Err(Error::InvalidData(
                    "rotation track is dynamic while its mask is not dynamic",
                ));
            }

            encode_dynamic_rotation(dynamic_track, quantization, out)
        }
    }
}

/// Encodes a dynamic quaternion spline.
///
/// The input quaternion values are interpreted as frame samples. PyNifly
/// first makes them sign-continuous, then fits every quaternion component to
/// the same degree-1 B-spline, then makes the resulting control points
/// sign-continuous and normalizes them.
///
/// # Errors
///
/// Returns [`Error`] if the track is empty, contains invalid quaternions, if
/// fitting fails, or if the quaternion quantization format is unsupported.
pub(super) fn encode_dynamic_rotation(
    track: &SplineDynamicTrackQuat,
    quantization: QuantizationType,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    let sample_count = track.track.len();

    if sample_count == 0 {
        return Err(Error::InvalidData(
            "dynamic quaternion track contains no frame samples",
        ));
    }

    if sample_count == 1 {
        return Err(Error::InvalidData(
            "dynamic quaternion track requires at least two frame samples",
        ));
    }

    if sample_count > u16::MAX as usize + 1 {
        return Err(Error::InvalidData(
            "too many quaternion spline frame samples",
        ));
    }

    let mut samples = Vec::with_capacity(sample_count);

    for quaternion in &track.track {
        let value = quaternion.to_array();

        if value.iter().any(|component| !component.is_finite()) {
            return Err(Error::InvalidData("quaternion contains a non-finite value"));
        }

        samples.push(value);
    }

    make_quaternion_continuous(&mut samples);

    let degree = 1usize;
    let knots = make_clamped_knots(sample_count, degree)?;

    let control_points = fit_bspline_quaternion(degree, &knots, &samples)?;

    let mut control_points = control_points;

    make_quaternion_continuous(&mut control_points);

    let num_items = u16::try_from(sample_count - 1)
        .map_err(|_| Error::InvalidData("too many quaternion spline samples"))?;

    out.extend_from_slice(&num_items.to_le_bytes());
    out.push(degree as u8);

    encode_knots(&knots, out)?;

    align_rotation(out, quantization);

    for control_point in control_points {
        let quaternion = normalize_quaternion(control_point)?;

        write_quaternion(
            out,
            QuatA16::new(quaternion[0], quaternion[1], quaternion[2], quaternion[3]),
            quantization,
        )?;
    }

    Ok(())
}

/// Makes a quaternion sequence sign-continuous.
///
/// `q` and `-q` represent the same rotation. Havok spline interpolation,
/// however, operates on the stored components, so adjacent samples must use
/// the same sign hemisphere.
///
/// # Errors
///
/// This function does not fail. Non-finite input is handled by the caller.
pub(super) fn make_quaternion_continuous(quaternions: &mut [[f32; 4]]) {
    for index in 1..quaternions.len() {
        let previous = quaternions[index - 1];
        let current = quaternions[index];

        let dot = previous[0].mul_add(
            current[0],
            previous[1].mul_add(
                current[1],
                previous[2].mul_add(current[2], previous[3] * current[3]),
            ),
        );

        if dot < 0.0 {
            for component in &mut quaternions[index] {
                *component = -*component;
            }
        }
    }
}

/// Normalizes a quaternion.
///
/// # Errors
///
/// Returns [`Error`] if the quaternion has a non-finite or zero length.
pub(super) fn normalize_quaternion(value: [f32; 4]) -> Result<[f32; 4], Error> {
    let length_squared = value[0].mul_add(
        value[0],
        value[1].mul_add(value[1], value[2].mul_add(value[2], value[3] * value[3])),
    );

    if !length_squared.is_finite() || length_squared <= f32::EPSILON {
        return Err(Error::InvalidData("quaternion has an invalid length"));
    }

    let inverse_length = length_squared.sqrt().recip();

    Ok([
        value[0] * inverse_length,
        value[1] * inverse_length,
        value[2] * inverse_length,
        value[3] * inverse_length,
    ])
}

/// Writes one quaternion in the selected Havok representation.
///
/// # Errors
///
/// Returns [`Error`] if the quaternion is invalid or if the requested
/// quantization format is not supported by this encoder.
pub(super) fn write_quaternion(
    out: &mut Vec<u8>,
    quaternion: QuatA16,
    quantization: QuantizationType,
) -> Result<(), Error> {
    let quaternion = normalize_quaternion(quaternion.to_array())?;

    match quantization {
        QuantizationType::Bit40 => {
            write_quat_three_comp40(out, quaternion);
            Ok(())
        }

        QuantizationType::Bit48 => {
            write_quat_three_comp48(out, quaternion);
            Ok(())
        }

        QuantizationType::Uncompressed => {
            out.extend_from_slice(&quaternion[0].to_le_bytes());
            out.extend_from_slice(&quaternion[1].to_le_bytes());
            out.extend_from_slice(&quaternion[2].to_le_bytes());
            out.extend_from_slice(&quaternion[3].to_le_bytes());
            Ok(())
        }

        _ => Err(Error::InvalidData(
            "quaternion quantization format is not supported by the PyNifly-compatible encoder",
        )),
    }
}

/// Writes a PyNifly-compatible THREECOMP40 quaternion.
///
/// The three non-dominant components are quantized into 12-bit unsigned
/// values. The largest component is reconstructed by the decoder.
///
/// PyNifly uses `0.000345436` as the quantization fractal and `2049` as the
/// integer offset.
///
/// # Errors
///
/// This function does not fail because its caller validates the quaternion
/// and the values are clamped to the representable range.
pub(super) fn write_quat_three_comp40(out: &mut Vec<u8>, quaternion: [f32; 4]) {
    const FRACTAL: f32 = 0.000345436;

    let mut implicit_dimension = 0usize;
    let mut max_abs = quaternion[0].abs();

    for (index, &value) in quaternion.iter().enumerate().skip(1) {
        let abs = value.abs();

        if abs > max_abs {
            max_abs = abs;
            implicit_dimension = index;
        }
    }

    let implicit_negative = quaternion[implicit_dimension] < 0.0;

    let mut encoded = [0u64; 3];
    let mut encoded_index = 0;

    for (index, &value) in quaternion.iter().enumerate() {
        if index == implicit_dimension {
            continue;
        }

        let raw = (value / FRACTAL + 2049.0).round().clamp(0.0, 4095.0);

        encoded[encoded_index] = raw as u64;
        encoded_index += 1;
    }

    let packed = (encoded[0] & 0xFFF)
        | ((encoded[1] & 0xFFF) << 12)
        | ((encoded[2] & 0xFFF) << 24)
        | ((implicit_dimension as u64) << 36)
        | ((implicit_negative as u64) << 38);

    out.extend_from_slice(&packed.to_le_bytes()[..5]);
}

/// Writes a PyNifly-compatible THREECOMP48 quaternion.
///
/// The three non-dominant components are stored as signed 15-bit values.
/// Two high bits encode the omitted component index and one high bit encodes
/// the omitted component sign.
///
/// # Errors
///
/// This function does not fail because the quaternion is validated and the
/// component values are clamped to the representable range.
pub(super) fn write_quat_three_comp48(out: &mut Vec<u8>, quaternion: [f32; 4]) {
    const FRACTAL: f32 = 0.000043161;
    const MASK: u32 = (1 << 15) - 1;
    const HALF: f32 = (MASK >> 1) as f32;

    let mut implicit_dimension = 0usize;
    let mut max_abs = quaternion[0].abs();

    for (index, &value) in quaternion.iter().enumerate().skip(1) {
        let abs = value.abs();

        if abs > max_abs {
            max_abs = abs;
            implicit_dimension = index;
        }
    }

    let implicit_negative = quaternion[implicit_dimension] < 0.0;

    let mut encoded = [0u16; 3];
    let mut encoded_index = 0;

    for (index, &value) in quaternion.iter().enumerate() {
        if index == implicit_dimension {
            continue;
        }

        let raw = (value / FRACTAL + HALF).round().clamp(0.0, MASK as f32) as u16;

        encoded[encoded_index] = raw;
        encoded_index += 1;
    }

    let mut x = encoded[0] as u32;
    let mut y = encoded[1] as u32;
    let mut z = encoded[2] as u32;

    x |= ((implicit_dimension & 1) as u32) << 15;
    y |= (((implicit_dimension >> 1) & 1) as u32) << 15;

    if implicit_negative {
        z |= 1 << 15;
    }

    out.extend_from_slice(&(x as u16).to_le_bytes());
    out.extend_from_slice(&(y as u16).to_le_bytes());
    out.extend_from_slice(&(z as u16).to_le_bytes());
}
