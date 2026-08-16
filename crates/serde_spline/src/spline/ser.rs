// SPDX-License-Identifier: GPL-3.0-or-later
//
// Reference:
// https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.hpp
// https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.cpp
// https://github.com/Grimrukh/soulstruct-havok/blob/e67d5b9642f321dc3060abc91017c352245c7f3d/src/soulstruct/havok/spline_compression.py

//! Encodes Havok spline-compressed animation blocks.
//!
//! The layout intentionally follows Soulstruct's
//! `SplineCompressedAnimationData.pack()` implementation.
//!
//! A transform block is encoded as:
//!
//! ```text
//! TransformMask[track_count]
//! align(4)
//!
//! for each transform track:
//!     translation
//!     align(4)
//!     rotation
//!     align(4)
//!     scale
//!
//! align(16)
//! ```
//!
//! Vector spline data is encoded as:
//!
//! ```text
//! u16(control_point_count - 1)
//! u8(degree)
//! u8[knot_count]
//! align(4)
//! axis metadata
//! quantized control points
//! ```
//!
//! Rotation spline data uses the same spline header, followed by
//! quantization-specific alignment and quaternion control points.

use havok_types::Vector4;

use super::{
    SplineDecompressor,
    math::{
        QuantizationType, QuatA16, SplineDynamicTrackQuat, SplineDynamicTrackVector,
        SplineTrackQuat, SplineTrackType, SplineTrackVector, TransformMask, TransformSplineBlock,
        TransformTrack, TransformType,
    },
};
use crate::error::Error;

/// Encoded spline animation data.
#[derive(Clone, Debug)]
pub struct SplineEncodedData {
    /// Serialized spline-compressed animation data.
    pub data: Vec<u8>,

    /// Byte offset of every encoded spline block.
    pub block_offsets: Vec<u32>,
}

impl SplineDecompressor {
    /// Encodes all spline blocks into Havok spline-compressed animation data.
    ///
    /// The encoding layout follows Soulstruct's
    /// `SplineCompressedAnimationData.pack()` implementation.
    ///
    /// `num_float_tracks` is intentionally not accepted here. This encoder
    /// handles transform tracks only, and the caller is responsible for
    /// constructing the corresponding Havok animation object.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if a block has inconsistent mask and track counts,
    /// contains invalid spline metadata, contains non-finite values, or uses
    /// an unsupported quaternion quantization format.
    pub fn encode(&self) -> Result<SplineEncodedData, Error> {
        if self.blocks.is_empty() {
            return Err(Error::InvalidData("cannot encode empty spline data"));
        }

        let transform_track_count = self.blocks[0].tracks.len();

        for block in &self.blocks {
            if block.masks.len() != transform_track_count
                || block.tracks.len() != transform_track_count
            {
                return Err(Error::InvalidData(
                    "animation data blocks do not have equal transform track counts",
                ));
            }
        }

        let mut data = Vec::new();
        let mut block_offsets = Vec::with_capacity(self.blocks.len());

        for block in &self.blocks {
            align4(&mut data);

            let offset = u32::try_from(data.len())
                .map_err(|_| Error::InvalidData("encoded spline data is too large"))?;

            block_offsets.push(offset);

            encode_block(block, &mut data)?;
        }

        Ok(SplineEncodedData {
            data,
            block_offsets,
        })
    }
}

/// Encodes one spline-compressed animation block.
///
/// The order exactly follows Soulstruct's block packing order:
///
/// 1. Transform headers.
/// 2. Four-byte alignment.
/// 3. Translation, rotation, and scale for every transform track.
/// 4. Sixteen-byte block alignment.
fn encode_block(block: &TransformSplineBlock, out: &mut Vec<u8>) -> Result<(), Error> {
    if block.masks.len() != block.tracks.len() {
        return Err(Error::InvalidData("mask count does not match track count"));
    }

    for mask in &block.masks {
        write_transform_mask(*mask, out);
    }

    align4(out);

    for (mask, track) in block.masks.iter().zip(&block.tracks) {
        encode_position(mask, track, out)?;
        align4(out);

        encode_rotation(mask, track, out)?;
        align4(out);

        encode_scale(mask, track, out)?;
    }

    align16(out);

    Ok(())
}

