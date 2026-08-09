// SPDX-FileCopyrightText: (C) 2016-2023 Lukas Cone
// SPDX-License-Identifier: GPL-3.0
//
// ref: https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.cpp
// ref: https://github.com/lennart99v/SageHavokEditor/blob/988ee1c25e3fe8f35b8ac43d4494331156b16147/SkyrimHavokEditor/Core/Animation/HavokSplineDecoder.cs

//! Spline-compressed animation block deserializer *and* evaluator.
//!
//! Unlike a plain deserializer, this module fully evaluates the decoded
//! curves: the public entry point returns one [`QsTransform`] per frame per
//! track, ready to sample directly, rather than raw control points.
//!
//! # Binary layout (one block)
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │  QsTransformMask × num_tracks  (4 bytes each)                 │
//! │  Float-track region (opaque, num_float_tracks bytes)        │
//! │  pad to 4                                                   │
//! │  ┌──────────────────────────────────────────────────────┐   │
//! │  │ Track i                                              │   │
//! │  │  [Position section]  pad to 4 after                  │   │
//! │  │  [Rotation section]  pad to 4 after                  │   │
//! │  │  [Scale   section]   pad to 4 after                  │   │
//! │  └──────────────────────────────────────────────────────┘   │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! This decoder handles exactly one block (as does the reference C#
//! implementation this was ported from). Multi-block animations should
//! call [`read_spline_compressed_block`] once per block, aligning the
//! remaining input to 16 bytes between calls if the blocks are packed
//! contiguously.
//!
//! # QsTransformMask (4 bytes)
//!
//! ```text
//! Byte 0 – quantization types
//!   bits [1:0]  PositionQuantizationType   (0=BITS8, 1=BITS16)
//!   bits [5:2]  RotationQuantizationType   (0=POLAR32 … 5=UNCOMPRESSED)
//!   bits [7:6]  ScaleQuantizationType      (0=BITS8, 1=BITS16)
//! Byte 1 – position  FlagOffset bitmask
//! Byte 2 – rotation  FlagOffset bitmask
//! Byte 3 – scale     FlagOffset bitmask
//! ```
//!
//! # FlagOffset bitmask
//!
//! ```text
//! bit 0  STATIC_X    bit 4  SPLINE_X
//! bit 1  STATIC_Y    bit 5  SPLINE_Y
//! bit 2  STATIC_Z    bit 6  SPLINE_Z
//! bit 3  STATIC_W    bit 7  SPLINE_W
//! ```
//!
//! # Float-track region
//!
//! Immediately after the QsTransformMask array (before the 4-byte padding)
//! sits a region of `num_float_tracks` raw bytes. These belong to Havok's
//! separate "float slot" animation tracks, which are distinct from the
//! qsTransform (position/rotation/scale) tracks handled by this module.
//!
//! This parser only reserves (skips) that region so that the offsets of the
//! following qsTransform-track data stay correct; it does not decode the
//! float tracks' contents. The upstream reference implementation
//! (HavokLib/source/hka_spline_decompressor.cpp) does the same – it also
//! only skips `numFloatTractks` bytes and marks float-track decoding itself
//! as `// TODO floats`.
//!
//! # Position / Scale section
//!
//! A position or scale section describes up to three axes (X, Y, Z). Static
//! and spline axes may be *mixed* within one section — e.g. X animated via
//! spline while Y and Z stay static — so STATIC values and SPLINE bounds
//! are read inline, in axis order, rather than as two mutually-exclusive
//! layouts:
//!
//! ```text
//! if any SPLINE_* bit is set:
//!     u16   num_items          (= control-point count − 1)
//!     u8    degree             (typically 3 = cubic B-spline)
//!     u8[]  knots              (num_items + degree + 2 bytes)
//!     pad to 4
//!
//! for each axis in {X, Y, Z}:
//!     if STATIC_<axis> set:  f32  <axis> value (constant for every frame)
//!     else if SPLINE_<axis> set:
//!         f32  bounds_min
//!         f32  bounds_max
//!     else: value is 0.0 for every frame
//!
//! if any SPLINE_* bit is set:
//!     quantized control points, interleaved per control point:
//!       for cp in 0..=num_items:
//!         for each SPLINE axis (in X, Y, Z order):
//!           u8 (BITS8) or u16-LE (BITS16)
//! ```
//!
//! # Rotation section
//!
//! Unlike position/scale, rotation is a single quantized quaternion per
//! control point (not per-axis scalars):
//!
//! ```text
//! if any SPLINE_* bit set:
//!     u16   num_items
//!     u8    degree
//!     u8[]  knots  (num_items + degree + 2 bytes)
//!     pad to rotation_align (type-dependent, see table below)
//!     Quaternion × (num_items + 1)  (encoding = RotationQuantizationType)
//! else if any STATIC_* bit set:
//!     pad to rotation_align
//!     Quaternion × 1
//! else:
//!     (no data) — identity rotation for every frame
//! ```
//!
//! ## RotationQuantizationType sizes / alignments
//!
//! | Variant      | bytes | align |
//! |--------------|-------|-------|
//! | POLAR32      |   4   |   4   |
//! | THREECOMP40  |   5   |   1   |
//! | THREECOMP48  |   6   |   2   |
//! | THREECOMP24  |   3   |   1   | ← not implemented
//! | STRAIGHT16   |   2   |   2   | ← not implemented
//! | UNCOMPRESSED |  16   |   4   |
//!
//! # Curve evaluation
//!
//! `num_frames` is not stored in the binary data — it is supplied by the
//! caller (it comes from the enclosing animation's frame count) — and
//! every decoded curve is evaluated at every integer frame `0..num_frames`
//! using Cox-de Boor recursion, exactly as the reference C# decoder does:
//!
//! - `find_knot_span` locates, via binary search over the (byte-valued)
//!   knot vector, the span `[knots[span], knots[span+1])` containing the
//!   target frame.
//! - Position/scale axes de Boor-recurse over plain `f32` control points
//!   (linear interpolation at each recursion step).
//! - Rotation de Boor-recurses over quaternion control points using
//!   **slerp** instead of linear interpolation, with a shortest-path
//!   dot-product sign flip applied before each slerp step.
//!
//! Static axes/quaternions are simply held constant across all frames;
//! axes with neither a STATIC nor a SPLINE bit set evaluate to `0.0`.
//!
//! # Quaternion encodings
//!
//! - **POLAR32**: a single packed `u32`. Bits `[17:0]` encode a polar
//!   angle pair, bits `[27:18]` a 10-bit magnitude term, and bits
//!   `[31:28]` the sign of each of X/Y/Z/W.
//! - **THREECOMP40**: 5 bytes. Three 12-bit unsigned components (bits
//!   `[11:0]`, `[23:12]`, `[35:24]`) dequantized onto
//!   `[-1/√2, 1/√2]`, a 2-bit `result_shift` (bits `[37:36]`, the
//!   quaternion slot reconstructed from the unit-length constraint), and a
//!   1-bit sign for that reconstructed component (bit 38). Ported from the
//!   validated reference `DecodeTC40`.
//! - **THREECOMP48**: 3 × `i16`. The top bit(s) of each `i16` encode
//!   `result_shift` and the reconstructed component's sign; the low 15
//!   bits of each encode a signed, biased component.
//! - **UNCOMPRESSED**: 4 × `f32` (x, y, z, w) directly.
//! - **THREECOMP24** / **STRAIGHT16** are not implemented upstream and are
//!   rejected here with a parse error.

