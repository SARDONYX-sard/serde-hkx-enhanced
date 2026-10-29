// SPDX-License-Identifier: GPL-3.0-or-later
//
// Reference:
// - https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.hpp

#![allow(clippy::missing_const_for_fn, reason = "SIMD intrinsics are not const")]

use core::{fmt, ops::{Add, BitAnd, BitOr, BitXor, Sub}};

use crate::{UVec4A16, Vec4A16};

use crate::simd::*;

/// A 16-byte-aligned four-lane signed integer SIMD value.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct IVec4A16(pub(crate) Int4);

impl IVec4A16 {
    /// Creates a four-lane signed integer SIMD value.
    #[inline]
    pub fn new(x: i32, y: i32, z: i32, w: i32) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_set_epi32(w, z, y, x)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe {
                let value = [x, y, z, w];
                Self(vld1q_s32(value.as_ptr()))
            }
        }
    }

    /// Creates a SIMD value with all lanes set to the same value.
    #[inline]
    pub fn splat(value: i32) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_set1_epi32(value)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vdupq_n_s32(value)) }
        }
    }

    /// Creates a value from a raw SIMD register.
    #[inline]
    pub const fn from_raw(value: Int4) -> Self {
        Self(value)
    }

    /// Returns the raw SIMD register.
    #[inline]
    pub const fn raw(self) -> Int4 {
        self.0
    }

    /// Returns the four signed integer lanes.
    #[inline]
    pub fn to_array(self) -> [i32; 4] {
        unsafe {
            let mut value = [0; 4];

            #[cfg(target_arch = "x86_64")]
            _mm_storeu_si128(value.as_mut_ptr().cast(), self.0);

            #[cfg(target_arch = "aarch64")]
            vst1q_s32(value.as_mut_ptr(), self.0);

            value
        }
    }

    /// Converts signed integer lanes to floating-point lanes.
    #[inline]
    pub fn to_f32(self) -> Vec4A16 {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Vec4A16(_mm_cvtepi32_ps(self.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Vec4A16(vcvtq_f32_s32(self.0)) }
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
            unsafe { Vec4A16(vreinterpretq_f32_s32(self.0)) }
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
            unsafe { Self(vreinterpretq_s32_u32(vceqq_s32(self.0, rhs.0))) }
        }
    }

    /// Returns one signed integer lane.
    #[inline]
    pub fn lane<const INDEX: i32>(self) -> i32 {
        self.to_array()[INDEX as usize]
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
            unsafe { Self(vshrq_n_s32(self.0, N)) }
        }
    }

    /// Performs an arithmetic right shift on every 32-bit lane.
    #[inline]
    pub fn sar<const N: i32>(self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_srai_epi32(self.0, N)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vshrq_n_s32(self.0, N)) }
        }
    }
}

impl fmt::Debug for IVec4A16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("IVec4A16").field(&self.to_array()).finish()
    }
}

impl From<UVec4A16> for IVec4A16 {
    #[inline]
    fn from(value: UVec4A16) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            Self(value.0)
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vreinterpretq_s32_u32(value.0)) }
        }
    }
}

impl From<IVec4A16> for Vec4A16 {
    #[inline]
    fn from(value: IVec4A16) -> Self {
        value.cast_f32()
    }
}

impl Add for IVec4A16 {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_add_epi32(self.0, rhs.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vaddq_s32(self.0, rhs.0)) }
        }
    }
}

impl Sub for IVec4A16 {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_sub_epi32(self.0, rhs.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vsubq_s32(self.0, rhs.0)) }
        }
    }
}

impl BitAnd for IVec4A16 {
    type Output = Self;

    #[inline]
    fn bitand(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_and_si128(self.0, rhs.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vandq_s32(self.0, rhs.0)) }
        }
    }
}

impl BitOr for IVec4A16 {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_or_si128(self.0, rhs.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(vorrq_s32(self.0, rhs.0)) }
        }
    }
}

impl BitXor for IVec4A16 {
    type Output = Self;

    #[inline]
    fn bitxor(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { Self(_mm_xor_si128(self.0, rhs.0)) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { Self(veorq_s32(self.0, rhs.0)) }
        }
    }
}