/// Writes one four-byte Havok transform mask.
#[inline]
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
) -> Result<(), Error> {
    let quantization = mask.position_quantization_type()?;

    encode_vector_track(
        mask,
        &track.position,
        [
            TransformType::PosX,
            TransformType::PosY,
            TransformType::PosZ,
        ],
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
) -> Result<(), Error> {
    let quantization = mask.scale_quantization_type()?;

    encode_vector_track(
        mask,
        &track.scale,
        [
            TransformType::ScaleX,
            TransformType::ScaleY,
            TransformType::ScaleZ,
        ],
        quantization,
        1.0,
        out,
    )
}

/// Encodes one position or scale vector.
fn encode_vector_track(
    mask: &TransformMask,
    source: &SplineTrackVector,
    types: [TransformType; 3],
    quantization: QuantizationType,
    default: f32,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    let dynamic = types
        .iter()
        .any(|&ty| mask.sub_track_type(ty) == SplineTrackType::Dynamic);

    match source {
        SplineTrackVector::Static(track) => {
            if dynamic {
                return Err(Error::InvalidData(
                    "vector track is static while its mask contains a dynamic axis",
                ));
            }

            encode_static_vector(
                mask,
                types,
                vector4_to_f32_array(&track.value),
                default,
                out,
            )
        }

        SplineTrackVector::Dynamic(track) => {
            if !dynamic {
                return Err(Error::InvalidData(
                    "vector track is dynamic while its mask contains no dynamic axis",
                ));
            }

            encode_dynamic_vector(mask, types, track, quantization, default, out)
        }
    }
}

/// Converts the three relevant components of a Havok vector.
#[inline]
const fn vector4_to_f32_array(value: &Vector4) -> [f32; 3] {
    [value.x, value.y, value.z]
}

/// Encodes a vector which contains only static and identity axes.
fn encode_static_vector(
    mask: &TransformMask,
    types: [TransformType; 3],
    values: [f32; 3],
    default: f32,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    for axis in 0..3 {
        match mask.sub_track_type(types[axis]) {
            SplineTrackType::Static => {
                let value = values[axis];

                if !value.is_finite() {
                    return Err(Error::InvalidData(
                        "static vector contains a non-finite value",
                    ));
                }

                out.extend_from_slice(&value.to_le_bytes());
            }

            SplineTrackType::Identity =>
            {
                #[expect(
                    clippy::float_cmp,
                    reason = "Identity tracks must contain the exact Havok default value."
                )]
                if values[axis] != default {
                    return Err(Error::InvalidData(
                        "identity vector component differs from its default value",
                    ));
                }
            }

            SplineTrackType::Dynamic => {
                return Err(Error::InvalidData(
                    "static vector contains a dynamic mask component",
                ));
            }
        }
    }

    Ok(())
}

/// Encodes a dynamic vector spline.
///
/// The three axes share one spline header. Only axes whose mask says Dynamic
/// have bounds and quantized control points. Static axes contain one f32 and
/// identity axes contain no payload.
fn encode_dynamic_vector(
    mask: &TransformMask,
    types: [TransformType; 3],
    track: &SplineDynamicTrackVector,
    quantization: QuantizationType,
    default: f32,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    let control_point_count = types
        .iter()
        .enumerate()
        .find_map(|(axis, &ty)| {
            (mask.sub_track_type(ty) == SplineTrackType::Dynamic)
                .then_some(track.tracks[axis].len())
        })
        .ok_or(Error::InvalidData(
            "dynamic vector contains no dynamic axis",
        ))?;

    if control_point_count == 0 {
        return Err(Error::InvalidData(
            "dynamic vector contains no control points",
        ));
    }

    #[expect(clippy::needless_range_loop)]
    for axis in 0..3 {
        if mask.sub_track_type(types[axis]) == SplineTrackType::Dynamic
            && track.tracks[axis].len() != control_point_count
        {
            return Err(Error::InvalidData(
                "dynamic vector axes have different control point counts",
            ));
        }
    }

    let num_items = control_point_count
        .checked_sub(1)
        .ok_or(Error::InvalidData("invalid vector control point count"))?;

    let num_items = u16::try_from(num_items)
        .map_err(|_| Error::InvalidData("too many vector spline control points"))?;

    let degree = track.degree;

    if degree == 0 {
        return Err(Error::InvalidData("spline degree must not be zero"));
    }

    let expected_knot_count = control_point_count
        .checked_add(degree as usize)
        .and_then(|count| count.checked_add(1))
        .ok_or(Error::InvalidData("invalid vector knot count"))?;

    if track.knots.len() != expected_knot_count {
        return Err(Error::InvalidData(
            "vector knot count does not match control point count and degree",
        ));
    }

    out.extend_from_slice(&num_items.to_le_bytes());
    out.push(degree);

    encode_knots(&track.knots, out)?;

    align4(out);

    let mut bounds = [[0.0f32; 2]; 3];

    for axis in 0..3 {
        match mask.sub_track_type(types[axis]) {
            SplineTrackType::Dynamic => {
                let values = &track.tracks[axis];

                let min = values
                    .iter()
                    .copied()
                    .reduce(f32::min)
                    .ok_or(Error::InvalidData("dynamic vector axis is empty"))?;

                let max = values
                    .iter()
                    .copied()
                    .reduce(f32::max)
                    .ok_or(Error::InvalidData("dynamic vector axis is empty"))?;

                if !min.is_finite() || !max.is_finite() {
                    return Err(Error::InvalidData(
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
                    return Err(Error::InvalidData(
                        "static vector axis must contain exactly one value",
                    ));
                }

                let value = values[0];

                if !value.is_finite() {
                    return Err(Error::InvalidData(
                        "static vector contains a non-finite value",
                    ));
                }

                out.extend_from_slice(&value.to_le_bytes());
            }

            SplineTrackType::Identity => {
                let values = &track.tracks[axis];

                #[expect(
                    clippy::float_cmp,
                    reason = "Identity tracks must contain the exact Havok default value."
                )]
                if values.iter().any(|&value| value != default) {
                    return Err(Error::InvalidData(
                        "identity vector component differs from its default value",
                    ));
                }
            }
        }
    }

    for control_point in 0..control_point_count {
        for axis in 0..3 {
            if mask.sub_track_type(types[axis]) != SplineTrackType::Dynamic {
                continue;
            }

            let value =
                track.tracks[axis]
                    .get(control_point)
                    .copied()
                    .ok_or(Error::InvalidData(
                        "vector control point index is out of bounds",
                    ))?;

            let [min, max] = bounds[axis];

            write_quantized_scalar(out, value, min, max, quantization)?;
        }
    }

    Ok(())
}