#![allow(clippy::needless_range_loop)]

use havok_classes::{Classes, hkaSplineCompressedAnimation};
use havok_types::{QsTransform, Quaternion, Vector4};
use serde_hkx_features::{ClassMap, Result};
use winnow::binary::{le_f32, le_i16, le_u16, le_u32, u8};
use winnow::combinator::fail;
use winnow::error::{StrContext, StrContextValue};
use winnow::prelude::*;
use winnow::token::take;
use winnow_ext::ReadableError;

use crate::spline::bail;
use crate::spline::skeleton::{
    AnimationAnnotation, AnimationClip, Skeleton, apply_skeleton, find_track_to_bone,
    into_skeleton, transpose_tracks,
};

use super::{FlagOffset, RotationQuantizationType, ScalarQuantizationType};

type Input<'a> = &'a [u8];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Advance `input` forward until its address is a multiple of `align`.
///
/// Uses the virtual address of the slice pointer, which matches what
/// `BinaryReaderEx.Pad()` does in the C# reference implementation.
#[inline]
fn pad_to_abs(input: &mut Input, align: usize) {
    let addr = input.as_ptr() as usize;
    let mis = addr % align;
    if mis != 0 {
        *input = &input[align - mis..];
    }
}

fn read_quantized_float(
    input: &mut Input,
    min: f32,
    max: f32,
    qt: ScalarQuantizationType,
) -> ModalResult<f32> {
    let ratio = match qt {
        ScalarQuantizationType::Bits8 => u8.parse_next(input)? as f32 / 255.0,
        ScalarQuantizationType::Bits16 => le_u16.parse_next(input)? as f32 / 65535.0,
    };
    Ok((max - min).mul_add(ratio, min))
}

