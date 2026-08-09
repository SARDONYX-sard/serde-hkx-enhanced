// SPDX-License-Identifier: GPL-3.0-or-later
//
// Reference:
// https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.hpp
// https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.cpp

//! Havok spline animation decompression.
//!
//! # Overview
//!
//! A Havok spline block is a compressed representation of animation tracks.
//! It does not normally store one complete `QsTransform` for every frame.
//!
//! Instead, a track can store:
//!
//! - `Static`: one value which is used for the entire track.
//! - `Dynamic`: a small number of spline control points.
//! - `Identity`: no value at all; the component uses its predefined default.
//!
//! For a dynamic track, the stored control points are evaluated with a
//! B-spline to obtain the value at a particular frame.
//!
//! Conceptually:
//!
//! ```text
//!
//!   animation frames
//!   0   1   2   3   4   5   6   7
//!   |   |   |   |   |   |   |   |
//!   |   |   |   |   |   |   |   |
//!   +---+---+---+---+---+---+---+
//!              B-spline
//!                  |
//!                  v
//!          evaluated animation value
//!
//! ```
//!
//! The important distinction is that the binary does not necessarily contain
//! the value for each frame. It contains enough information to reconstruct a
//! smooth curve.
//!
//! # B-spline
//!
//! A B-spline is a curve defined by control points and a knot vector.
//!
//! For example, instead of storing:
//!
//! ```text
//! frame:          0     1     2     3     4     5
//! value:         1.0   1.2   1.8   2.7   3.1   3.0
//! ```
//!
//! the format can store a much smaller set of control points:
//!
//! ```text
//! control points:
//!
//!       P0        P1        P2        P3
//!        *---------*---------*---------*
//!          \      / \       /
//!           \    /   \     /
//!            \__/     \___/
//!
//!              B-spline
//! ```
//!
//! The knot vector determines which control points influence each part of
//! the curve. `degree` determines the polynomial degree of the spline.
//!
//! During decoding, `find_knot_span()` finds the part of the knot vector
//! containing the requested frame, and `get_single_point()` evaluates the
//! corresponding B-spline basis functions.
//!
//! # Quantization
//!
//! Dynamic scalar tracks usually do not store their control points directly
//! as `f32` values.
//!
//! Instead, the encoder first chooses a real-valued range:
//
//! ```text
//!             min                         max
//!              |---------------------------|
//!              0                           1
//! ```
//!
//! and stores each value as a small integer representing its position inside
//! that range.
//!
//! For 8-bit quantization:
//
//! ```text
//! stored value = 0..=255
//!
//! normalized = stored / 255.0
//!
//! decoded = min + normalized * (max - min)
//! ```
//!
//! Therefore:
//!
//! ```text
//! stored integer
//!       |
//!       v
//!   0 .. 255
//!       |
//!       | normalize
//!       v
//!   0.0 .. 1.0
//!       |
//!       | restore range
//!       v
//!   min .. max
//! ```
//!
//! Sixteen-bit quantization performs the same operation with `0..=65535`.
//!
//! This is intentionally lossy. The decoded value is an approximation of
//! the original value because many possible `f32` values map to the same
//! quantized integer.
//!
//! `TrackBbox { min, max }` stores the information required to map the
//! normalized integer back into the original numerical range.
//!
//! # Quaternion compression
//!
//! Rotation tracks are different from position and scale tracks.
//!
//! A quaternion contains four components, but a normalized quaternion has
//! only three independent degrees of freedom. The fourth component can be
//! reconstructed from the unit-length constraint:
//!
//! ```text
//! x² + y² + z² + w² = 1
//! ```
//!
//! Havok therefore provides specialized quaternion encodings such as
//! 32-bit, 40-bit, and 48-bit representations.
//!
//! These are not ordinary floating-point quantization formats. They pack
//! quaternion information into a fixed number of bits and reconstruct an
//! approximate normalized quaternion during decoding.
//!
//! The `read32_quat()`, `read40_quat()`, and `read48_quat()` functions are
//! therefore intentionally specialized and should be kept close to the
//! corresponding binary specification/reference implementation.
//!
//! `Uncompressed` is the simple case: four IEEE-754 `f32` values are stored
//! directly.
//!
//! # Track layout
//!
//! Each transform track contains three logical components:
//!
//! ```text
//! TransformTrack
//! ├── position
//! │   ├── X
//! │   ├── Y
//! │   └── Z
//! ├── rotation
//! │   └── quaternion
//! └── scale
//!     ├── X
//!     ├── Y
//!     └── Z
//! ```
//!
//! Each component is independently classified as `Static`, `Dynamic`, or
//! `Identity` by `TransformMask`.
//!
//! This is important because the binary does not have a fixed representation
//! for every transform. The mask determines which bytes exist after it.
//!
//! For example:
//
//! ```text
//! position X = Static     -> one f32 exists
//! position Y = Identity   -> no bytes exist
//! position Z = Dynamic    -> control-point data exists
//! ```
//!
//! The decoder must therefore inspect the mask before deciding what to read.
//!
//! # Alignment
//!
//! Some parts of the Havok format are aligned to 2- or 4-byte boundaries.
//! `Reader::align()` advances over padding bytes; those bytes are not part of
//! the logical animation data.
//!
//! Alignment must be handled at the same places as the reference format.
//! Reading the correct value with the wrong alignment will shift every
//! subsequent read and corrupt the remainder of the block.
//!
//! # Decode pipeline
//!
//! The complete process is:
//
//! ```text
//! binary block
//!      |
//!      v
//! TransformMask
//!      |
//!      +--------------------+
//!      |                    |
//!      v                    v
//! position/scale        rotation
//!      |                    |
//!      |                    +--> quaternion decoding
//!      |
//!      +--> static value
//!      |
//!      +--> identity/default value
//!      |
//!      +--> quantized control points
//!                    |
//!                    v
//!                B-spline
//!                    |
//!                    v
//!             evaluated value
//!                    |
//!                    +---------+
//!                              |
//!                              v
//!                        QsTransform
//! ```
//!
//! The purpose of this module is therefore not to reproduce the original
//! floating-point values bit-for-bit. The binary format itself deliberately
//! trades precision for a smaller representation.
//!
//! The decoder reconstructs the value represented by the compressed spline,
//! within the precision allowed by the selected quantization format.

