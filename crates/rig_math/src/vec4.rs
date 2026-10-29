// SPDX-License-Identifier: GPL-3.0-or-later
//
// Reference:
// - https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.hpp

#![allow(clippy::missing_const_for_fn, reason = "SIMD intrinsics are not const")]

use core::{fmt, ops::{Add, Mul, Neg, Sub}};

use havok_types::Vector4;

use crate::simd::*;
use crate::{IVec4A16, UVec4A16};

/// A 16-byte-aligned four-lane floating-point SIMD value.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct Vec4A16(pub(crate) Float4);

impl Vec4A16 {
    /// Creates a four-lane SIMD value.
    #[inline]
    pub fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_set_ps(w, z, y, x)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe {
                let value = [x, y, z, w];
                Self(vld1q_f32(value.as_ptr()))
            }
        }
    }

    /// Creates a SIMD value with all lanes set to the same value.
    #[inline]
    pub fn splat(value: f32) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_set1_ps(value)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vdupq_n_f32(value)) }
        }
    }

    /// Creates a value from a raw SIMD register.
    #[inline]
    pub const fn from_raw(value: Float4) -> Self {
        Self(value)
    }

    /// Returns the raw SIMD register.
    #[inline]
    pub const fn raw(self) -> Float4 {
        self.0
    }

    /// Returns the four lanes.
    #[inline]
    pub fn to_array(self) -> [f32; 4] {
        unsafe {
            let mut value = [0.0; 4];

            #[cfg(target_arch = "x86_64")]
            _mm_storeu_ps(value.as_mut_ptr(), self.0);

            #[cfg(target_arch = "aarch64")]
            vst1q_f32(value.as_mut_ptr(), self.0);

            value
        }
    }

    /// Converts the SIMD value into a `Vector4`.
    #[inline]
    pub fn into_vector4(self) -> Vector4 {
        let [x, y, z, w] = self.to_array();

        Vector4 { x, y, z, w }
    }

    /// Reinterprets the bits as signed integer lanes.
    #[inline]
    pub fn cast_i32(self) -> IVec4A16 {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { IVec4A16(_mm_castps_si128(self.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { IVec4A16(vreinterpretq_s32_f32(self.0)) }
        }
    }

    /// Reinterprets the bits as unsigned integer lanes.
    #[inline]
    pub fn cast_u32(self) -> UVec4A16 {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { UVec4A16(_mm_castps_si128(self.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { UVec4A16(vreinterpretq_u32_f32(self.0)) }
        }
    }

    /// Returns the absolute value of every lane.
    #[inline]
    pub fn abs(self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe {
                Self(_mm_and_ps(
                    self.0,
                    _mm_castsi128_ps(_mm_set1_epi32(0x7fff_ffff)),
                ))
            }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vabsq_f32(self.0)) }
        }
    }

    /// Returns the square root of every lane.
    #[inline]
    pub fn sqrt(self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_sqrt_ps(self.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vsqrtq_f32(self.0)) }
        }
    }

    /// Selects `rhs` where the mask is set and `self` otherwise.
    #[inline]
    pub fn select(self, rhs: Self, mask: IVec4A16) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe {
                let mask = _mm_castsi128_ps(mask.0);

                Self(_mm_or_ps(
                    _mm_andnot_ps(mask, self.0),
                    _mm_and_ps(mask, rhs.0),
                ))
            }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vbslq_f32(vreinterpretq_u32_s32(mask.0), rhs.0, self.0)) }
        }
    }

    /// Shuffles lanes using an SSE-compatible shuffle mask.
    ///
    /// On AArch64 this is implemented through scalar lane extraction because
    /// NEON does not provide a direct equivalent of `_mm_shuffle_ps`.
    #[inline]
    pub fn shuffle<const MASK: i32>(self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_shuffle_ps(self.0, self.0, MASK)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            let lanes = self.to_array();

            let x = lanes[(MASK & 0x03) as usize];
            let y = lanes[((MASK >> 2) & 0x03) as usize];
            let z = lanes[((MASK >> 4) & 0x03) as usize];
            let w = lanes[((MASK >> 6) & 0x03) as usize];

            Self::new(x, y, z, w)
        }
    }
}

impl fmt::Debug for Vec4A16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Vec4A16").field(&self.to_array()).finish()
    }
}

impl From<Vector4> for Vec4A16 {
    #[inline]
    fn from(value: Vector4) -> Self {
        Self::new(value.x, value.y, value.z, value.w)
    }
}

impl From<Vec4A16> for Vector4 {
    #[inline]
    fn from(value: Vec4A16) -> Self {
        value.into_vector4()
    }
}

impl Add for Vec4A16 {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_add_ps(self.0, rhs.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vaddq_f32(self.0, rhs.0)) }
        }
    }
}

impl Sub for Vec4A16 {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_sub_ps(self.0, rhs.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vsubq_f32(self.0, rhs.0)) }
        }
    }
}

impl Mul for Vec4A16 {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_mul_ps(self.0, rhs.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vmulq_f32(self.0, rhs.0)) }
        }
    }
}

impl Neg for Vec4A16 {
    type Output = Self;

    #[inline]
    fn neg(self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_xor_ps(self.0, _mm_set1_ps(-0.0))) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vnegq_f32(self.0)) }
        }
    }
}
