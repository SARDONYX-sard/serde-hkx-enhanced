// SPDX-License-Identifier: GPL-3.0-or-later
//
// Reference:
// - https://github.com/BadDogSkyrim/PyNifly/blob/7fd4644f5a6416c1502983b7d49a853eb0d24509/io_scene_nifly/hkx/anim_fo4.py
// - https://github.com/BadDogSkyrim/PyNifly/blob/7fd4644f5a6416c1502983b7d49a853eb0d24509/io_scene_nifly/hkx/anim_skyrim.py
//
// Additional format references:
// - https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.hpp
// - https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.cpp

//! Encodes Havok spline-compressed animation blocks.
//!
//! The compressor follows the export implementation used by PyNifly:
//!
//! - dynamic scalar tracks are fitted to degree-1 B-splines;
//! - quaternion samples are made sign-continuous before fitting;
//! - quaternion control points are made sign-continuous and normalized;
//! - scalar tracks support 8-bit and 16-bit quantization;
//! - quaternion tracks support THREECOMP40 and THREECOMP48;
//! - block, track, spline, and quaternion alignment follows Havok's format.
//!
//! The [`SplineDynamicTrackVector`] and [`SplineDynamicTrackQuat`] values are
//! treated as sampled frame data when encoding. Their stored knot vectors are
//! not blindly reused as already-fitted control-point data.

#[cfg(feature = "tracing")]
mod debug;

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
    /// Dynamic tracks are interpreted as sampled frame values and are fitted
    /// to the B-spline representation expected by Havok.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the animation contains no blocks, if block track
    /// counts differ, if a track contains invalid spline data, if a value is
    /// non-finite, or if a selected quantization format cannot be encoded.
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

        for (block_index, block) in self.blocks.iter().enumerate() {
            align4(&mut data);

            let offset = u32::try_from(data.len())
                .map_err(|_| Error::InvalidData("encoded spline data is too large"))?;

            block_offsets.push(offset);

            encode_block(block, block_index, &mut data)?;
        }

        Ok(SplineEncodedData {
            data,
            block_offsets,
        })
    }
}

