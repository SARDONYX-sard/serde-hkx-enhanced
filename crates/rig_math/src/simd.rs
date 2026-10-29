// SPDX-License-Identifier: GPL-3.0-or-later
//
// Reference:
// - https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.hpp

//! Platform SIMD register types.
//!
//! Every vector type wraps one 128-bit register; only x86_64 (SSE) and
//! aarch64 (NEON) are supported because those are the targets Skyrim tools
//! run on, and both have the 4-lane instructions the spline decoder needs.

#[cfg(target_arch = "aarch64")]
pub(crate) use core::arch::aarch64::*;
#[cfg(target_arch = "x86_64")]
pub(crate) use core::arch::x86_64::*;
#[cfg(target_arch = "x86_64")]
pub(crate) type Float4 = __m128;

#[cfg(target_arch = "aarch64")]
pub(crate) type Float4 = float32x4_t;

#[cfg(target_arch = "x86_64")]
pub(crate) type Int4 = __m128i;

#[cfg(target_arch = "aarch64")]
pub(crate) type Int4 = int32x4_t;

#[cfg(target_arch = "x86_64")]
pub(crate) type UInt4 = __m128i;

#[cfg(target_arch = "aarch64")]
pub(crate) type UInt4 = uint32x4_t;
