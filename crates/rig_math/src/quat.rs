// SPDX-License-Identifier: GPL-3.0-or-later
//
// Reference:
// - https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.hpp

#![allow(clippy::missing_const_for_fn, reason = "SIMD intrinsics are not const")]

use core::{
    fmt,
    ops::{Add, Mul, Sub},
};

use havok_types::Quaternion;

use crate::Vec4A16;

use crate::simd::*;

/// A SIMD quaternion represented by four floating-point lanes.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct QuatA16(Vec4A16);

impl QuatA16 {
    /// Creates a SIMD quaternion.
    #[inline]
    pub fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self(Vec4A16::new(x, y, z, w))
    }

    /// Creates the identity quaternion.
    #[inline]
    pub fn identity() -> Self {
        Self::new(0.0, 0.0, 0.0, 1.0)
    }

    /// Creates a quaternion from a SIMD vector.
    #[inline]
    pub const fn from_vec4(value: Vec4A16) -> Self {
        Self(value)
    }

    /// Returns the underlying SIMD vector.
    #[inline]
    pub const fn into_vec4(self) -> Vec4A16 {
        self.0
    }

    /// Returns the four quaternion components.
    #[inline]
    pub fn to_array(self) -> [f32; 4] {
        self.0.to_array()
    }

    /// Computes the quaternion dot product.
    #[inline]
    pub fn dot(self, rhs: Self) -> f32 {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe {
                let product = _mm_mul_ps(self.0.raw(), rhs.0.raw());
                let shuf = _mm_movehdup_ps(product);
                let sums = _mm_add_ps(product, shuf);
                let shuf = _mm_movehl_ps(shuf, sums);
                let sums = _mm_add_ss(sums, shuf);

                _mm_cvtss_f32(sums)
            }
        }

        #[cfg(target_arch = "aarch64")]
        {
            let [x0, y0, z0, w0] = self.to_array();
            let [x1, y1, z1, w1] = rhs.to_array();
            x0.mul_add(x1, y0.mul_add(y1, z0.mul_add(z1, w0 * w1)))
        }
    }

    /// Returns the negated quaternion.
    #[inline]
    pub fn negated(self) -> Self {
        Self(-self.0)
    }

    /// Normalizes the quaternion.
    #[inline]
    pub fn normalize(self) -> Self {
        let [x, y, z, w] = self.to_array();

        let length = w.mul_add(w, z.mul_add(z, y.mul_add(y, x * x))).sqrt();

        if length <= f32::EPSILON {
            return Self::identity();
        }

        Self::new(x / length, y / length, z / length, w / length)
    }
}

impl fmt::Debug for QuatA16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("QuatA16").field(&self.to_array()).finish()
    }
}

impl From<Quaternion> for QuatA16 {
    #[inline]
    fn from(value: Quaternion) -> Self {
        Self::new(value.x, value.y, value.z, value.scaler)
    }
}

impl From<QuatA16> for Quaternion {
    #[inline]
    fn from(value: QuatA16) -> Self {
        let [x, y, z, w] = value.to_array();

        Self::new(x, y, z, w)
    }
}

impl Add for QuatA16 {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl Sub for QuatA16 {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl Mul<f32> for QuatA16 {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: f32) -> Self {
        Self(self.0 * Vec4A16::splat(rhs))
    }
}