/// Encodes one Havok spline block.
///
/// The block layout is:
///
/// ```text
/// TransformMask[track_count]
/// align(4)
///
/// for each track:
///     position
///     align(4)
///     rotation
///     align(4)
///     scale
///
/// align(16)
/// ```
///
/// # Errors
///
/// Returns [`Error`] if any track is inconsistent with its transform mask or
/// if one of its spline payloads cannot be encoded.
fn encode_block(
    block: &TransformSplineBlock,
    block_index: usize,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    #[cfg(not(feature = "tracing"))]
    let _ = block_index;

    if block.masks.len() != block.tracks.len() {
        return Err(Error::InvalidData("mask count does not match track count"));
    }

    #[cfg(feature = "tracing")]
    let block_start = out.len();

    for mask in &block.masks {
        write_transform_mask(*mask, out);
    }

    align4(out);

    #[cfg(feature = "tracing")]
    let mut debug_tracks = Vec::with_capacity(block.tracks.len());

    for (mask, track) in block.masks.iter().zip(&block.tracks) {
        #[cfg(feature = "tracing")]
        let position_offset = out.len();

        encode_position(mask, track, out)?;

        #[cfg(feature = "tracing")]
        let position_size = out.len() - position_offset;

        align4(out);

        #[cfg(feature = "tracing")]
        let rotation_offset = out.len();

        encode_rotation(mask, track, out)?;

        #[cfg(feature = "tracing")]
        let rotation_size = out.len() - rotation_offset;

        align4(out);

        #[cfg(feature = "tracing")]
        let scale_offset = out.len();

        encode_scale(mask, track, out)?;

        #[cfg(feature = "tracing")]
        let scale_size = out.len() - scale_offset;

        #[cfg(feature = "tracing")]
        debug_tracks.push(debug::SerializeDebugTrack {
            position_offset,
            position_size,
            rotation_offset,
            rotation_size,
            scale_offset,
            scale_size,
        });
    }

    align16(out);

    #[cfg(feature = "tracing")]
    debug::log_serialized_block(block, block_index, block_start, out.len(), &debug_tracks);

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

/// Encodes a position track.
///
/// # Errors
///
/// Returns [`Error`] if the position quantization type is invalid or if the
/// track representation does not match its mask.
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

/// Encodes a scale track.
///
/// # Errors
///
/// Returns [`Error`] if the scale quantization type is invalid or if the
/// track representation does not match its mask.
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

/// Encodes a position or scale track.
///
/// # Errors
///
/// Returns [`Error`] if the track representation is inconsistent with the
/// mask, if values are invalid, or if the scalar quantization type is not
/// supported.
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

/// Encodes a vector containing only static and identity axes.
///
/// # Errors
///
/// Returns [`Error`] if a static component is non-finite, an identity
/// component does not equal its Havok default, or a dynamic component is
/// present in a static vector representation.
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
/// The values in `track.tracks` are frame samples. They are fitted to a
/// degree-1 B-spline before quantization, matching PyNifly's exporter.
///
/// # Errors
///
/// Returns [`Error`] if the dynamic axes contain different sample counts, if
/// there are too many samples, if a sample is non-finite, if the B-spline
/// system cannot be solved, or if the quantization type is unsupported.
fn encode_dynamic_vector(
    mask: &TransformMask,
    types: [TransformType; 3],
    track: &SplineDynamicTrackVector,
    quantization: QuantizationType,
    default: f32,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    let sample_count = types
        .iter()
        .enumerate()
        .find_map(|(axis, &ty)| {
            (mask.sub_track_type(ty) == SplineTrackType::Dynamic)
                .then_some(track.tracks[axis].len())
        })
        .ok_or(Error::InvalidData(
            "dynamic vector contains no dynamic axis",
        ))?;

    if sample_count == 0 {
        return Err(Error::InvalidData(
            "dynamic vector contains no frame samples",
        ));
    }

    if sample_count <= 1 {
        return Err(Error::InvalidData(
            "dynamic vector requires at least two frame samples",
        ));
    }

    if sample_count > u16::MAX as usize + 1 {
        return Err(Error::InvalidData("too many vector spline frame samples"));
    }

    #[expect(clippy::needless_range_loop)]
    for axis in 0..3 {
        match mask.sub_track_type(types[axis]) {
            SplineTrackType::Dynamic => {
                if track.tracks[axis].len() != sample_count {
                    return Err(Error::InvalidData(
                        "dynamic vector axes have different frame sample counts",
                    ));
                }

                if track.tracks[axis].iter().any(|value| !value.is_finite()) {
                    return Err(Error::InvalidData(
                        "dynamic vector contains a non-finite value",
                    ));
                }
            }

            SplineTrackType::Static => {
                if track.tracks[axis].len() != 1 {
                    return Err(Error::InvalidData(
                        "static vector axis must contain exactly one value",
                    ));
                }

                if !track.tracks[axis][0].is_finite() {
                    return Err(Error::InvalidData(
                        "static vector contains a non-finite value",
                    ));
                }
            }

            SplineTrackType::Identity => {
                #[expect(
                    clippy::float_cmp,
                    reason = "Identity tracks must contain the exact Havok default value."
                )]
                if track.tracks[axis].iter().any(|&value| value != default) {
                    return Err(Error::InvalidData(
                        "identity vector component differs from its default value",
                    ));
                }
            }
        }
    }

    let degree = 1usize;
    let knots = make_clamped_knots(sample_count, degree)?;

    let num_items = u16::try_from(sample_count - 1)
        .map_err(|_| Error::InvalidData("too many vector spline frame samples"))?;

    out.extend_from_slice(&num_items.to_le_bytes());
    out.push(degree as u8);

    encode_knots(&knots, out)?;
    align4(out);

    let mut bounds = [[0.0f32; 2]; 3];
    let mut fitted = [None, None, None];

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

                let max = if (max - min).abs() < 1.0e-30 {
                    min + 1.0e-6
                } else {
                    max
                };

                bounds[axis] = [min, max];

                out.extend_from_slice(&min.to_le_bytes());
                out.extend_from_slice(&max.to_le_bytes());

                fitted[axis] = Some(fit_bspline_scalar(degree, &knots, values)?);
            }

            SplineTrackType::Static => {
                out.extend_from_slice(&track.tracks[axis][0].to_le_bytes());
            }

            SplineTrackType::Identity => {}
        }
    }

    for control_point in 0..sample_count {
        for axis in 0..3 {
            if mask.sub_track_type(types[axis]) != SplineTrackType::Dynamic {
                continue;
            }

            let values = fitted[axis]
                .as_ref()
                .ok_or(Error::InvalidData("missing fitted vector control points"))?;

            let value = *values.get(control_point).ok_or(Error::InvalidData(
                "vector control point index is out of bounds",
            ))?;

            let [min, max] = bounds[axis];

            write_quantized_scalar(out, value, min, max, quantization)?;
        }
    }

    align4(out);

    Ok(())
}

