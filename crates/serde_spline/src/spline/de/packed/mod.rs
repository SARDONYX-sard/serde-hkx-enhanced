// SPDX-License-Identifier: GPL-3.0-or-later

//! Havok spline binary representations.
mod bits;

use crate::spline::math::{IVec4A16, QuatA16, Vec4A16};

use self::bits::Field;

/// A packed 32-bit Havok quaternion.
///
/// The binary representation consists of:
///
/// - bits 0..18: angular information
/// - bits 18..28: radial information
/// - bits 28..32: component signs
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct Quat32(u32);

impl Quat32 {
    /// Packed angular field.
    const ANGLE: Field<0, 18> = Field::new();

    /// Packed radial field.
    const RADIAL: Field<18, 10> = Field::new();

    /// Creates a packed quaternion.
    #[inline]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the raw packed value.
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Returns the packed angular value.
    #[inline]
    pub const fn angle(self) -> u32 {
        Self::ANGLE.extract(self.0)
    }

    /// Returns the packed radial value.
    #[inline]
    pub const fn radial(self) -> u32 {
        Self::RADIAL.extract(self.0)
    }

    /// Decodes a packed 32-bit quaternion.
    ///
    /// This function contains only the mathematical reconstruction. Binary loading
    /// and bit-field extraction are handled by [`Quat32`] and [`quat32`].
    pub fn decode(self) -> QuatA16 {
        use core::f32::consts::{FRAC_PI_4, PI};
        const R_FRAC: f32 = 1.0 / ((1u32 << 10) - 1) as f32;
        const PHI_FRAC: f32 = (0.5 * PI) / 511.0;

        let value = {
            let mut r = self.radial() as f32 * R_FRAC;
            r = r.mul_add(-r, 1.0);

            let phi_theta = self.angle() as f32;

            let mut phi = phi_theta.sqrt().floor();
            let mut theta = 0.0;

            if phi > 0.0 {
                theta = FRAC_PI_4 * phi.mul_add(-phi, phi_theta) / phi;
                phi *= PHI_FRAC;
            }

            let magnitude = r.mul_add(-r, 1.0).sqrt();

            let sin_phi = phi.sin();
            let cos_phi = phi.cos();
            let sin_theta = theta.sin();
            let cos_theta = theta.cos();

            Vec4A16::new(sin_phi, sin_phi, cos_phi, r)
                * Vec4A16::new(cos_theta, sin_theta, 1.0, 1.0)
                * Vec4A16::new(magnitude, magnitude, magnitude, 1.0)
        };

        let blend_mask = {
            let sign_mask = {
                const X_MASK: i32 = 1 << 28;
                const Y_MASK: i32 = 1 << 29;
                const Z_MASK: i32 = 1 << 30;
                const W_MASK: i32 = 1 << 31;
                IVec4A16::new(X_MASK, Y_MASK, Z_MASK, W_MASK)
            };
            let packed = IVec4A16::splat(self.raw() as i32);

            (packed & sign_mask).cmp_eq(sign_mask)
        };

        QuatA16::from_vec4(value.select(-value, blend_mask))
    }
}

/// A packed 40-bit Havok quaternion.
///
/// The format is exactly five bytes and is defined in terms of individual
/// bit fields, so no external byte order is required.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quat40 {
    bytes: [u8; 5],
}

impl Quat40 {
    /// Creates a packed 40-bit quaternion.
    #[inline]
    pub const fn new(bytes: [u8; 5]) -> Self {
        Self { bytes }
    }

    /// Returns the first 12-bit component.
    #[inline]
    pub const fn x(self) -> u16 {
        self.bytes[0] as u16 | (((self.bytes[1] & 0x0f) as u16) << 8)
    }

    /// Returns the second 12-bit component.
    #[inline]
    pub const fn y(self) -> u16 {
        ((self.bytes[1] >> 4) as u16) | ((self.bytes[2] as u16) << 4)
    }

    /// Returns the third 12-bit component.
    #[inline]
    pub const fn z(self) -> u16 {
        self.bytes[3] as u16 | (((self.bytes[4] & 0x0f) as u16) << 8)
    }

    /// Returns the index of the reconstructed quaternion component.
    #[inline]
    pub const fn result_shift(self) -> usize {
        ((self.bytes[4] >> 4) & 0x03) as usize
    }

    /// Returns the sign bit of the reconstructed component.
    #[inline]
    pub const fn sign(self) -> bool {
        (self.bytes[4] & 0x40) != 0
    }

    /// Decodes a packed 40-bit quaternion.
    pub fn decode(self) -> QuatA16 {
        #[inline]
        fn de_quantize(value: u16) -> f32 {
            const INV_SQRT2: f32 = core::f32::consts::FRAC_1_SQRT_2;
            (value as f32 / 4095.0).mul_add(2.0 * INV_SQRT2, -INV_SQRT2)
        }

        let components = [
            de_quantize(self.x()),
            de_quantize(self.y()),
            de_quantize(self.z()),
        ];

        let sum_sq = components[2].mul_add(
            components[2],
            components[1].mul_add(components[1], components[0] * components[0]),
        );

        let mut reconstructed = (1.0 - sum_sq).max(0.0).sqrt();

        if self.sign() {
            reconstructed = -reconstructed;
        }

        let mut result = [0.0; 4];
        let mut source = 0;

        for (index, component) in result.iter_mut().enumerate() {
            if index == self.result_shift() {
                *component = reconstructed;
            } else {
                *component = components[source];
                source += 1;
            }
        }

        QuatA16::new(result[0], result[1], result[2], result[3])
    }
}

/// A packed 48-bit Havok quaternion.
///
/// Three 16-bit values are loaded using the selected byte order. Bit-level
/// interpretation is performed after parsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quat48 {
    pub x: u16,
    pub y: u16,
    pub z: u16,
}

impl Quat48 {
    /// Returns the index of the reconstructed component.
    #[inline]
    pub const fn result_shift(self) -> u32 {
        (((self.y >> 14) & 2) as u32) | (((self.x >> 15) & 1) as u32)
    }

    /// Returns whether the reconstructed component is negated.
    #[inline]
    pub const fn sign(self) -> bool {
        (self.z & 0x8000) != 0
    }

    /// Decodes a packed 48-bit quaternion.
    pub fn decode(self) -> QuatA16 {
        const MASK: i32 = (1 << 15) - 1;
        const FRACTION: f32 = 0.000043161;

        let value = IVec4A16::new(self.x as i32, self.y as i32, self.z as i32, 0);
        let mask = IVec4A16::splat(MASK);
        let value = (value & mask) - IVec4A16::splat(MASK >> 1);
        let value = value.to_f32() * Vec4A16::new(FRACTION, FRACTION, FRACTION, 0.0);
        let value = value * Vec4A16::new(1.0, 1.0, 1.0, if self.sign() { -1.0 } else { 1.0 });

        let value = match self.result_shift() {
            0 => value.shuffle::<0b11_00_01_10>(),
            1 => value.shuffle::<0b00_11_01_10>(),
            2 => value.shuffle::<0b00_01_11_10>(),
            _ => value,
        };

        QuatA16::from_vec4(value)
    }
}