use core::{
    f32::consts::PI,
    mem::size_of,
    ops::{Add, Mul},
};

use havok_types::{QsTransform, Quaternion, Vector4};

use super::math::{
    IVec4A16, QuantizationType, QuatA16, SplineDynamicTrackQuat, SplineDynamicTrackVector,
    SplineStaticTrack, SplineTrackQuat, SplineTrackType, SplineTrackVector, TransformMask,
    TransformSplineBlock, TransformTrack, TransformType, Vec4A16,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplineError {
    /// The input ended before the requested number of bytes was available.
    ///
    /// This usually means that the block is truncated or that an earlier
    /// value was decoded with the wrong size/alignment.
    UnexpectedEof,

    /// The input contains an invalid quantization type.
    ///
    /// Quantization types determine how the following bytes are interpreted.
    /// An invalid value means the decoder cannot determine the layout of the
    /// following data.
    InvalidQuantizationType(u8),

    /// The input contains an invalid spline degree.
    ///
    /// The degree controls how many control points participate in the
    /// evaluation of a spline segment. This implementation supports degrees
    /// up to four.
    InvalidDegree(u8),

    /// The spline knot vector is malformed.
    ///
    /// A knot vector must contain enough entries for the declared degree and
    /// control-point count, and its denominators must be valid during basis
    /// function evaluation.
    InvalidKnotVector,

    /// The spline control-point count is invalid.
    ///
    /// A dynamic spline must contain at least one control point.
    InvalidControlPointCount,

    /// A spline evaluation index is outside the control-point array.
    ///
    /// This indicates an inconsistency between the declared degree, knot
    /// vector, and number of control points.
    InvalidControlPointIndex,

    /// The decompressed block does not contain the requested track.
    TrackOutOfRange,

    /// The input data cannot be represented by the spline format.
    ///
    /// This is primarily used by the encoder when a requested representation
    /// conflicts with the information expressible by the transform mask.
    InvalidData(&'static str),
}

impl core::fmt::Display for SplineError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnexpectedEof => f.write_str("unexpected end of spline data"),
            Self::InvalidQuantizationType(value) => {
                write!(f, "invalid quantization type: {value}")
            }
            Self::InvalidDegree(value) => {
                write!(f, "invalid spline degree: {value}")
            }
            Self::InvalidKnotVector => f.write_str("invalid spline knot vector"),
            Self::InvalidControlPointCount => f.write_str("invalid spline control-point count"),
            Self::InvalidControlPointIndex => f.write_str("invalid spline control-point index"),
            Self::TrackOutOfRange => f.write_str("track index is out of range"),
            Self::InvalidData(message) => f.write_str(message),
        }
    }
}

impl core::error::Error for SplineError {}

struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    const fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    fn read_u8(&mut self) -> core::result::Result<u8, SplineError> {
        let value = *self
            .data
            .get(self.position)
            .ok_or(SplineError::UnexpectedEof)?;

        self.position += 1;
        Ok(value)
    }

    fn read_u16_le(&mut self) -> core::result::Result<u16, SplineError> {
        let bytes = self.read_array::<2>()?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn read_u32_le(&mut self) -> core::result::Result<u32, SplineError> {
        let bytes = self.read_array::<4>()?;
        Ok(u32::from_le_bytes(bytes))
    }

    #[expect(unused)]
    fn read_u64_le(&mut self) -> core::result::Result<u64, SplineError> {
        let bytes = self.read_array::<8>()?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn read_f32_le(&mut self) -> core::result::Result<f32, SplineError> {
        Ok(f32::from_bits(self.read_u32_le()?))
    }

    fn read_array<const N: usize>(&mut self) -> core::result::Result<[u8; N], SplineError> {
        let end = self
            .position
            .checked_add(N)
            .ok_or(SplineError::UnexpectedEof)?;

        let bytes = self
            .data
            .get(self.position..end)
            .ok_or(SplineError::UnexpectedEof)?;

        let mut result = [0u8; N];
        result.copy_from_slice(bytes);

        self.position = end;

        Ok(result)
    }

    /// Skip `<count>` bytes.
    fn skip(&mut self, count: usize) -> core::result::Result<(), SplineError> {
        let end = self
            .position
            .checked_add(count)
            .ok_or(SplineError::UnexpectedEof)?;

        if end > self.data.len() {
            return Err(SplineError::UnexpectedEof);
        }

        self.position = end;
        Ok(())
    }

    fn align(&mut self, alignment: usize) -> core::result::Result<(), SplineError> {
        debug_assert!(alignment.is_power_of_two());

        let mask = alignment - 1;
        let aligned = (self.position + mask) & !mask;

        self.skip(aligned.saturating_sub(self.position))
    }

    fn read_bytes(&mut self, count: usize) -> core::result::Result<&'a [u8], SplineError> {
        let end = self
            .position
            .checked_add(count)
            .ok_or(SplineError::UnexpectedEof)?;

        let result = self
            .data
            .get(self.position..end)
            .ok_or(SplineError::UnexpectedEof)?;

        self.position = end;

        Ok(result)
    }
}

/// The 32-bit representation does not store four independent floats.
///
/// A normalized quaternion has:
///
///     x² + y² + z² + w² = 1
///
/// so only three degrees of freedom are necessary. Havok packs the
/// information needed to reconstruct those values into a 32-bit word.
///
/// The bit fields contain:
///
///     - a magnitude-related value,
///     - angular information,
///     - sign bits for the reconstructed components.
///
/// The constants below are therefore part of the binary encoding and
/// should not be replaced with a generic "epsilon" or generic
/// quantization formula.
fn read32_quat(reader: &mut Reader<'_>) -> core::result::Result<QuatA16, SplineError> {
    const R_MASK: u32 = (1 << 10) - 1;
    const R_FRAC: f32 = 1.0 / ((1u32 << 10) - 1) as f32;
    const PI_4: f32 = 0.25 * PI;
    const PHI_FRAC: f32 = (0.5 * PI) / 511.0;

    let c_val = reader.read_u32_le()?;

    // Recover the packed radial component.
    let mut r = ((c_val >> 18) & R_MASK) as f32 * R_FRAC;
    r = 1.0 - r * r;

    // The lower 18 bits contain the packed angular coordinates.
    let phi_theta = (c_val & 0x3ffff) as f32;

    // Decode the triangular/angular packing used by the reference implementation.
    let mut phi = phi_theta.sqrt().floor();
    let mut theta = 0.0;

    if phi > 0.0 {
        theta = PI_4 * (phi_theta - phi * phi) / phi;
        phi *= PHI_FRAC;
    }

    // Reconstruct the missing magnitude from the unit-quaternion relation.
    let magnitude = (1.0 - r * r).sqrt();

    let sin_phi = phi.sin();
    let cos_phi = phi.cos();
    let sin_theta = theta.sin();
    let cos_theta = theta.cos();

    let value = Vec4A16::new(sin_phi, sin_phi, cos_phi, r)
        * Vec4A16::new(cos_theta, sin_theta, 1.0, 1.0)
        * Vec4A16::new(magnitude, magnitude, magnitude, 1.0);

    // The four high bits contain the signs of the reconstructed components.
    let sign_mask = IVec4A16::new(0x1000_0000, 0x2000_0000, 0x4000_0000, 0x8000_0000u32 as i32);

    let packed = IVec4A16::new(c_val as i32, c_val as i32, c_val as i32, c_val as i32);

    let blend_mask = (packed & sign_mask).cmp_eq(sign_mask);

    let value = value.select(-value, blend_mask);

    Ok(QuatA16::from_vec4(value))
}