/// Find the B-spline knot span containing parameter `t` (an integer frame
/// index, expressed as `f32` for the comparisons below).
///
/// `n` is `num_control_points - 1` and `degree` is the curve degree. Ported
/// verbatim from the reference `FindKnotSpan` (binary search over the knot
/// vector, clamped to the valid `[degree, n]` range).
fn find_knot_span(knots: &[u8], t: f32, n: usize, degree: usize) -> usize {
    if t >= knots[n + 1] as f32 {
        return n;
    }
    if t <= knots[0] as f32 {
        return degree;
    }

    let (mut lo, mut hi) = (degree, n);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if (knots[mid] as f32) <= t {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo.clamp(degree, n)
}

// ---------------------------------------------------------------------------
// QsTransformMask
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct QsTransformMask {
    pub pos_q: ScalarQuantizationType,
    pub rot_q: RotationQuantizationType,
    pub scale_q: ScalarQuantizationType,
    pub pos_flags: FlagOffset,
    pub rot_flags: FlagOffset,
    pub scale_flags: FlagOffset,
}

fn parse_qs_transform_mask(input: &mut Input) -> ModalResult<QsTransformMask> {
    let qt = u8
        .context(StrContext::Expected(StrContextValue::Description(
            "QsTransformMask byte 0: quantization types (pos[1:0] rot[5:2] scale[7:6])",
        )))
        .parse_next(input)?;
    let pos_byte = u8
        .context(StrContext::Expected(StrContextValue::Description(
            "QsTransformMask byte 1: position FlagOffset bitmask",
        )))
        .parse_next(input)?;
    let rot_byte = u8
        .context(StrContext::Expected(StrContextValue::Description(
            "QsTransformMask byte 2: rotation FlagOffset bitmask",
        )))
        .parse_next(input)?;
    let scale_byte = u8
        .context(StrContext::Expected(StrContextValue::Description(
            "QsTransformMask byte 3: scale FlagOffset bitmask",
        )))
        .parse_next(input)?;

    Ok(QsTransformMask {
        pos_q: match qt & 0x3 {
            0 => ScalarQuantizationType::Bits8,
            _ => ScalarQuantizationType::Bits16,
        },
        rot_q: match (qt >> 2) & 0xF {
            0 => RotationQuantizationType::Polar32,
            1 => RotationQuantizationType::ThreeComp40,
            2 => RotationQuantizationType::ThreeComp48,
            3 => RotationQuantizationType::ThreeComp24,
            4 => RotationQuantizationType::Straight16,
            _ => RotationQuantizationType::Uncompressed,
        },
        scale_q: match (qt >> 6) & 0x3 {
            0 => ScalarQuantizationType::Bits8,
            _ => ScalarQuantizationType::Bits16,
        },
        pos_flags: FlagOffset::from_bits_truncate(pos_byte),
        rot_flags: FlagOffset::from_bits_truncate(rot_byte),
        scale_flags: FlagOffset::from_bits_truncate(scale_byte),
    })
}

// ---------------------------------------------------------------------------
// Raw quaternion (internal math representation)
// ---------------------------------------------------------------------------

/// Internal quaternion representation used for every decode/eval step.
///
/// Kept separate from [`havok_types::Quaternion`] because the de Boor /
/// slerp evaluation below needs to read back `x`/`y`/`z`/`w` (for dot
/// products and sign flips), which the public type is not assumed to
/// expose. Converted to [`havok_types::Quaternion`] only once, at the very
/// end of decoding (see [`build_qsTransform`]).
#[derive(Debug, Clone, Copy)]
struct RawQuat {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

impl RawQuat {
    const IDENTITY: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    fn dot(self, rhs: Self) -> f32 {
        self.x.mul_add(
            rhs.x,
            self.y.mul_add(rhs.y, self.z.mul_add(rhs.z, self.w * rhs.w)),
        )
    }

    fn negated(self) -> Self {
        Self::new(-self.x, -self.y, -self.z, -self.w)
    }

    /// Spherical linear interpolation, matching the reference `Slerp`
    /// (including its linear-interpolation fallback for near-parallel
    /// quaternions, to avoid dividing by ~0).
    fn slerp(a: Self, b: Self, t: f32) -> Self {
        let dot = a.dot(b).clamp(-1.0, 1.0);
        let theta = dot.abs().acos();

        if theta < 1e-6 {
            let rx = t.mul_add(b.x - a.x, a.x);
            let ry = t.mul_add(b.y - a.y, a.y);
            let rz = t.mul_add(b.z - a.z, a.z);
            let rw = t.mul_add(b.w - a.w, a.w);
            let len = rw
                .mul_add(rw, rz.mul_add(rz, ry.mul_add(ry, rx * rx)))
                .sqrt();
            return if len > 0.0 {
                Self::new(rx / len, ry / len, rz / len, rw / len)
            } else {
                Self::IDENTITY
            };
        }

        let sin_t = theta.sin();
        let wa = ((1.0 - t) * theta).sin() / sin_t;
        let mut wb = (t * theta).sin() / sin_t;
        if dot < 0.0 {
            wb = -wb;
        }
        Self::new(
            wa * a.x + wb * b.x,
            wa * a.y + wb * b.y,
            wa * a.z + wb * b.z,
            wa * a.w + wb * b.w,
        )
    }
}

/// Scatter three decoded components into a 4-element quaternion, skipping
/// the reconstructed (largest-magnitude) slot at index `skip`.
#[inline]
fn scatter3_to4(a: f32, b: f32, c: f32, skip: usize) -> [f32; 4] {
    let src = [a, b, c];
    let mut out = [0.0_f32; 4];
    let mut si = 0;
    for i in 0..4 {
        if i != skip {
            out[i] = src[si];
            si += 1;
        }
    }
    out
}

/// Decode a POLAR32-packed quaternion (see module docs for the bit layout).
fn read_quat_polar32(input: &mut Input) -> ModalResult<RawQuat> {
    use core::f32::consts::{FRAC_PI_2, FRAC_PI_4};

    let c = le_u32
        .context(StrContext::Expected(StrContextValue::Description(
            "POLAR32 quaternion: u32 packed value",
        )))
        .parse_next(input)?;

    const R_MASK: u32 = (1 << 10) - 1;
    const R_FRAC: f32 = 1.0 / R_MASK as f32;
    let r_raw = ((c >> 18) & R_MASK) as f32 * R_FRAC;
    let mut w = 1.0 - r_raw * r_raw;

    let phi_theta = (c & 0x0003_FFFF) as f32;
    let phi_int = phi_theta.sqrt().floor();
    let (theta, phi_rad) = if phi_int > 0.0 {
        let t = FRAC_PI_4 * (phi_theta - phi_int * phi_int) / phi_int;
        (t, phi_int * (FRAC_PI_2 / 511.0))
    } else {
        (0.0_f32, 0.0_f32)
    };

    let mag = (1.0 - w * w).sqrt();
    let mut x = phi_rad.sin() * theta.cos() * mag;
    let mut y = phi_rad.sin() * theta.sin() * mag;
    let mut z = phi_rad.cos() * mag;

    if c & 0x1000_0000 != 0 {
        x = -x;
    }
    if c & 0x2000_0000 != 0 {
        y = -y;
    }
    if c & 0x4000_0000 != 0 {
        z = -z;
    }
    if c & 0x8000_0000 != 0 {
        w = -w;
    }

    Ok(RawQuat::new(x, y, z, w))
}

/// Decode a THREECOMP48-packed quaternion (3 × `i16`).
fn read_quat_three_comp48(input: &mut Input) -> ModalResult<RawQuat> {
    // (1 << 15) - 1 overflows i16, so write the bit pattern directly.
    const MASK: i16 = 0b0111_1111_1111_1111; // 0x7FFF
    const FRACTAL: f32 = 0.000_043_161;

    let raw_x = le_i16
        .context(StrContext::Expected(StrContextValue::Description(
            "THREECOMP48 i16 x",
        )))
        .parse_next(input)?;
    let raw_y = le_i16
        .context(StrContext::Expected(StrContextValue::Description(
            "THREECOMP48 i16 y",
        )))
        .parse_next(input)?;
    let raw_z = le_i16
        .context(StrContext::Expected(StrContextValue::Description(
            "THREECOMP48 i16 z",
        )))
        .parse_next(input)?;

    let result_shift = (((raw_y >> 14) & 0x2) | ((raw_x >> 15) & 0x1)) as usize;
    let r_sign = (raw_z >> 15) != 0;

    let fx = ((raw_x & MASK) - (MASK >> 1)) as f32 * FRACTAL;
    let fy = ((raw_y & MASK) - (MASK >> 1)) as f32 * FRACTAL;
    let fz = ((raw_z & MASK) - (MASK >> 1)) as f32 * FRACTAL;

    let mut out = scatter3_to4(fx, fy, fz, result_shift);
    let mut w = (1.0 - (fx * fx + fy * fy + fz * fz)).max(0.0).sqrt();
    if r_sign {
        w = -w;
    }
    out[result_shift] = w;

    Ok(RawQuat::new(out[0], out[1], out[2], out[3]))
}

/// Decode a THREECOMP40-packed quaternion (5 bytes).
///
/// Ported from the validated reference decoder (`DecodeTC40` in
/// `HavokSplineDecoder.cs`): three 12-bit unsigned components plus a 2-bit
/// `result_shift` (which of the four quaternion slots was omitted, i.e.
/// reconstructed from the unit-length constraint) and a 1-bit sign for that
/// reconstructed component. Each 12-bit component is dequantized onto
/// `[-1/√2, 1/√2]`.
fn read_quat_three_comp40(input: &mut Input) -> ModalResult<RawQuat> {
    let b = take(5_usize)
        .context(StrContext::Expected(StrContextValue::Description(
            "THREECOMP40 quaternion: 5 packed bytes",
        )))
        .parse_next(input)?;

    let va = b[0] as u32 | (((b[1] & 0xF) as u32) << 8);
    let vb = ((b[1] >> 4) & 0xF) as u32 | ((b[2] as u32) << 4);
    let vc = b[3] as u32 | (((b[4] & 0xF) as u32) << 8);
    let result_shift = ((b[4] >> 4) & 0x3) as usize;
    let sign = (b[4] >> 6) & 0x1 != 0;

    const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;
    let dequant = |q: u32| -> f32 { (q as f32 / 4095.0).mul_add(2.0 * INV_SQRT2, -INV_SQRT2) };

    let s = [dequant(va), dequant(vb), dequant(vc)];
    let sum_sq = s[2].mul_add(s[2], s[1].mul_add(s[1], s[0] * s[0]));
    let mut recon = (1.0 - sum_sq).max(0.0).sqrt();
    if sign {
        recon = -recon;
    }

    let mut out = scatter3_to4(s[0], s[1], s[2], result_shift);
    out[result_shift] = recon;

    Ok(RawQuat::new(out[0], out[1], out[2], out[3]))
}

fn read_quantized_quat(input: &mut Input, qt: RotationQuantizationType) -> ModalResult<RawQuat> {
    match qt {
        RotationQuantizationType::Polar32 => read_quat_polar32(input),
        RotationQuantizationType::ThreeComp40 => read_quat_three_comp40(input),
        RotationQuantizationType::ThreeComp48 => read_quat_three_comp48(input),
        RotationQuantizationType::Uncompressed => {
            let x = le_f32
                .context(StrContext::Expected(StrContextValue::Description(
                    "Uncompressed quat x",
                )))
                .parse_next(input)?;
            let y = le_f32
                .context(StrContext::Expected(StrContextValue::Description(
                    "Uncompressed quat y",
                )))
                .parse_next(input)?;
            let z = le_f32
                .context(StrContext::Expected(StrContextValue::Description(
                    "Uncompressed quat z",
                )))
                .parse_next(input)?;
            let w = le_f32
                .context(StrContext::Expected(StrContextValue::Description(
                    "Uncompressed quat w",
                )))
                .parse_next(input)?;
            Ok(RawQuat::new(x, y, z, w))
        }
        RotationQuantizationType::ThreeComp24 | RotationQuantizationType::Straight16 => fail
            .context(StrContext::Expected(StrContextValue::Description(
                "unsupported RotationQuantizationType (ThreeComp24 / Straight16)",
            )))
            .parse_next(input),
    }
}

// ---------------------------------------------------------------------------
// Vec3 (position / scale) curve: parse + evaluate
// ---------------------------------------------------------------------------

/// One axis of a position/scale curve, already dequantized but not yet
/// evaluated at any particular frame.
#[derive(Debug)]
enum AxisCurve {
    /// Neither a STATIC nor a SPLINE bit was set for this axis: the value
    /// is `0.0` for every frame.
    Zero,
    /// A STATIC bit was set: the value is constant for every frame.
    Static(f32),
    /// A SPLINE bit was set: `values` holds one dequantized control point
    /// per knot (`num_items + 1` entries), evaluated per-frame via
    /// [`eval_scalar_over_frames`].
    Spline(Vec<f32>),
}

#[derive(Debug)]
struct Vec3Section<'a> {
    degree: u8,
    knots: &'a [u8],
    axes: [AxisCurve; 3],
}

/// Parse one position or scale section.
///
/// Static and spline axes may be mixed within a single section (e.g. X
/// animated via spline while Y/Z stay static) — the reference decoder reads
/// STATIC values and SPLINE bounds inline, in axis order, rather than
/// treating "static" and "spline" as separate mutually-exclusive layouts.
fn parse_vec3_section<'a>(
    input: &mut Input<'a>,
    flags: FlagOffset,
    qt: ScalarQuantizationType,
) -> ModalResult<Vec3Section<'a>> {
    const STATIC_BITS: [FlagOffset; 3] = [
        FlagOffset::STATIC_X,
        FlagOffset::STATIC_Y,
        FlagOffset::STATIC_Z,
    ];
    const SPLINE_BITS: [FlagOffset; 3] = [
        FlagOffset::SPLINE_X,
        FlagOffset::SPLINE_Y,
        FlagOffset::SPLINE_Z,
    ];

    let any_spline =
        flags.intersects(FlagOffset::SPLINE_X | FlagOffset::SPLINE_Y | FlagOffset::SPLINE_Z);

    let (degree, knots, num_items) = if any_spline {
        let num_items = le_u16
            .context(StrContext::Expected(StrContextValue::Description(
                "Vec3 curve num_items (u16, = control_points - 1)",
            )))
            .parse_next(input)? as usize;
        let degree = u8
            .context(StrContext::Expected(StrContextValue::Description(
                "Vec3 curve degree (u8, typically 3 for cubic B-spline)",
            )))
            .parse_next(input)?;
        let knot_count = num_items + degree as usize + 2;
        let knots = take(knot_count)
            .context(StrContext::Expected(StrContextValue::Description(
                "Vec3 curve knot vector (num_items + degree + 2 bytes)",
            )))
            .parse_next(input)?;
        pad_to_abs(input, 4);
        (degree, knots, num_items)
    } else {
        (0, &[][..], 0)
    };

    // Bounds (for spline axes) and static values are read inline, in axis
    // order, exactly as the reference decoder does.
    let mut bounds: [(f32, f32); 3] = [(0.0, 0.0); 3];
    let mut is_spline_axis = [false; 3];
    let mut axes: [AxisCurve; 3] = [AxisCurve::Zero, AxisCurve::Zero, AxisCurve::Zero];

    for i in 0..3 {
        if flags.contains(STATIC_BITS[i]) {
            let v = le_f32
                .context(StrContext::Expected(StrContextValue::Description(
                    "static axis value (f32)",
                )))
                .parse_next(input)?;
            axes[i] = AxisCurve::Static(v);
        } else if flags.contains(SPLINE_BITS[i]) {
            let min = le_f32
                .context(StrContext::Expected(StrContextValue::Description(
                    "spline axis bounds_min",
                )))
                .parse_next(input)?;
            let max = le_f32
                .context(StrContext::Expected(StrContextValue::Description(
                    "spline axis bounds_max",
                )))
                .parse_next(input)?;
            bounds[i] = (min, max);
            is_spline_axis[i] = true;
            axes[i] = AxisCurve::Spline(Vec::with_capacity(num_items + 1));
        }
    }

    if any_spline {
        for _ in 0..=num_items {
            for i in 0..3 {
                if is_spline_axis[i] {
                    let (min, max) = bounds[i];
                    let v = read_quantized_float(input, min, max, qt)?;
                    if let AxisCurve::Spline(values) = &mut axes[i] {
                        values.push(v);
                    }
                }
            }
        }
    }

    Ok(Vec3Section {
        degree,
        knots,
        axes,
    })
}

