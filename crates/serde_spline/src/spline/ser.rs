// SPDX-License-Identifier: GPL-3.0-or-later
//
// Reference:
// https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.hpp
// https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.cpp

//! Encodes Havok spline decompression structures back into binary blocks.

use havok_types::Vector4;

use super::{
    SplineDecompressor, SplineError,
    math::{
        QuantizationType, QuatA16, SplineTrackQuat, SplineTrackType, SplineTrackVector,
        TransformMask, TransformSplineBlock, TransformTrack,
    },
};

/// Encoded spline animation data.
#[derive(Clone, Debug)]
pub struct SplineEncodedData {
    /// Serialized animation data.
    pub data: Vec<u8>,

    /// Byte offset of each spline block.
    pub block_offsets: Vec<u32>,
}

impl SplineDecompressor {
    /// Encodes decoded spline blocks into Havok spline-compressed data.
    ///
    /// This does not reproduce the decompressor's original input; it produces
    /// a valid representation of the decoded values using the current masks.
    ///
    /// # Errors
    /// Returns an error when a block cannot be represented by its mask.
    pub fn encode(&self, num_float_tracks: usize) -> Result<SplineEncodedData, SplineError> {
        let mut data = Vec::new();
        let mut block_offsets = Vec::with_capacity(self.blocks.len());

        for block in &self.blocks {
            align4(&mut data);

            let offset = u32::try_from(data.len())
                .map_err(|_| SplineError::InvalidData("encoded spline data is too large"))?;

            block_offsets.push(offset);

            encode_block(block, num_float_tracks, &mut data)?;
        }

        Ok(SplineEncodedData {
            data,
            block_offsets,
        })
    }
}

/// Encodes one transform spline block.
fn encode_block(
    block: &TransformSplineBlock,
    num_float_tracks: usize,
    out: &mut Vec<u8>,
) -> Result<(), SplineError> {
    if block.masks.len() != block.tracks.len() {
        return Err(SplineError::InvalidData(
            "mask count does not match track count",
        ));
    }

    for mask in &block.masks {
        write_transform_mask(*mask, out);
    }

    // The Havok decoder skips the float-track region immediately after
    // the transform masks and then aligns to four bytes.
    out.resize(
        out.len()
            .checked_add(num_float_tracks)
            .ok_or(SplineError::InvalidData("encoded spline data is too large"))?,
        0,
    );

    align4(out);

    for (mask, track) in block.masks.iter().zip(&block.tracks) {
        encode_position(mask, track, out)?;
        align4(out);

        encode_rotation(mask, track, out)?;
        align4(out);

        encode_scale(mask, track, out)?;
    }

    Ok(())
}

/// Writes one packed transform mask.
fn write_transform_mask(mask: TransformMask, out: &mut Vec<u8>) {
    out.push(mask.quantization_types);
    out.push(mask.position_types);
    out.push(mask.rotation_types);
    out.push(mask.scale_types);
}

/// Encodes the position track.
fn encode_position(
    mask: &TransformMask,
    track: &TransformTrack,
    out: &mut Vec<u8>,
) -> Result<(), SplineError> {
    let quantization = mask.position_quantization_type()?;

    encode_vector_track(
        mask,
        track,
        &track.position,
        TransformKind::Position,
        quantization,
        0.0,
        out,
    )
}

/// Encodes the scale track.
fn encode_scale(
    mask: &TransformMask,
    track: &TransformTrack,
    out: &mut Vec<u8>,
) -> Result<(), SplineError> {
    let quantization = mask.scale_quantization_type()?;

    encode_vector_track(
        mask,
        track,
        &track.scale,
        TransformKind::Scale,
        quantization,
        1.0,
        out,
    )
}

#[derive(Clone, Copy)]
enum TransformKind {
    Position,
    Scale,
}

impl TransformKind {
    #[inline]
    const fn transform_types(self) -> [super::math::TransformType; 3] {
        use super::math::TransformType;

        match self {
            Self::Position => [
                TransformType::PosX,
                TransformType::PosY,
                TransformType::PosZ,
            ],
            Self::Scale => [
                TransformType::ScaleX,
                TransformType::ScaleY,
                TransformType::ScaleZ,
            ],
        }
    }
}