fn read40_quat(reader: &mut Reader<'_>) -> core::result::Result<QuatA16, SplineError> {
    let bytes = reader.read_bytes(5)?;

    let va = bytes[0] as u32 | (((bytes[1] & 0x0F) as u32) << 8);
    let vb = ((bytes[1] >> 4) & 0x0F) as u32 | ((bytes[2] as u32) << 4);
    let vc = bytes[3] as u32 | (((bytes[4] & 0x0F) as u32) << 8);

    let result_shift = ((bytes[4] >> 4) & 0x03) as usize;
    let sign = (bytes[4] >> 6) & 0x01 != 0;

    const INV_SQRT2: f32 = core::f32::consts::FRAC_1_SQRT_2;

    let dequant =
        |value: u32| -> f32 { (value as f32 / 4095.0).mul_add(2.0 * INV_SQRT2, -INV_SQRT2) };

    let components = [dequant(va), dequant(vb), dequant(vc)];

    let sum_sq = components[2].mul_add(
        components[2],
        components[1].mul_add(components[1], components[0] * components[0]),
    );

    let mut reconstructed = (1.0 - sum_sq).max(0.0).sqrt();

    if sign {
        reconstructed = -reconstructed;
    }

    let mut result = [0.0; 4];
    let mut source = 0;

    for (i, result) in result.iter_mut().enumerate() {
        if i == result_shift {
            *result = reconstructed;
        } else {
            *result = components[source];
            source += 1;
        }
    }

    Ok(QuatA16::new(result[0], result[1], result[2], result[3]))
}

fn read48_quat(reader: &mut Reader<'_>) -> core::result::Result<QuatA16, SplineError> {
    const MASK: u32 = (1 << 15) - 1;
    const FRACTION: f32 = 0.000043161;

    let x = reader.read_u16_le()?;
    let y = reader.read_u16_le()?;
    let z = reader.read_u16_le()?;

    let result_shift = (((y >> 14) & 2) | ((x >> 15) & 1)) as u32;
    let r_sign = (z >> 15) != 0;

    let value = IVec4A16::new(x as i32, y as i32, z as i32, 0);

    let mask = IVec4A16::splat(MASK as i32);

    let value = (value & mask) - IVec4A16::splat((MASK >> 1) as i32);

    let value = value.to_f32() * Vec4A16::new(FRACTION, FRACTION, FRACTION, 0.0);

    let value = value * Vec4A16::new(1.0, 1.0, 1.0, if r_sign { -1.0 } else { 1.0 });

    let value = match result_shift {
        0 => value.shuffle::<0b11_00_01_10>(),
        1 => value.shuffle::<0b00_11_01_10>(),
        2 => value.shuffle::<0b00_01_11_10>(),
        _ => value,
    };

    Ok(QuatA16::from_vec4(value))
}

fn read_quat(
    reader: &mut Reader<'_>,
    quantization: QuantizationType,
) -> core::result::Result<QuatA16, SplineError> {
    match quantization {
        QuantizationType::Bit32 => read32_quat(reader),

        QuantizationType::Bit40 => read40_quat(reader),

        QuantizationType::Bit48 => read48_quat(reader),

        QuantizationType::Uncompressed => {
            let x = reader.read_f32_le()?;
            let y = reader.read_f32_le()?;
            let z = reader.read_f32_le()?;
            let w = reader.read_f32_le()?;

            Ok(QuatA16::new(x, y, z, w))
        }

        _ => Ok(QuatA16::identity()),
    }
}

fn find_knot_span(
    degree: usize,
    value: f32,
    control_point_count: usize,
    knots: &[f32],
) -> core::result::Result<usize, SplineError> {
    if control_point_count == 0 {
        return Err(SplineError::InvalidControlPointCount);
    }

    if knots.len() <= control_point_count {
        return Err(SplineError::InvalidKnotVector);
    }

    if value >= knots[control_point_count] {
        return Ok(control_point_count - 1);
    }

    let mut low = degree;
    let mut high = control_point_count;

    if low >= knots.len() || high >= knots.len() {
        return Err(SplineError::InvalidKnotVector);
    }

    let mut mid = (low + high) / 2;

    while value < knots[mid] || value >= knots[mid + 1] {
        if value < knots[mid] {
            high = mid;
        } else {
            low = mid;
        }

        mid = (low + high) / 2;

        if mid + 1 >= knots.len() {
            return Err(SplineError::InvalidKnotVector);
        }
    }

    Ok(mid)
}