/// Evaluate one scalar B-spline axis at every frame `0..num_frames`, using
/// Cox-de Boor recursion. Ported verbatim from the reference
/// `EvalComponent` / `DecodeVecCurve` loop.
fn eval_scalar_over_frames(
    values: &[f32],
    knots: &[u8],
    degree: u8,
    num_frames: usize,
) -> Vec<f32> {
    let n = values.len() - 1;
    let deg = degree as usize;
    let mut d = vec![0.0_f32; deg + 1];
    let mut out = Vec::with_capacity(num_frames);

    for frame in 0..num_frames {
        let t = frame as f32;
        let span = find_knot_span(knots, t, n, deg);

        for j in 0..=deg {
            let ci = (span - deg + j).min(n);
            d[j] = values[ci];
        }

        for r in 1..=deg {
            for j in (r..=deg).rev() {
                let klo = knots[j + span - deg] as f32;
                let khi = knots[j + span - r + 1] as f32;
                let a = if khi > klo {
                    ((t - klo) / (khi - klo)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                d[j] = (1.0 - a).mul_add(d[j - 1], a * d[j]);
            }
        }

        out.push(d[deg]);
    }

    out
}

/// Evaluate one axis of a [`Vec3Section`] at every frame.
fn eval_axis(axis: &AxisCurve, knots: &[u8], degree: u8, num_frames: usize) -> Vec<f32> {
    match axis {
        AxisCurve::Zero => vec![0.0; num_frames],
        AxisCurve::Static(v) => vec![*v; num_frames],
        AxisCurve::Spline(values) => eval_scalar_over_frames(values, knots, degree, num_frames),
    }
}

fn eval_vec3_section(section: &Vec3Section, num_frames: usize) -> [Vec<f32>; 3] {
    [
        eval_axis(&section.axes[0], section.knots, section.degree, num_frames),
        eval_axis(&section.axes[1], section.knots, section.degree, num_frames),
        eval_axis(&section.axes[2], section.knots, section.degree, num_frames),
    ]
}

// ---------------------------------------------------------------------------
// Rotation curve: parse + evaluate
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum RotationCurve<'a> {
    /// No STATIC_* or SPLINE_* bit set at all: identity for every frame.
    Identity,
    /// Any STATIC_* bit set (and no SPLINE_* bit): one constant quaternion.
    Static(RawQuat),
    /// Any SPLINE_* bit set: `values` holds one quaternion per knot
    /// (`num_items + 1` entries), evaluated per-frame via
    /// [`eval_quat_over_frames`].
    Spline {
        degree: u8,
        knots: &'a [u8],
        values: Vec<RawQuat>,
    },
}

fn parse_rotation_section<'a>(
    input: &mut Input<'a>,
    flags: FlagOffset,
    qt: RotationQuantizationType,
) -> ModalResult<RotationCurve<'a>> {
    if flags.intersects(
        FlagOffset::SPLINE_X | FlagOffset::SPLINE_Y | FlagOffset::SPLINE_Z | FlagOffset::SPLINE_W,
    ) {
        let num_items = le_u16
            .context(StrContext::Expected(StrContextValue::Description(
                "SplineRotation num_items (u16)",
            )))
            .parse_next(input)? as usize;
        let degree = u8
            .context(StrContext::Expected(StrContextValue::Description(
                "SplineRotation degree (u8)",
            )))
            .parse_next(input)?;
        let knot_count = num_items + degree as usize + 2;
        let knots = take(knot_count)
            .context(StrContext::Expected(StrContextValue::Description(
                "SplineRotation knot vector (num_items + degree + 2 bytes)",
            )))
            .parse_next(input)?;

        let Some(align) = qt.rotation_align() else {
            return fail
                .context(StrContext::Expected(StrContextValue::Description(
                    "SplineRotation: unsupported RotationQuantizationType \
                     (ThreeComp24 / Straight16 not implemented)",
                )))
                .parse_next(input);
        };
        pad_to_abs(input, align);

        let mut values = Vec::with_capacity(num_items + 1);
        for _ in 0..=num_items {
            values.push(read_quantized_quat(input, qt)?);
        }

        Ok(RotationCurve::Spline {
            degree,
            knots,
            values,
        })
    } else if flags.intersects(
        FlagOffset::STATIC_X | FlagOffset::STATIC_Y | FlagOffset::STATIC_Z | FlagOffset::STATIC_W,
    ) {
        let Some(align) = qt.rotation_align() else {
            return fail
                .context(StrContext::Expected(StrContextValue::Description(
                    "StaticRotation: unsupported RotationQuantizationType \
                     (ThreeComp24 / Straight16 not implemented)",
                )))
                .parse_next(input);
        };
        pad_to_abs(input, align);
        Ok(RotationCurve::Static(read_quantized_quat(input, qt)?))
    } else {
        Ok(RotationCurve::Identity)
    }
}