/// Encodes a position or scale vector track.
fn encode_vector_track(
    mask: &TransformMask,
    _track: &TransformTrack,
    source: &SplineTrackVector,
    kind: TransformKind,
    quantization: QuantizationType,
    default: f32,
    out: &mut Vec<u8>,
) -> Result<(), SplineError> {
    let types = kind.transform_types();

    let dynamic = types
        .iter()
        .any(|ty| mask.sub_track_type(*ty) == SplineTrackType::Dynamic);

    match source {
        SplineTrackVector::Static(track) => {
            if dynamic {
                return Err(SplineError::InvalidData(
                    "vector track is static while its mask is dynamic",
                ));
            }

            encode_static_vector(
                mask,
                &types,
                vector4_to_f32_array(&track.value),
                default,
                out,
            )
        }

        SplineTrackVector::Dynamic(track) => {
            if !dynamic {
                return Err(SplineError::InvalidData(
                    "vector track is dynamic while its mask is not dynamic",
                ));
            }

            encode_dynamic_vector(mask, &types, track, quantization, default, out)
        }
    }
}

#[inline]
const fn vector4_to_f32_array(value: &Vector4) -> [f32; 3] {
    [value.x, value.y, value.z]
}

/// Encodes a static vector track.
fn encode_static_vector(
    mask: &TransformMask,
    types: &[super::math::TransformType; 3],
    value: [f32; 3],
    default: f32,
    out: &mut Vec<u8>,
) -> Result<(), SplineError> {
    for axis in 0..3 {
        #[expect(
            clippy::float_cmp,
            reason = "Identity tracks must contain the exact default value."
        )]
        if mask.sub_track_type(types[axis]) == SplineTrackType::Static {
            let value = value[axis];

            if !value.is_finite() {
                return Err(SplineError::InvalidData(
                    "static vector contains a non-finite value",
                ));
            }

            out.extend_from_slice(&value.to_le_bytes());
        } else if mask.sub_track_type(types[axis]) == SplineTrackType::Identity
            && value[axis] != default
        {
            return Err(SplineError::InvalidData(
                "identity vector component differs from its default value",
            ));
        }
    }

    Ok(())
}

fn encode_knots(knots: &[f32], out: &mut Vec<u8>) -> Result<(), SplineError> {
    for &knot in knots {
        if !knot.is_finite() || knot < 0.0 || knot > u8::MAX as f32 || knot.fract() != 0.0 {
            return Err(SplineError::InvalidData(
                "spline knot cannot be represented as u8",
            ));
        }

        out.push(knot as u8);
    }

    Ok(())
}

/// Encodes a dynamic vector track.
fn encode_dynamic_vector(
    mask: &TransformMask,
    types: &[super::math::TransformType; 3],
    track: &super::math::SplineDynamicTrackVector,
    quantization: QuantizationType,
    default: f32,
    out: &mut Vec<u8>,
) -> Result<(), SplineError> {
    let control_point_count = track
        .tracks
        .iter()
        .filter(|values| values.len() > 1)
        .map(Vec::len)
        .max()
        .unwrap_or(0);

    if control_point_count == 0 {
        return Err(SplineError::InvalidData(
            "dynamic vector track contains no control points",
        ));
    }

    let num_items = control_point_count
        .checked_sub(1)
        .ok_or(SplineError::InvalidData(
            "invalid vector control point count",
        ))?;

    let num_items_u16 = u16::try_from(num_items)
        .map_err(|_| SplineError::InvalidData("too many vector spline control points"))?;

    let degree = track.degree;

    out.extend_from_slice(&num_items_u16.to_le_bytes());

    // Reserved byte used by Havok's spline block format.
    out.push(0);

    out.push(degree);

    let expected_knots = num_items
        .checked_add(degree as usize)
        .and_then(|value| value.checked_add(2))
        .ok_or(SplineError::InvalidData("invalid vector knot count"))?;

    if track.knots.len() != expected_knots {
        return Err(SplineError::InvalidData(
            "vector knot count does not match num_items and degree",
        ));
    }

    encode_knots(&track.knots, out)?;
    align4(out);

    let mut bounds = [[0.0f32; 2]; 3];

    for axis in 0..3 {
        match mask.sub_track_type(types[axis]) {
            SplineTrackType::Dynamic => {
                let values = &track.tracks[axis];

                if values.len() != control_point_count {
                    return Err(SplineError::InvalidData(
                        "dynamic vector axes have different control point counts",
                    ));
                }

                let min = values
                    .iter()
                    .copied()
                    .reduce(f32::min)
                    .ok_or(SplineError::InvalidData("dynamic vector axis is empty"))?;

                let max = values
                    .iter()
                    .copied()
                    .reduce(f32::max)
                    .ok_or(SplineError::InvalidData("dynamic vector axis is empty"))?;

                if !min.is_finite() || !max.is_finite() {
                    return Err(SplineError::InvalidData(
                        "dynamic vector contains a non-finite value",
                    ));
                }

                bounds[axis] = [min, max];

                out.extend_from_slice(&min.to_le_bytes());
                out.extend_from_slice(&max.to_le_bytes());
            }

            SplineTrackType::Static => {
                let values = &track.tracks[axis];

                if values.len() != 1 {
                    return Err(SplineError::InvalidData(
                        "static vector axis must contain exactly one value",
                    ));
                }

                let value = values[0];

                if !value.is_finite() {
                    return Err(SplineError::InvalidData(
                        "static vector contains a non-finite value",
                    ));
                }

                out.extend_from_slice(&value.to_le_bytes());
            }

            SplineTrackType::Identity => {
                let values = &track.tracks[axis];
                #[expect(
                    clippy::float_cmp,
                    reason = "Identity tracks must contain the exact default value."
                )]
                if !values.is_empty() && values.iter().any(|value| *value != default) {
                    return Err(SplineError::InvalidData(
                        "identity vector component has a non-default value",
                    ));
                }
            }
        }
    }

    for item in 0..control_point_count {
        for axis in 0..3 {
            if mask.sub_track_type(types[axis]) != SplineTrackType::Dynamic {
                continue;
            }

            let value = track.tracks[axis]
                .get(item)
                .copied()
                .ok_or(SplineError::InvalidData(
                    "vector control point index is out of bounds",
                ))?;

            let min = bounds[axis][0];
            let max = bounds[axis][1];

            write_quantized_scalar(out, value, min, max, quantization)?;
        }
    }

    align4(out);

    Ok(())
}