fn get_single_point<T>(
    knot_span: usize,
    degree: usize,
    frame: f32,
    knots: &[f32],
    control_points: &[T],
) -> core::result::Result<T, SplineError>
where
    T: Copy + Add<Output = T> + Mul<f32, Output = T>,
{
    if control_points.is_empty() {
        return Err(SplineError::InvalidControlPointCount);
    }

    // The implementation below is the Cox-de Boor recurrence for evaluating
    // B-spline basis functions.
    //
    // Conceptually, each basis function answers:
    //
    //     "How much does this control point contribute at this frame?"
    //
    // For a degree-3 spline, at most four neighboring control points
    // participate in the result:
    //
    //             P0       P1       P2       P3
    //              *--------*--------*--------*
    //                 \       |       /
    //                  \      |      /
    //                   \     |     /
    //                    \____|____/
    //                         ^
    //                       frame
    //
    // The resulting value is the weighted sum of those control points.
    if degree > 4 {
        return Err(SplineError::InvalidDegree(degree as u8));
    }

    if knot_span < degree {
        return Err(SplineError::InvalidControlPointIndex);
    }

    let mut basis = [0.0f32; 5];
    basis[0] = 1.0;

    for i in 1..=degree {
        for j in (0..i).rev() {
            let left_index = knot_span - j;
            let right_index = knot_span + i - j;

            if right_index >= knots.len() || left_index >= knots.len() {
                return Err(SplineError::InvalidKnotVector);
            }

            let denominator = knots[right_index] - knots[left_index];

            if denominator == 0.0 {
                return Err(SplineError::InvalidKnotVector);
            }

            let a = (frame - knots[left_index]) / denominator;
            let tmp = basis[j] * a;

            basis[j + 1] += basis[j] - tmp;
            basis[j] = tmp;
        }
    }

    // `knot_span - degree` is the first control point participating in this
    // spline segment. Only the local neighborhood is required; the complete
    // control-point array is not evaluated for every frame.
    let first_index = knot_span
        .checked_sub(degree)
        .ok_or(SplineError::InvalidControlPointIndex)?;

    let first = control_points
        .get(first_index)
        .copied()
        .ok_or(SplineError::InvalidControlPointIndex)?;

    let mut result = first * basis[degree];

    for i in 1..=degree {
        let index = knot_span
            .checked_sub(degree - i)
            .ok_or(SplineError::InvalidControlPointIndex)?;

        let control_point = control_points
            .get(index)
            .copied()
            .ok_or(SplineError::InvalidControlPointIndex)?;

        result = result + control_point * basis[degree - i];
    }

    Ok(result)
}

fn get_single_scalar_point(
    knot_span: usize,
    degree: usize,
    frame: f32,
    knots: &[f32],
    control_points: &[f32],
) -> core::result::Result<f32, SplineError> {
    get_single_point(knot_span, degree, frame, knots, control_points)
}

fn evaluate_vector_track(
    track: &SplineDynamicTrackVector,
    local_frame: f32,
) -> core::result::Result<Vector4, SplineError> {
    // Position and scale are represented as three independent scalar
    // splines:
    //
    //     X(t)
    //     Y(t)
    //     Z(t)
    //
    // They share the same knot vector and degree, but each axis has its own
    // control-point values.
    //
    // This is why the stored representation is:
    //
    //     tracks: [Vec<f32>; 3]
    //
    // rather than Vec<Vector4>.
    let mut result = Vector4 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 0.0,
    };

    // All three axes use the same knot vector. Once the knot span for the
    // requested frame has been found, it can be reused for X/Y/Z.
    let mut knot_span = None;

    for (control_points, axis) in
        track
            .tracks
            .iter()
            .zip([&mut result.x, &mut result.y, &mut result.z])
    {
        if control_points.len() == 1 {
            *axis = control_points[0];
            continue;
        }

        let span = match knot_span {
            Some(span) => span,
            None => {
                // Locate the knot interval containing this frame.
                //
                // Only the control points surrounding this interval affect the B-spline result.
                let span = find_knot_span(
                    track.degree as usize,
                    local_frame,
                    control_points.len(),
                    &track.knots,
                )?;

                knot_span = Some(span);
                span
            }
        };

        *axis = get_single_scalar_point(
            span,
            track.degree as usize,
            local_frame,
            &track.knots,
            control_points,
        )?;
    }

    Ok(result)
}