/// Evaluate a rotation curve at every frame `0..num_frames`, using
/// Cox-de Boor recursion with slerp in place of linear interpolation (and a
/// shortest-path dot-product sign flip before each slerp step, exactly as
/// the reference `DecodeRotation` does).
fn eval_quat_over_frames(
    values: &[RawQuat],
    knots: &[u8],
    degree: u8,
    num_frames: usize,
) -> Vec<RawQuat> {
    let n = values.len() - 1;
    let deg = degree as usize;
    let mut d = vec![RawQuat::IDENTITY; deg + 1];
    let mut out = Vec::with_capacity(num_frames);

    for frame in 0..num_frames {
        let t = frame as f32;
        let span = find_knot_span(knots, t, n, deg);

        for j in 0..=deg {
            let ci = (span - deg + j).min(n);
            d[j] = values[ci];
        }

        for r in 1..=deg {
            for j in (r..=deg).rev() {
                let klo = knots[j + span - deg] as f32;
                let khi = knots[j + span - r + 1] as f32;
                let a = if khi > klo {
                    ((t - klo) / (khi - klo)).clamp(0.0, 1.0)
                } else {
                    0.0
                };

                if d[j - 1].dot(d[j]) < 0.0 {
                    d[j] = d[j].negated();
                }
                d[j] = RawQuat::slerp(d[j - 1], d[j], a);
            }
        }

        out.push(d[deg]);
    }

    out
}