/// Writes one scalar quantized according to Havok's scalar track format.
///
/// `Bit8` stores one byte and `Bit16` stores two bytes. There is no per-value
/// four-byte padding; the surrounding vector payload is aligned after all
/// control points have been written.
fn write_quantized_scalar(
    out: &mut Vec<u8>,
    value: f32,
    min: f32,
    max: f32,
    quantization: QuantizationType,
) -> Result<(), Error> {
    if !value.is_finite() || !min.is_finite() || !max.is_finite() {
        return Err(Error::InvalidData("cannot quantize a non-finite scalar"));
    }

    let range = max - min;

    let normalized = if range == 0.0 {
        #[expect(clippy::float_cmp)]
        if value != min {
            return Err(Error::InvalidData(
                "quantization bounds are equal but the value differs",
            ));
        }

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
        }

        _ => {
            return Err(Error::InvalidData("invalid scalar quantization type"));
        }
    }

    Ok(())
}

/// Encodes the rotation track.
///
/// Soulstruct's encoder only creates new quaternion data for
/// `ThreeComp40`. Other quaternion quantization formats require the original
/// raw bytes for a byte-preserving re-pack. The current Rust representation
/// stores decoded quaternions only, so those formats are rejected.
fn encode_rotation(
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

            if quantization != QuantizationType::Bit40 {
                return Err(Error::InvalidData(
                    "static quaternion requires raw data for unsupported quantization",
                ));
            }

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
fn encode_dynamic_rotation(
    track: &SplineDynamicTrackQuat,
    quantization: QuantizationType,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    if track.track.is_empty() {
        return Err(Error::InvalidData(
            "dynamic quaternion track contains no control points",
        ));
    }

    if quantization != QuantizationType::Bit40 {
        return Err(Error::InvalidData(
            "dynamic quaternion requires raw data for unsupported quantization",
        ));
    }

    let control_point_count = track.track.len();

    let num_items = control_point_count
        .checked_sub(1)
        .ok_or(Error::InvalidData("invalid quaternion control point count"))?;

    let num_items = u16::try_from(num_items)
        .map_err(|_| Error::InvalidData("too many quaternion spline control points"))?;

    let degree = track.degree;

    if degree == 0 {
        return Err(Error::InvalidData("spline degree must not be zero"));
    }

    let expected_knot_count = control_point_count
        .checked_add(degree as usize)
        .and_then(|count| count.checked_add(1))
        .ok_or(Error::InvalidData("invalid quaternion knot count"))?;

    if track.knots.len() != expected_knot_count {
        return Err(Error::InvalidData(
            "quaternion knot count does not match control point count and degree",
        ));
    }

    out.extend_from_slice(&num_items.to_le_bytes());
    out.push(degree);

    encode_knots(&track.knots, out)?;

    align_rotation(out, quantization);

    for quaternion in &track.track {
        write_quaternion(out, *quaternion, quantization)?;
    }

    Ok(())
}

/// Encodes a spline knot vector.
///
/// Havok stores spline knots as unsigned bytes.
fn encode_knots(knots: &[f32], out: &mut Vec<u8>) -> Result<(), Error> {
    for &knot in knots {
        if !knot.is_finite() || knot < 0.0 || knot > u8::MAX as f32 || knot.fract() != 0.0 {
            return Err(Error::InvalidData(
                "spline knot cannot be represented as u8",
            ));
        }

        out.push(knot as u8);
    }

    Ok(())
}

/// Aligns a quaternion payload according to its quantization format.
///
/// These values match Soulstruct's `get_rotation_align()`:
///
/// - Polar32: 4
/// - ThreeComp40: 1
/// - ThreeComp48: 2
/// - ThreeComp25: 1
/// - Straight16: 2
/// - Uncompressed: 4
fn align_rotation(out: &mut Vec<u8>, quantization: QuantizationType) {
    let alignment = match quantization {
        QuantizationType::Bit8
        | QuantizationType::Bit16
        | QuantizationType::Bit24
        | QuantizationType::Bit40 => 1,

        QuantizationType::Bit48 | QuantizationType::Bit16Quat => 2,
        QuantizationType::Bit32 | QuantizationType::Uncompressed => 4,
    };

    align_to(out, alignment);
}

/// Writes one quaternion using the supported Havok encoding.
///
/// New quaternion encoding intentionally follows Soulstruct and only emits
/// THREECOMP40. Other formats require raw source bytes to reproduce the
/// original representation.
fn write_quaternion(
    out: &mut Vec<u8>,
    quaternion: QuatA16,
    quantization: QuantizationType,
) -> Result<(), Error> {
    match quantization {
        QuantizationType::Bit40 => {
            write_quat_three_comp40(out, quaternion)?;
            Ok(())
        }
        _ => Err(Error::InvalidData(
            "quaternion quantization requires unsupported raw encoding",
        )),
    }
}

/// Writes a Havok THREECOMP40 quaternion.
///
/// The format stores three 12-bit values, a two-bit omitted-component index,
/// and one sign bit in a 40-bit integer.
///
/// The constants match the corresponding decoded representation in this
/// crate and Soulstruct's THREECOMP40 format.
fn write_quat_three_comp40(out: &mut Vec<u8>, quaternion: QuatA16) -> Result<(), Error> {
    const MASK: u64 = (1 << 12) - 1;
    const START: f32 = -core::f32::consts::FRAC_1_SQRT_2;
    const STEP: f32 = core::f32::consts::SQRT_2 / 4094.0;

    let quat = quaternion.to_array();

    if quat.iter().any(|value| !value.is_finite()) {
        return Err(Error::InvalidData("quaternion contains a non-finite value"));
    }

    let mut implicit_dimension = 0usize;
    let mut max_abs = quat[0].abs();

    for (index, &value) in quat.iter().enumerate().skip(1) {
        let abs = value.abs();

        if abs > max_abs {
            max_abs = abs;
            implicit_dimension = index;
        }
    }

    let implicit_negative = quat[implicit_dimension] < 0.0;

    let mut encoded = [0u64; 3];
    let mut encoded_index = 0;

    for (index, &value) in quat.iter().enumerate() {
        if index == implicit_dimension {
            continue;
        }

        let value = ((value - START) / STEP).round();

        if !(0.0..=4095.0).contains(&value) {
            return Err(Error::InvalidData(
                "quaternion component cannot be represented by THREECOMP40",
            ));
        }

        encoded[encoded_index] = value as u64;
        encoded_index += 1;
    }

    let packed = (encoded[0] & MASK)
        | ((encoded[1] & MASK) << 12)
        | ((encoded[2] & MASK) << 24)
        | ((implicit_dimension as u64) << 36)
        | ((implicit_negative as u64) << 38);

    let bytes = packed.to_le_bytes();

    out.extend_from_slice(&bytes[..5]);

    Ok(())
}

#[inline]
fn align4(out: &mut Vec<u8>) {
    align_to(out, 4);
}

#[inline]
fn align16(out: &mut Vec<u8>) {
    align_to(out, 16);
}

#[inline]
fn align_to(out: &mut Vec<u8>, alignment: usize) {
    if alignment <= 1 {
        return;
    }

    let remainder = out.len() % alignment;

    if remainder != 0 {
        out.resize(out.len() + alignment - remainder, 0);
    }
}