fn evaluate_quat_track(
    track: &SplineDynamicTrackQuat,
    local_frame: f32,
) -> core::result::Result<QuatA16, SplineError> {
    let knot_span = find_knot_span(
        track.degree as usize,
        local_frame,
        track.track.len(),
        &track.knots,
    )?;

    get_single_point(
        knot_span,
        track.degree as usize,
        local_frame,
        &track.knots,
        &track.track,
    )
}

fn read_transform_mask(
    reader: &mut Reader<'_>,
) -> core::result::Result<TransformMask, SplineError> {
    Ok(TransformMask {
        quantization_types: reader.read_u8()?,
        position_types: reader.read_u8()?,
        rotation_types: reader.read_u8()?,
        scale_types: reader.read_u8()?,
    })
}

/// The original floating-point range represented by the quantized values.
///
/// A quantized control point is stored as an integer, not as an f32:
///
///     integer -> normalized [0, 1] -> [min, max]
///
/// Without these two values the original numerical scale could not be
/// reconstructed.
#[derive(Clone, Copy)]
struct TrackBbox {
    min: f32,
    max: f32,
}

fn read_dynamic_vector_track(
    reader: &mut Reader<'_>,
    mask: TransformMask,
    quantization: QuantizationType,
    default_value: f32,
    transform_types: [TransformType; 3],
) -> core::result::Result<SplineTrackVector, SplineError> {
    // `num_items` is one less than the number of control points.
    //
    // The actual number of control points is therefore:
    //
    //     control_points = num_items + 1
    //
    // This convention is used by the binary format and is also why the
    // evaluation loop later uses `0..=num_items`.
    let num_items = reader.read_u16_le()? as usize;

    // // The byte after numItems is reserved by the source format.
    // // It occupies space in the binary but has no semantic value here.
    // reader.skip(1)?;

    let degree = reader.read_u8()?;

    // For a B-spline, the knot vector must contain enough entries to describe
    // both the control points and the polynomial degree.
    //
    // Havok's representation uses:
    //
    //     knot_count = num_items + degree + 2
    //
    // Do not derive this from a generic B-spline implementation without also
    // checking the reference binary format: the serialized count is part of
    // Havok's representation.
    let knot_count = num_items
        .checked_add(degree as usize)
        .and_then(|value| value.checked_add(2))
        .ok_or(SplineError::InvalidControlPointCount)?;

    // Knots are stored as bytes in this representation.
    //
    // They are promoted to f32 because spline evaluation operates in the
    // floating-point domain. The binary representation is therefore not a
    // general-purpose f32 knot vector.
    let knots = reader
        .read_bytes(knot_count)?
        .iter()
        .map(|&value| value as f32)
        .collect::<Vec<_>>();

    reader.align(4)?;

    let mut extremes = [
        TrackBbox { min: 0.0, max: 0.0 },
        TrackBbox { min: 0.0, max: 0.0 },
        TrackBbox { min: 0.0, max: 0.0 },
    ];

    let mut tracks = [Vec::<f32>::new(), Vec::<f32>::new(), Vec::<f32>::new()];

    for axis in 0..3 {
        match mask.sub_track_type(transform_types[axis]) {
            SplineTrackType::Dynamic => {
                // A dynamic component stores its control points in normalized
                // quantized form. These two floats restore the component's
                // original numerical range.
                //
                // Example:
                //
                //     stored = 128
                //     normalized = 128 / 255
                //     value = min + normalized * (max - min)
                extremes[axis] = TrackBbox {
                    min: reader.read_f32_le()?,
                    max: reader.read_f32_le()?,
                };

                tracks[axis].resize(num_items + 1, 0.0);
            }

            SplineTrackType::Static => {
                // A static component has exactly one f32 value. It does not
                // need a spline because the value is constant for every frame.
                tracks[axis].push(reader.read_f32_le()?);
            }

            SplineTrackType::Identity => {
                // Identity components are omitted from the binary completely.
                // Their value is supplied by the caller as `default_value`
                // (0.0 for position and 1.0 for scale).
                tracks[axis].push(default_value);
            }
        }
    }

    #[expect(clippy::needless_range_loop)]
    for item in 0..=num_items {
        for axis in 0..3 {
            let transform_type = transform_types[axis];

            if mask.sub_track_type(transform_type) != SplineTrackType::Dynamic {
                continue;
            }

            // The control point is stored as a normalized integer.
            //
            // Quantization deliberately reduces precision in exchange for
            // fewer bytes. This is therefore reconstruction, not a bit-exact
            // recovery of the original f32.
            let value = match quantization {
                QuantizationType::Bit8 => {
                    let value = reader.read_u8()? as f32;
                    value / 255.0
                }

                QuantizationType::Bit16 => {
                    let value = reader.read_u16_le()? as f32;

                    // The source implementation advances by six bytes for
                    // each scalar:
                    //
                    //     2 bytes: quantized value
                    //     2 bytes: padding
                    //
                    // The remaining layout is defined by the surrounding
                    // component packing. This skip must therefore remain
                    // synchronized with the reference implementation.
                    // reader.skip(2)?;

                    value / 65535.0
                }

                _ => {
                    return Err(SplineError::InvalidQuantizationType(quantization as u8));
                }
            };

            // Convert the normalized quantized value back into the original numerical range.
            tracks[axis][item] =
                (extremes[axis].max - extremes[axis].min).mul_add(value, extremes[axis].min);
        }
    }

    reader.align(4)?;

    Ok(SplineTrackVector::Dynamic(SplineDynamicTrackVector {
        tracks,
        knots,
        degree,
    }))
}