/// Writes one quantized scalar.
///
/// Havok's 16-bit vector representation advances by four bytes per scalar:
/// two bytes contain the value and two bytes are padding.
fn write_quantized_scalar(
    out: &mut Vec<u8>,
    value: f32,
    min: f32,
    max: f32,
    quantization: QuantizationType,
) -> Result<(), SplineError> {
    if !value.is_finite() || !min.is_finite() || !max.is_finite() {
        return Err(SplineError::InvalidData(
            "cannot quantize a non-finite scalar",
        ));
    }

    let range = max - min;

    let normalized = if range.abs() <= f32::EPSILON {
        0.0
    } else {
        ((value - min) / range).clamp(0.0, 1.0)
    };

    match quantization {
        QuantizationType::Bit8 => {
            let encoded = (normalized * 255.0).round() as u8;
            out.push(encoded);
        }

        QuantizationType::Bit16 => {
            let encoded = (normalized * 65535.0).round() as u16;

            out.extend_from_slice(&encoded.to_le_bytes());

            // The decoder skips two bytes after every 16-bit scalar.
            out.extend_from_slice(&[0, 0]);
        }

        _ => {
            return Err(SplineError::InvalidData("invalid scalar quantization type"));
        }
    }

    Ok(())
}

/// Encodes the rotation track.
fn encode_rotation(
    mask: &TransformMask,
    track: &TransformTrack,
    out: &mut Vec<u8>,
) -> Result<(), SplineError> {
    let quantization = mask.rotation_quantization_type()?;

    match &track.rotation {
        SplineTrackQuat::Identity => {
            if mask.sub_track_type(super::math::TransformType::Rotation)
                != SplineTrackType::Identity
            {
                return Err(SplineError::InvalidData(
                    "rotation track is identity while its mask is not identity",
                ));
            }

            Ok(())
        }

        SplineTrackQuat::Static(static_track) => {
            if mask.sub_track_type(super::math::TransformType::Rotation) != SplineTrackType::Static
            {
                return Err(SplineError::InvalidData(
                    "rotation track is static while its mask is not static",
                ));
            }

            align_rotation(out, quantization)?;
            write_quaternion(out, static_track.value, quantization)
        }

        SplineTrackQuat::Dynamic(dynamic_track) => {
            if mask.sub_track_type(super::math::TransformType::Rotation) != SplineTrackType::Dynamic
            {
                return Err(SplineError::InvalidData(
                    "rotation track is dynamic while its mask is not dynamic",
                ));
            }

            encode_dynamic_rotation(dynamic_track, quantization, out)
        }
    }
}