fn eval_rotation_curve(curve: &RotationCurve, num_frames: usize) -> Vec<RawQuat> {
    match curve {
        RotationCurve::Identity => vec![RawQuat::IDENTITY; num_frames],
        RotationCurve::Static(q) => vec![*q; num_frames],
        RotationCurve::Spline {
            degree,
            knots,
            values,
        } => eval_quat_over_frames(values, knots, *degree, num_frames),
    }
}

// ---------------------------------------------------------------------------
// QsTransform assembly
// ---------------------------------------------------------------------------

/// Build a [`havok_types::QsTransform`] from separately-evaluated
/// translation / rotation / scale components.
///
/// NOTE: adjust the field names / constructors here if they differ from
/// the actual `havok_types::QsTransform` definition — this is the only place
/// in the module that touches its shape.
#[inline]
const fn build_qs_transform(t: (f32, f32, f32), r: RawQuat, s: (f32, f32, f32)) -> QsTransform {
    QsTransform {
        transition: Vector4::new(t.0, t.1, t.2, 0.0),
        quaternion: Quaternion::new(r.x, r.y, r.z, r.w),
        scale: Vector4::new(s.0, s.1, s.2, 0.0),
    }
}

// ---------------------------------------------------------------------------
// Per-track / per-block decoding
// ---------------------------------------------------------------------------