fn read_vector_track(
    reader: &mut Reader<'_>,
    mask: TransformMask,
    quantization: QuantizationType,
    default_value: f32,
    transform_types: [TransformType; 3],
) -> core::result::Result<SplineTrackVector, SplineError> {
    let dynamic = transform_types
        .iter()
        .copied()
        .any(|ty| mask.sub_track_type(ty) == SplineTrackType::Dynamic);

    if dynamic {
        return read_dynamic_vector_track(
            reader,
            mask,
            quantization,
            default_value,
            transform_types,
        );
    }

    let mut value = Vector4 {
        x: default_value,
        y: default_value,
        z: default_value,
        w: 0.0,
    };

    for axis in [0, 1, 2] {
        if mask.sub_track_type(transform_types[axis]) == SplineTrackType::Static {
            let component = reader.read_f32_le()?;

            match axis {
                0 => value.x = component,
                1 => value.y = component,
                2 => value.z = component,
                _ => unreachable!(),
            }
        }
    }

    Ok(SplineTrackVector::Static(SplineStaticTrack { value }))
}

fn read_rotation_track(
    reader: &mut Reader<'_>,
    mask: TransformMask,
) -> core::result::Result<SplineTrackQuat, SplineError> {
    match mask.sub_track_type(TransformType::Rotation) {
        SplineTrackType::Dynamic => {
            let num_items = reader.read_u16_le()? as usize;

            // Reserved byte.
            // reader.skip(1)?;

            let degree = reader.read_u8()?;

            let knot_count = num_items
                .checked_add(degree as usize)
                .and_then(|value| value.checked_add(2))
                .ok_or(SplineError::InvalidControlPointCount)?;

            let knots = reader
                .read_bytes(knot_count)?
                .iter()
                .map(|&value| value as f32)
                .collect::<Vec<_>>();

            let quantization = mask.rotation_quantization_type()?;

            match quantization {
                QuantizationType::Bit48 | QuantizationType::Bit16Quat => reader.align(2)?,
                QuantizationType::Bit32 | QuantizationType::Uncompressed => reader.align(4)?,
                _ => {}
            }

            let mut track = Vec::with_capacity(num_items + 1);

            for _ in 0..=num_items {
                track.push(read_quat(reader, quantization)?);
            }

            Ok(SplineTrackQuat::Dynamic(SplineDynamicTrackQuat {
                track,
                knots,
                degree,
            }))
        }

        SplineTrackType::Static => {
            let quantization = mask.rotation_quantization_type()?;
            match quantization {
                QuantizationType::Bit48 | QuantizationType::Bit16Quat => reader.align(2)?,
                QuantizationType::Bit32 | QuantizationType::Uncompressed => reader.align(4)?,
                _ => {}
            }

            let value = read_quat(reader, quantization)?;
            Ok(SplineTrackQuat::Static(SplineStaticTrack { value }))
        }

        SplineTrackType::Identity => Ok(SplineTrackQuat::Identity),
    }
}

fn read_transform_track(
    reader: &mut Reader<'_>,
    mask: TransformMask,
) -> core::result::Result<TransformTrack, SplineError> {
    let position = read_vector_track(
        reader,
        mask,
        mask.position_quantization_type()?,
        0.0,
        [
            TransformType::PosX,
            TransformType::PosY,
            TransformType::PosZ,
        ],
    )?;

    let rotation = read_rotation_track(reader, mask)?;

    reader.align(4)?;

    let scale = read_vector_track(
        reader,
        mask,
        mask.scale_quantization_type()?,
        1.0,
        [
            TransformType::ScaleX,
            TransformType::ScaleY,
            TransformType::ScaleZ,
        ],
    )?;

    Ok(TransformTrack {
        position,
        rotation,
        scale,
    })
}

