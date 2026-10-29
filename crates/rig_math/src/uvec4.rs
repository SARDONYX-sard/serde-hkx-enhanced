// SPDX-License-Identifier: GPL-3.0-or-later
//
// Reference:
// - https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.hpp

#![allow(clippy::missing_const_for_fn, reason = "SIMD intrinsics are not const")]

use core::{
    fmt,
    ops::{BitAnd, BitOr, BitXor},
};

use crate::{IVec4A16, Vec4A16};

use crate::simd::*;

/// A 16-byte-aligned four-lane unsigned integer SIMD value.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct UVec4A16(pub(crate) UInt4);

impl UVec4A16 {
    /// Creates a four-lane unsigned integer SIMD value.
    #[inline]
    pub fn new(x: u32, y: u32, z: u32, w: u32) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_set_epi32(w as i32, z as i32, y as i32, x as i32)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe {
                let value = [x, y, z, w];
                Self(vld1q_u32(value.as_ptr()))
            }
        }
    }

    /// Creates a SIMD value with all lanes set to the same value.
    #[inline]
    pub fn splat(value: u32) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_set1_epi32(value as i32)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vdupq_n_u32(value)) }
        }
    }

    /// Creates a value from a raw SIMD register.
    #[inline]
    pub const fn from_raw(value: UInt4) -> Self {
        Self(value)
    }

    /// Returns the raw SIMD register.
    #[inline]
    pub const fn raw(self) -> UInt4 {
        self.0
    }

    /// Returns the four unsigned integer lanes.
    #[inline]
    pub fn to_array(self) -> [u32; 4] {
        unsafe {
            let mut value = [0u32; 4];

            #[cfg(target_arch = "x86_64")]
            _mm_storeu_si128(value.as_mut_ptr().cast(), self.0);

            #[cfg(target_arch = "aarch64")]
            vst1q_u32(value.as_mut_ptr(), self.0);

            value
        }
    }

    /// Reinterprets the bits as signed integer lanes.
    #[inline]
    pub fn cast_i32(self) -> IVec4A16 {
        #[cfg(target_arch = "x86_64")]
        {
            IVec4A16(self.0)
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { IVec4A16(vreinterpretq_s32_u32(self.0)) }
        }
    }

    /// Reinterprets the bits as floating-point lanes.
    #[inline]
    pub fn cast_f32(self) -> Vec4A16 {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Vec4A16(_mm_castsi128_ps(self.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Vec4A16(vreinterpretq_f32_u32(self.0)) }
        }
    }

    /// Compares all lanes for equality.
    #[inline]
    pub fn cmp_eq(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_cmpeq_epi32(self.0, rhs.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vceqq_u32(self.0, rhs.0)) }
        }
    }

    /// Performs a low 32-bit multiplication on every lane.
    #[inline]
    pub fn mul_lo(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_mullo_epi32(self.0, rhs.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vmulq_u32(self.0, rhs.0)) }
        }
    }

    /// Performs a logical right shift on every 32-bit lane.
    #[inline]
    pub fn shr<const N: i32>(self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_srli_epi32(self.0, N)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vshrq_n_u32(self.0, N)) }
        }
    }
}

impl fmt::Debug for UVec4A16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("UVec4A16").field(&self.to_array()).finish()
    }
}

impl From<IVec4A16> for UVec4A16 {
    #[inline]
    fn from(value: IVec4A16) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            Self(value.0)
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vreinterpretq_u32_s32(value.0)) }
        }
    }
}

impl BitAnd for UVec4A16 {
    type Output = Self;

    #[inline]
    fn bitand(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_and_si128(self.0, rhs.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vandq_u32(self.0, rhs.0)) }
        }
    }
}

impl BitOr for UVec4A16 {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_or_si128(self.0, rhs.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vorrq_u32(self.0, rhs.0)) }
        }
    }
}

impl BitXor for UVec4A16 {
    type Output = Self;

    #[inline]
    fn bitxor(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_xor_si128(self.0, rhs.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(veorq_u32(self.0, rhs.0)) }
        }
    }
}