/// Fits scalar frame samples to a B-spline.
///
/// This follows PyNifly's exact-interpolation approach: there is one control
/// point for every frame sample and the collocation points are `0..N-1`.
///
/// # Errors
///
/// Returns [`Error`] if the frame count is zero, the knot vector is invalid,
/// or the interpolation matrix contains a singular pivot.
fn fit_bspline_scalar(
    degree: usize,
    knots: &[u8],
    frame_values: &[f32],
) -> Result<Vec<f32>, Error> {
    let n_cp = frame_values.len();

    if n_cp == 0 {
        return Err(Error::InvalidData("cannot fit an empty scalar spline"));
    }

    if knots.len() != n_cp + degree + 1 {
        return Err(Error::InvalidData(
            "B-spline knot count does not match frame sample count",
        ));
    }

    let knots = knots.iter().map(|&value| value as f32).collect::<Vec<_>>();

    let mut matrix = Vec::with_capacity(n_cp);

    for frame in 0..n_cp {
        matrix.push(bspline_basis_row(degree, frame as f32, n_cp, &knots)?);
    }

    let mut rhs = frame_values.to_vec();

    solve_banded(&mut matrix, &mut rhs)?;

    Ok(rhs)
}

/// Builds PyNifly's clamped uniform knot vector.
///
/// The knot range is `[0, n_cp - 1]` and interior knots are integer-valued.
///
/// # Errors
///
/// Returns [`Error`] if the control-point count is invalid or if the resulting
/// knot count cannot be represented.
fn make_clamped_knots(n_cp: usize, degree: usize) -> Result<Vec<u8>, Error> {
    if n_cp == 0 {
        return Err(Error::InvalidData(
            "B-spline requires at least one control point",
        ));
    }

    if degree == 0 {
        return Err(Error::InvalidData("B-spline degree must not be zero"));
    }

    if n_cp <= degree {
        return Err(Error::InvalidData(
            "B-spline control point count must exceed degree",
        ));
    }

    let max_t = n_cp - 1;

    if max_t > u8::MAX as usize {
        return Err(Error::InvalidData(
            "B-spline knot values exceed the u8 representation",
        ));
    }

    let knot_count = n_cp
        .checked_add(degree)
        .and_then(|value| value.checked_add(1))
        .ok_or(Error::InvalidData("B-spline knot count overflow"))?;

    let interior_count = n_cp
        .checked_sub(degree)
        .and_then(|value| value.checked_sub(1))
        .ok_or(Error::InvalidData("invalid B-spline interior knot count"))?;

    let mut knots = Vec::with_capacity(knot_count);

    knots.extend(core::iter::repeat_n(0u8, degree + 1));

    for index in 0..interior_count {
        let value = ((index + 1) * max_t + interior_count.div_ceil(2)) / (interior_count + 1);

        let value = u8::try_from(value)
            .map_err(|_| Error::InvalidData("B-spline knot exceeds u8 range"))?;

        knots.push(value);
    }

    let max_t =
        u8::try_from(max_t).map_err(|_| Error::InvalidData("B-spline knot exceeds u8 range"))?;

    knots.extend(core::iter::repeat_n(max_t, degree + 1));

    if knots.len() != knot_count {
        return Err(Error::InvalidData(
            "generated B-spline knot count is invalid",
        ));
    }

    Ok(knots)
}

