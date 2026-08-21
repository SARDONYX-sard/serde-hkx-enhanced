// SPDX-License-Identifier: GPL-3.0-or-later

//! Parsers for Havok spline binary representations.

use winnow::{
    Parser,
    binary::{self, Endianness},
    combinator::seq,
    error::{ContextError, ErrMode, StrContext::*, StrContextValue::*},
};

use crate::spline::{
    de::packed::{Quat32, Quat40, Quat48},
    math::{QuatA16, TransformMask},
};

type Error = ErrMode<ContextError>;

/// Parses a packed 32-bit quaternion.
///
/// # Errors
///
/// Returns a [`winnow`] parser error when four bytes are not available.
#[inline]
pub fn quat32<'a>(endian: Endianness) -> impl Parser<&'a [u8], Quat32, Error> {
    binary::u32(endian)
        .map(Quat32::new)
        .context(Label("quat32: u32"))
}

/// Parses a packed 40-bit quaternion.
///
/// # Errors
///
/// Returns a [`winnow`] parser error when five bytes are not available.
#[inline]
pub fn quat40<'a>() -> impl Parser<&'a [u8], Quat40, Error> {
    #[allow(clippy::tuple_array_conversions)]
    (binary::u8, binary::u8, binary::u8, binary::u8, binary::u8)
        .map(|(b0, b1, b2, b3, b4)| Quat40::new([b0, b1, b2, b3, b4]))
        .context(Label("quat40"))
}

/// Parses a packed 48-bit quaternion.
///
/// # Errors
///
/// Returns a [`winnow`] parser error when six bytes are not available.
#[inline]
pub fn uncompressed_quat<'a>(endian: Endianness) -> impl Parser<&'a [u8], QuatA16, Error> {
    move |input: &mut &'a [u8]| {
        let x = binary::f32(endian)
            .context(Expected(Description("x: f32")))
            .parse_next(input)?;
        let y = binary::f32(endian)
            .context(Expected(Description("y: f32")))
            .parse_next(input)?;
        let z = binary::f32(endian)
            .context(Expected(Description("z: f32")))
            .parse_next(input)?;
        let w = binary::f32(endian)
            .context(Expected(Description("w: f32")))
            .parse_next(input)?;

        Ok(QuatA16::new(x, y, z, w))
    }
}

/// Parses a packed 48-bit quaternion.
///
/// # Errors
///
/// Returns a [`winnow`] parser error when six bytes are not available.
#[inline]
pub fn quat48<'a>(endian: Endianness) -> impl Parser<&'a [u8], Quat48, Error> {
    seq! {
        Quat48 {
            x: binary::u16(endian).context(Expected(Description("x: u16"))),
            y: binary::u16(endian).context(Expected(Description("y: u16"))),
            z: binary::u16(endian).context(Expected(Description("z: u16"))),
        }
    }
    .context(Label("quat48"))
}

/// Parses the four-byte transform mask.
///
/// The four bytes are intentionally retained as the existing
/// [`TransformMask`] representation. Interpretation of individual fields is
/// handled by the mask type rather than by this byte parser.
///
/// # Errors
///
/// Returns a [`winnow`] parser error when four bytes are not available.
#[inline]
pub fn transform_mask<'a>() -> impl Parser<&'a [u8], TransformMask, Error> {
    seq! {
        TransformMask {
            quantization_types: binary::u8.context(Expected(Description("quantization_types: u8"))),
            position_types: binary::u8.context(Expected(Description("position_types: u8"))),
            rotation_types: binary::u8.context(Expected(Description("rotation_types: u8"))),
            scale_types: binary::u8.context(Expected(Description("scale_types: u8"))),
        }
    }
    .context(Label("transform_mask"))
}