/// Encodes a dynamic quaternion spline.
fn encode_dynamic_rotation(
    track: &super::math::SplineDynamicTrackQuat,
    quantization: QuantizationType,
    out: &mut Vec<u8>,
) -> Result<(), SplineError> {
    if track.track.is_empty() {
        return Err(SplineError::InvalidData(
            "dynamic quaternion track contains no control points",
        ));
    }

    let num_items = track.track.len() - 1;
    let num_items_u16 = u16::try_from(num_items)
        .map_err(|_| SplineError::InvalidData("too many quaternion spline control points"))?;

    let expected_knots = num_items
        .checked_add(track.degree as usize)
        .and_then(|value| value.checked_add(2))
        .ok_or(SplineError::InvalidData("invalid quaternion knot count"))?;

    if track.knots.len() != expected_knots {
        return Err(SplineError::InvalidData(
            "quaternion knot count does not match num_items and degree",
        ));
    }

    out.extend_from_slice(&num_items_u16.to_le_bytes());

    // Reserved byte.
    out.push(0);

    out.push(track.degree);

    encode_knots(&track.knots, out)?;

    align_rotation(out, quantization)?;

    for quaternion in &track.track {
        write_quaternion(out, *quaternion, quantization)?;
    }

    Ok(())
}

/// Aligns the rotation payload exactly as the Havok decoder does.
fn align_rotation(out: &mut Vec<u8>, quantization: QuantizationType) -> Result<(), SplineError> {
    let alignment = match quantization {
        QuantizationType::Bit48 | QuantizationType::Bit16Quat => 2,
        QuantizationType::Bit32 | QuantizationType::Uncompressed => 4,
        // QuantizationType::Bit40 => 1,
        // QuantizationType::Bit24 => 1,
        _ => 1,
    };

    align_to(out, alignment);

    Ok(())
}

/// Writes one quaternion in the selected Havok representation.
fn write_quaternion(
    out: &mut Vec<u8>,
    quaternion: QuatA16,
    quantization: QuantizationType,
) -> Result<(), SplineError> {
    let [x, y, z, w] = quaternion.to_array();

    if !x.is_finite() || !y.is_finite() || !z.is_finite() || !w.is_finite() {
        return Err(SplineError::InvalidData(
            "quaternion contains a non-finite value",
        ));
    }

    match quantization {
        QuantizationType::Bit32 => write_quat_polar32(out, quaternion),

        QuantizationType::Bit40 => write_quat_three_comp40(out, quaternion),

        QuantizationType::Bit48 => write_quat_three_comp48(out, quaternion),

        QuantizationType::Uncompressed => {
            out.extend_from_slice(&x.to_le_bytes());
            out.extend_from_slice(&y.to_le_bytes());
            out.extend_from_slice(&z.to_le_bytes());
            out.extend_from_slice(&w.to_le_bytes());
            Ok(())
        }

        QuantizationType::Bit24 | QuantizationType::Bit16Quat => Err(SplineError::InvalidData(
            "quaternion encoding is not implemented for this quantization type",
        )),

        QuantizationType::Bit8 | QuantizationType::Bit16 => Err(SplineError::InvalidData(
            "scalar quantization type cannot encode a quaternion",
        )),
    }
}

/// Encodes Havok THREECOMP40 quaternion data.
fn write_quat_three_comp40(out: &mut Vec<u8>, quaternion: QuatA16) -> Result<(), SplineError> {
    const MASK: u64 = (1 << 11) - 1;
    const BIAS: i32 = 1023;
    const FRACTAL: f32 = 0.000_345_436;

    let [x, y, z, w] = quaternion.normalize().to_array();
    let components = [x, y, z, w];

    let (result_shift, result_sign) = largest_component(&components);

    let sign = if result_sign { -1.0 } else { 1.0 };

    let mut encoded = [0u64; 3];
    let mut index = 0;

    for component in components {
        if index == result_shift {
            continue;
        }

        let value = (component * sign / FRACTAL).round() as i32 + BIAS;

        if !(0..=MASK as i32).contains(&value) {
            return Err(SplineError::InvalidData(
                "quaternion component is outside THREECOMP40 range",
            ));
        }

        encoded[index] = value as u64;
        index += 1;
    }

    let packed = encoded[0]
        | (encoded[1] << 11)
        | (encoded[2] << 22)
        | ((result_shift as u64) << 33)
        | ((result_sign as u64) << 35);

    let bytes = packed.to_le_bytes();

    out.extend_from_slice(&bytes[..5]);

    Ok(())
}