/// Evaluates all B-spline basis functions at one parameter.
///
/// This is the de Boor basis calculation used by PyNifly.
///
/// # Errors
///
/// Returns [`Error`] if the knot vector is invalid or the requested degree
/// cannot be evaluated.
fn bspline_basis_row(degree: usize, t: f32, n_cp: usize, knots: &[f32]) -> Result<Vec<f32>, Error> {
    if degree == 0 {
        return Err(Error::InvalidData("B-spline degree must not be zero"));
    }

    if degree > 4 {
        return Err(Error::InvalidData(
            "B-spline degree is greater than the supported degree",
        ));
    }

    if knots.len() != n_cp + degree + 1 {
        return Err(Error::InvalidData("invalid B-spline knot vector length"));
    }

    let span = find_knot_span(degree, t, n_cp, knots)?;

    let mut basis = vec![0.0f32; degree + 1];
    basis[0] = 1.0;

    for i in 1..=degree {
        for j in (0..i).rev() {
            let left = span
                .checked_sub(j)
                .ok_or(Error::InvalidData("invalid B-spline knot span"))?;

            let right = span
                .checked_add(i)
                .and_then(|value| value.checked_sub(j))
                .ok_or(Error::InvalidData("invalid B-spline knot span"))?;

            if right >= knots.len() || left >= knots.len() {
                return Err(Error::InvalidData(
                    "B-spline basis references an invalid knot",
                ));
            }

            let denominator = knots[right] - knots[left];

            let a = if denominator >= 1.0e-10 {
                (t - knots[left]) / denominator
            } else {
                0.0
            };

            let tmp = basis[j] * a;

            basis[j + 1] += basis[j] - tmp;
            basis[j] = tmp;
        }
    }

    let mut row = vec![0.0f32; n_cp];

    #[expect(clippy::needless_range_loop)]
    for i in 0..=degree {
        let index = span
            .checked_sub(i)
            .ok_or(Error::InvalidData("invalid B-spline basis index"))?;

        if index < n_cp {
            row[index] = basis[i];
        }
    }

    Ok(row)
}

/// Finds the knot span containing a parameter value.
///
/// # Errors
///
/// Returns [`Error`] if the control-point count or knot vector is invalid.
fn find_knot_span(degree: usize, value: f32, n_cp: usize, knots: &[f32]) -> Result<usize, Error> {
    if n_cp == 0 {
        return Err(Error::InvalidData("B-spline has no control points"));
    }

    if knots.len() <= n_cp {
        return Err(Error::InvalidData("B-spline knot vector is too short"));
    }

    if degree >= knots.len() {
        return Err(Error::InvalidData("B-spline degree exceeds knot vector"));
    }

    if value >= knots[n_cp] {
        return Ok(n_cp - 1);
    }

    let mut low = degree;
    let mut high = n_cp;

    while value < knots[low] || value >= knots[low + 1] {
        let mid = (low + high) / 2;

        if value < knots[mid] {
            high = mid;
        } else {
            low = mid;
        }

        if low + 1 >= knots.len() {
            return Err(Error::InvalidData(
                "B-spline knot span search exceeded knot vector",
            ));
        }

        if low == high {
            break;
        }
    }

    Ok(low)
}

