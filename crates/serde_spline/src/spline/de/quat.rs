// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: (C) 2016-2023 Lukas Cone
//
// Reference:
// - https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.cpp

//! Quantized quaternion formats (32/40/48 bits per rotation).

use core::f32::consts::PI;

use rig_math::{IVec4A16, QuatA16, Vec4A16};

use super::reader::Reader;
use crate::error::Error;
use crate::spline::track::QuantizationType;

/// The 32-bit representation does not store four independent floats.
///
/// A normalized quaternion has `x² + y² + z² + w² = 1`, so only three
/// degrees of freedom are necessary. Havok packs the information needed to
/// reconstruct those values into a 32-bit word.
///
/// The bit fields contain:
///
/// - a magnitude-related value,
/// - angular information,
/// - sign bits for the reconstructed components.
///
/// The constants below are therefore part of the binary encoding and
/// should not be replaced with a generic "epsilon" or generic
/// quantization formula.
pub(super) fn read32_quat(reader: &mut Reader<'_>) -> Result<QuatA16, Error> {
    const R_MASK: u32 = (1 << 10) - 1;
    const R_FRAC: f32 = 1.0 / ((1u32 << 10) - 1) as f32;
    const PI_4: f32 = 0.25 * PI;
    const PHI_FRAC: f32 = (0.5 * PI) / 511.0;

    let c_val = reader.read_u32_le()?;

    // Recover the packed radial component.
    let mut r = ((c_val >> 18) & R_MASK) as f32 * R_FRAC;
    r = r.mul_add(-r, 1.0);

    // The lower 18 bits contain the packed angular coordinates.
    let phi_theta = (c_val & 0x3ffff) as f32;

    // Decode the triangular/angular packing used by the reference implementation.
    let mut phi = phi_theta.sqrt().floor();
    let mut theta = 0.0;

    if phi > 0.0 {
        theta = PI_4 * phi.mul_add(-phi, phi_theta) / phi;
        phi *= PHI_FRAC;
    }

    // Reconstruct the missing magnitude from the unit-quaternion relation.
    let magnitude = r.mul_add(-r, 1.0).sqrt();

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

pub(super) fn read40_quat(reader: &mut Reader<'_>) -> Result<QuatA16, Error> {
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

#[cfg_attr(
    feature = "tracing",
    tracing::instrument(level = "trace", skip(reader))
)]
pub(super) fn read48_quat(reader: &mut Reader<'_>) -> Result<QuatA16, Error> {
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

pub(super) fn read_quat(
    reader: &mut Reader<'_>,
    quantization: QuantizationType,
) -> Result<QuatA16, Error> {
    #[cfg(feature = "tracing")]
    tracing::trace!(quantization = ?quantization, "enter read_quat");

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