/// Encodes Havok THREECOMP48 quaternion data.
fn write_quat_three_comp48(out: &mut Vec<u8>, quaternion: QuatA16) -> Result<(), SplineError> {
    const MASK: i32 = (1 << 15) - 1;
    const BIAS: i32 = MASK >> 1;
    const FRACTAL: f32 = 0.000_043_161;

    let [x, y, z, w] = quaternion.normalize().to_array();
    let components = [x, y, z, w];

    let (result_shift, result_sign) = largest_component(&components);

    let sign = if result_sign { -1.0 } else { 1.0 };

    let mut values = [0i16; 3];
    let mut index = 0;

    for component in components {
        if index == result_shift {
            continue;
        }

        let value = (component * sign / FRACTAL).round() as i32 + BIAS;

        if !(0..=MASK).contains(&value) {
            return Err(SplineError::InvalidData(
                "quaternion component is outside THREECOMP48 range",
            ));
        }

        values[index] = value as i16;
        index += 1;
    }

    let mut x = values[0] as u16;
    let mut y = values[1] as u16;
    let z = values[2] as u16;

    x |= ((result_shift & 1) as u16) << 15;
    y |= (((result_shift >> 1) & 1) as u16) << 14;

    let z = if result_sign { z | 0x8000 } else { z };

    out.extend_from_slice(&x.to_le_bytes());
    out.extend_from_slice(&y.to_le_bytes());
    out.extend_from_slice(&z.to_le_bytes());

    Ok(())
}

/// Encodes Havok POLAR32 quaternion data.
///
/// The representation follows the decoder's inverse mapping.
fn write_quat_polar32(out: &mut Vec<u8>, quaternion: QuatA16) -> Result<(), SplineError> {
    use core::f32::consts::{FRAC_PI_2, FRAC_PI_4};

    const R_MASK: u32 = (1 << 10) - 1;

    let [x, y, z, w] = quaternion.normalize().to_array();

    let sign_x = x < 0.0;
    let sign_y = y < 0.0;
    let sign_z = z < 0.0;
    let sign_w = w < 0.0;

    let ax = x.abs();
    let ay = y.abs();
    let az = z.abs();
    let aw = w.abs();

    let radial = (ax.mul_add(ax, ay.mul_add(ay, az * az))).sqrt();

    let phi = radial.atan2(aw);

    let phi_index = (phi * 511.0 / FRAC_PI_2).round().clamp(0.0, 511.0);

    let phi = phi_index;

    let theta = if radial > f32::EPSILON {
        ay.atan2(ax)
    } else {
        0.0
    };

    let phi_theta = if phi > 0.0 {
        (theta * phi / FRAC_PI_4 + phi * phi)
            .round()
            .clamp(0.0, 262143.0)
    } else {
        0.0
    };

    let r = aw.mul_add(-aw, 1.0).max(0.0).sqrt();

    let r_quantized = (r * R_MASK as f32).round().clamp(0.0, R_MASK as f32) as u32;

    let packed = phi_theta as u32
        | (r_quantized << 18)
        | if sign_x { 0x1000_0000 } else { 0 }
        | if sign_y { 0x2000_0000 } else { 0 }
        | if sign_z { 0x4000_0000 } else { 0 }
        | if sign_w { 0x8000_0000 } else { 0 };

    out.extend_from_slice(&packed.to_le_bytes());

    Ok(())
}

/// Returns the omitted quaternion component and whether it was negative.
fn largest_component(components: &[f32; 4]) -> (usize, bool) {
    let mut index = 0;
    let mut magnitude = components[0].abs();

    for (candidate, component) in components.iter().enumerate().skip(1) {
        let candidate_magnitude = component.abs();

        if candidate_magnitude > magnitude {
            index = candidate;
            magnitude = candidate_magnitude;
        }
    }

    (index, components[index] < 0.0)
}

#[inline]
fn align4(out: &mut Vec<u8>) {
    align_to(out, 4);
}

#[inline]
fn align_to(out: &mut Vec<u8>, alignment: usize) {
    if alignment <= 1 {
        return;
    }

    let remainder = out.len() % alignment;

    if remainder != 0 {
        out.resize(out.len() + (alignment - remainder), 0);
    }
}