/// Solves the interpolation system using PyNifly's banded Gaussian
/// elimination with partial pivoting.
///
/// # Errors
///
/// Returns [`Error`] if the matrix is empty or contains a singular pivot.
fn solve_banded(matrix: &mut [Vec<f32>], rhs: &mut [f32]) -> Result<(), Error> {
    let n = rhs.len();

    if n == 0 || matrix.len() != n {
        return Err(Error::InvalidData("invalid B-spline interpolation matrix"));
    }

    for row in matrix.iter() {
        if row.len() != n {
            return Err(Error::InvalidData(
                "B-spline interpolation matrix is not square",
            ));
        }
    }

    for col in 0..n {
        let mut max_row = col;
        let mut max_value = matrix[col][col].abs();

        #[expect(clippy::needless_range_loop)]
        for row in (col + 1)..n.min(col + 8) {
            let value = matrix[row][col].abs();

            if value > max_value {
                max_value = value;
                max_row = row;
            }
        }

        if max_row != col {
            matrix.swap(col, max_row);
            rhs.swap(col, max_row);
        }

        let pivot = matrix[col][col];

        if pivot.abs() < 1.0e-30 {
            return Err(Error::InvalidData(
                "B-spline interpolation matrix is singular",
            ));
        }

        for row in (col + 1)..n.min(col + 8) {
            let factor = matrix[row][col] / pivot;

            if factor.abs() < 1.0e-30 {
                continue;
            }

            #[expect(clippy::needless_range_loop)]
            for k in col..n.min(col + 8) {
                matrix[row][k] = factor.mul_add(-matrix[col][k], matrix[row][k]);
            }

            rhs[row] = factor.mul_add(-rhs[col], rhs[row]);
        }
    }

    for row in (0..n).rev() {
        let mut value = rhs[row];

        for column in (row + 1)..n.min(row + 8) {
            value = matrix[row][column].mul_add(-rhs[column], value);
        }

        let pivot = matrix[row][row];

        if pivot.abs() < 1.0e-30 {
            return Err(Error::InvalidData(
                "B-spline back substitution encountered a singular pivot",
            ));
        }

        rhs[row] = value / pivot;
    }

    Ok(())
}

/// Writes one scalar using Havok's scalar quantization.
///
/// # Errors
///
/// Returns [`Error`] if the value or bounds are non-finite, if the bounds are
/// invalid, or if the requested quantization is not an 8-bit or 16-bit scalar
/// format.
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

    let normalized = if range.abs() < 1.0e-30 {
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
fn encode_dynamic_rotation(
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

/// Fits four quaternion components independently to the same B-spline.
///
/// # Errors
///
/// Returns [`Error`] if any scalar component cannot be fitted.
fn fit_bspline_quaternion(
    degree: usize,
    knots: &[u8],
    samples: &[[f32; 4]],
) -> Result<Vec<[f32; 4]>, Error> {
    let mut components = [
        Vec::<f32>::with_capacity(samples.len()),
        Vec::<f32>::with_capacity(samples.len()),
        Vec::<f32>::with_capacity(samples.len()),
        Vec::<f32>::with_capacity(samples.len()),
    ];

    for sample in samples {
        for component in 0..4 {
            components[component].push(sample[component]);
        }
    }

    let fitted = [
        fit_bspline_scalar(degree, knots, &components[0])?,
        fit_bspline_scalar(degree, knots, &components[1])?,
        fit_bspline_scalar(degree, knots, &components[2])?,
        fit_bspline_scalar(degree, knots, &components[3])?,
    ];

    let mut result = Vec::with_capacity(samples.len());

    #[expect(clippy::needless_range_loop)]
    for index in 0..samples.len() {
        result.push([
            fitted[0][index],
            fitted[1][index],
            fitted[2][index],
            fitted[3][index],
        ]);
    }

    Ok(result)
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
fn make_quaternion_continuous(quaternions: &mut [[f32; 4]]) {
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
fn normalize_quaternion(value: [f32; 4]) -> Result<[f32; 4], Error> {
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
fn write_quaternion(
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
fn write_quat_three_comp40(out: &mut Vec<u8>, quaternion: [f32; 4]) {
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
fn write_quat_three_comp48(out: &mut Vec<u8>, quaternion: [f32; 4]) {
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

/// Writes the spline knot vector.
///
/// # Errors
///
/// Returns [`Error`] if a knot cannot be represented as an unsigned byte.
fn encode_knots(knots: &[u8], out: &mut Vec<u8>) -> Result<(), Error> {
    if knots.is_empty() {
        return Err(Error::InvalidData("spline knot vector is empty"));
    }

    out.extend_from_slice(knots);

    Ok(())
}

/// Aligns a quaternion payload according to its quantization format.
#[inline]
fn align_rotation(out: &mut Vec<u8>, quantization: QuantizationType) {
    let alignment = match quantization {
        QuantizationType::Bit40
        | QuantizationType::Bit24
        | QuantizationType::Bit8
        | QuantizationType::Bit16 => 1,

        QuantizationType::Bit48 | QuantizationType::Bit16Quat => 2,

        QuantizationType::Bit32 | QuantizationType::Uncompressed => 4,
    };

    align_to(out, alignment);
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