/// A stateful parser that tracks the current byte offset for diagnostics.
struct SplineParser<'a> {
    /// Pointer to the very first byte of the data buffer (never advanced).
    full: &'a [u8],
    /// Remaining input (advanced as bytes are consumed).
    input: &'a [u8],
    /// Most recently started track index (for diagnostics).
    current_track: usize,
}

impl<'a> SplineParser<'a> {
    const fn new(data: &'a [u8]) -> Self {
        Self {
            full: data,
            input: data,
            current_track: 0,
        }
    }

    fn current_position(&self) -> usize {
        self.input.as_ptr() as usize - self.full.as_ptr() as usize
    }

    fn pad_to(&mut self, align: usize) {
        let addr = self.input.as_ptr() as usize;
        let mis = addr % align;
        if mis != 0 {
            let skip = align - mis;
            // Guard against reading past the end of the buffer; this can
            // happen at the very end of the block.
            if skip <= self.input.len() {
                self.input = &self.input[skip..];
            } else {
                self.input = &self.input[self.input.len()..];
            }
        }
    }

    /// Parse and fully evaluate one track, producing `num_frames`
    /// [`QsTransform`]s.
    fn decode_track(
        &mut self,
        mask: &QsTransformMask,
        num_frames: usize,
    ) -> ModalResult<Vec<QsTransform>> {
        let pos = parse_vec3_section(&mut self.input, mask.pos_flags, mask.pos_q)?;
        self.pad_to(4);
        let rot = parse_rotation_section(&mut self.input, mask.rot_flags, mask.rot_q)?;
        self.pad_to(4);
        let scale = parse_vec3_section(&mut self.input, mask.scale_flags, mask.scale_q)?;
        self.pad_to(4);

        let [px, py, pz] = eval_vec3_section(&pos, num_frames);
        let [sx, sy, sz] = eval_vec3_section(&scale, num_frames);
        let rq = eval_rotation_curve(&rot, num_frames);

        Ok((0..num_frames)
            .map(|f| build_qs_transform((px[f], py[f], pz[f]), rq[f], (sx[f], sy[f], sz[f])))
            .collect())
    }