impl TransformSplineBlock {
    /// Decompresses one transform spline block.
    ///
    /// A block is decoded in the same order in which the binary stores it:
    ///
    /// ```text
    /// +-----------------------------+
    /// | TransformMask × num_tracks  |
    /// +-----------------------------+
    /// | float-track region          |
    /// +-----------------------------+
    /// | alignment padding            |
    /// +-----------------------------+
    /// | transform track data        |
    /// |                             |
    /// |   position                  |
    /// |   rotation                  |
    /// |   alignment                 |
    /// |   scale                     |
    /// |                             |
    /// |   ... next track ...        |
    /// +-----------------------------+
    /// ```
    ///
    /// The masks must be read first because they determine the representation
    /// and therefore the number of bytes belonging to every subsequent track.
    ///
    /// # Errors
    ///
    /// Returns [`SplineError::UnexpectedEof`] if the block ends before the
    /// declared data is available. Other [`SplineError`] variants indicate
    /// malformed mask, quantization, spline, or track data.
    pub fn decode(
        data: &[u8],
        num_tracks: usize,
        num_float_tracks: usize,
    ) -> core::result::Result<Self, SplineError> {
        // Every transform track begins with a four-byte mask.
        num_tracks
            .checked_mul(size_of::<TransformMask>())
            .ok_or(SplineError::UnexpectedEof)?;

        let mut reader = Reader::new(data);

        let mut masks = Vec::with_capacity(num_tracks);

        for _ in 0..num_tracks {
            masks.push(read_transform_mask(&mut reader)?);
        }

        // Float tracks are a separate region. This decoder currently does not
        // expose them as transform tracks, but they still occupy bytes in the
        // binary and must therefore be skipped before decoding transforms.
        reader.skip(num_float_tracks)?;
        reader.align(4)?; // The first transform track starts on a four-byte boundary.

        let mut tracks = Vec::with_capacity(num_tracks);
        for mask in masks.iter().copied() {
            tracks.push(read_transform_track(&mut reader, mask)?);
        }

        Ok(Self { masks, tracks })
    }

    /// Evaluates one transform track at the specified local frame.
    ///
    /// # Errors
    /// If not found track_id
    pub fn get_value(
        &self,
        track_id: usize,
        time: f32,
    ) -> core::result::Result<QsTransform, SplineError> {
        let track = self
            .tracks
            .get(track_id)
            .ok_or(SplineError::TrackOutOfRange)?;

        let transition = match &track.position {
            SplineTrackVector::Static(track) => track.value.clone(),
            SplineTrackVector::Dynamic(track) => evaluate_vector_track(track, time)?,
        };

        let rotation = match &track.rotation {
            SplineTrackQuat::Static(track) => track.value,
            SplineTrackQuat::Dynamic(track) => evaluate_quat_track(track, time)?,
            SplineTrackQuat::Identity => QuatA16::identity(),
        };

        let scale = match &track.scale {
            SplineTrackVector::Static(track) => track.value.clone(),
            SplineTrackVector::Dynamic(track) => evaluate_vector_track(track, time)?,
        };

        Ok(QsTransform {
            transition,
            quaternion: Quaternion::from(rotation),
            scale,
        })
    }
}

/// A decompressed Havok spline animation.
#[derive(Clone, Debug, Default)]
pub struct SplineDecompressor {
    pub blocks: Vec<TransformSplineBlock>,
}

impl SplineDecompressor {
    /// Creates an empty spline decompressor.
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes all spline blocks from their block offsets.
    ///
    /// # Errors
    /// Unexpected data.
    pub fn decode(
        data: &[u8],
        block_offsets: &[u32],
        num_tracks: usize,
        num_float_tracks: usize,
    ) -> core::result::Result<Self, SplineError> {
        if block_offsets.is_empty() {
            return Ok(Self { blocks: Vec::new() });
        }

        let mut blocks = Vec::with_capacity(block_offsets.len());

        for &offset in block_offsets {
            let offset = offset as usize;

            let block_data = data.get(offset..).ok_or(SplineError::UnexpectedEof)?;

            blocks.push(TransformSplineBlock::decode(
                block_data,
                num_tracks,
                num_float_tracks,
            )?);
        }

        Ok(Self { blocks })
    }

    /// Evaluates one track in one decompressed block.
    ///
    /// # Errors
    /// Out of range block id
    pub fn get_value(
        &self,
        block_id: usize,
        track_id: usize,
        time: f32,
    ) -> core::result::Result<QsTransform, SplineError> {
        self.blocks
            .get(block_id)
            .ok_or(SplineError::TrackOutOfRange)?
            .get_value(track_id, time)
    }
}