    /// Parse and fully evaluate one spline-compressed block.
    ///
    /// Returns one `Vec<QsTransform>` per track (outer index = track, inner
    /// index = frame, `0..num_frames`).
    fn decode_block(
        &mut self,
        num_tracks: usize,
        num_float_tracks: usize,
        num_frames: usize,
    ) -> ModalResult<Vec<Vec<QsTransform>>> {
        // Read all QsTransformMasks upfront (4 bytes each).
        let mut masks = Vec::with_capacity(num_tracks);
        for _ in 0..num_tracks {
            masks.push(parse_qs_transform_mask(&mut self.input)?);
        }

        // Skip the float-track region (opaque; see the module-level docs).
        // This must happen *before* the 4-byte padding below, exactly as in
        // the reference C++ (`buffer += sizeof(QsTransformMask) * numTracks +
        // numFloatTractks; ApplyPadding(buffer);`).
        if num_float_tracks > 0 {
            take(num_float_tracks)
                .context(StrContext::Expected(StrContextValue::Description(
                    "float-track region (opaque, not decoded)",
                )))
                .parse_next(&mut self.input)?;
        }
        self.pad_to(4);

        let mut tracks = Vec::with_capacity(num_tracks);
        for (track_idx, mask) in masks.iter().enumerate() {
            self.current_track = track_idx;
            tracks.push(self.decode_track(mask, num_frames)?);
        }

        Ok(tracks)
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// # Errors
pub fn de_spline_from_hkx_or_xml<'a, P, P2>(
    anim_bytes: &'a [u8],
    anim_path: P,
    skeleton_bytes: &'a [u8],
    skeleton_path: P2,
) -> Result<(AnimationClip, Skeleton)>
where
    P: AsRef<std::path::Path>,
    P2: AsRef<std::path::Path>,
{
    let skeleton = serde_hkx_features::convert::process_serde_with(
        skeleton_bytes,
        skeleton_path,
        into_skeleton,
        into_skeleton,
    )?;

    let f = |class_map: ClassMap| {
        let class = class_map
            .iter()
            .find(|class| matches!(class.1, Classes::hkaSplineCompressedAnimation(_)));
        let Some((_, Classes::hkaSplineCompressedAnimation(spline))) = class else {
            bail!("not found hkaSplineCompressedAnimation");
        };

        let duration = spline.parent.m_duration;
        let num_frames = spline.m_numFrames as usize;
        let num_blocks = spline.m_numBlocks;
        if num_blocks != 1 {
            bail!(format!("multi block animation unsupported: {}", num_blocks));
        }

        let mask_size = spline.m_maskAndQuantizationSize as usize;
        let num_tracks = spline.parent.m_numberOfTransformTracks as usize;
        let num_float_tracks = spline.parent.m_numberOfFloatTracks as usize;
        let data = spline.m_data.as_slice();

        let tracks = read_spline(data, num_frames, num_float_tracks, mask_size).map_err(|e| {
            serde_hkx_features::error::Error::DeError {
                input: std::path::PathBuf::from("test"),
                source: Box::new(serde_hkx_features::serde::de::DeError::Hkx {
                    source: serde_hkx::errors::de::Error::ReadableError { source: e },
                    location: snafu::location!(),
                }),
            }
        })?;

        let frame_tracks = transpose_tracks(&tracks, num_frames);
        let track_to_bone = find_track_to_bone(&class_map);

        let frames = apply_skeleton(
            frame_tracks,
            &skeleton,
            track_to_bone.as_deref(),
            num_tracks,
        );

        Ok(AnimationClip {
            duration,
            num_frames,
            num_tracks,
            frames,
            annotations: find_annotations(spline),
            track_count_exceeds_bones: num_tracks > skeleton.reference_pose.len(),
        })
    };

    Ok((
        serde_hkx_features::convert::process_serde_with(anim_bytes, anim_path, f, f)?,
        skeleton,
    ))
}

fn find_annotations(spline: &hkaSplineCompressedAnimation<'_>) -> Vec<AnimationAnnotation> {
    let mut result = Vec::new();

    for (track_index, track) in spline.parent.m_annotationTracks.iter().enumerate() {
        for annotation in track.m_annotations.iter() {
            let time = annotation.m_time;
            let text = &annotation.m_text;

            if !text.is_null() {
                result.push(AnimationAnnotation {
                    time,
                    text: text.to_string(),
                    track_index,
                });
            }
        }
    }

    result.sort_by(|a, b| {
        a.time
            .partial_cmp(&b.time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    result
}

fn read_spline(
    data: &[u8],
    num_frames: usize,
    num_float_tracks: usize,
    mask_size: usize,
) -> Result<Vec<Vec<QsTransform>>, ReadableError> {
    let num_tracks = mask_size / 4;

    let mut parser = SplineParser::new(data);
    parser
        .decode_block(num_tracks, num_float_tracks, num_frames)
        .map_err(|e| {
            let input_hex = serde_hkx::bytes::hexdump::to_string(parser.full);
            let err_pos = serde_hkx::bytes::hexdump::to_hexdump_pos(parser.current_position());
            ReadableError::from_context(e, input_hex, err_pos)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke-test against a captured spline block from a real `.hkx` file.
    #[test]
    #[ignore = "requires external test data + a known frame count"]
    fn decode_against_captured_block() {
        let (anim_path, skeleton_path) = {
            (
                "../../tests/output/xml/x64/meshes/actors/cow/animations/attack1.xml",
                "../../tests/output/xml/x64/meshes/actors/cow/character assets/skeleton.xml",
            )
            // (
            //     "../../tests/output/xml/x86/meshes/actors/character/animations/wall_idleshoulder.xml",
            //     "../../tests/output/xml/x86/meshes/actors/character/character assets/skeleton.xml"
            // )
        };
        let anim_bytes = std::fs::read(anim_path).expect("failed to read anim bytes");
        let skeleton_bytes = std::fs::read(skeleton_path).expect("failed to read skeleton bytes");
        let re = de_spline_from_hkx_or_xml(&anim_bytes, anim_path, &skeleton_bytes, skeleton_path)
            .unwrap_or_else(|e| panic!("{e}"));

        std::fs::create_dir_all("../../logs/").unwrap();
        std::fs::write("../../logs/data.debug.log", format!("{re:#?}")).unwrap();
    }
}
