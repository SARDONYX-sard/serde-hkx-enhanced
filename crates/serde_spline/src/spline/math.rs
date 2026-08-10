// SPDX-License-Identifier: GPL-3.0-or-later
//
// Reference:
// https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.hpp
// https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.cpp

//! SIMD and spline types used by the Havok spline decompressor.

use core::{
    arch::x86_64::*,
    fmt,
    ops::{Add, BitAnd, BitOr, BitXor, Mul, Neg, Sub},
};

use super::SplineError;
use havok_types::{Quaternion, Vector4};

/// A 16-byte-aligned four-lane floating-point SIMD value.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct Vec4A16(__m128);

impl Vec4A16 {
    /// Creates a four-lane SIMD value.
    #[inline]
    pub fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        unsafe { Self(_mm_set_ps(w, z, y, x)) }
    }

    /// Creates a SIMD value with all lanes set to the same value.
    #[inline]
    pub fn splat(value: f32) -> Self {
        unsafe { Self(_mm_set1_ps(value)) }
    }

    /// Creates a value from a raw SIMD register.
    #[inline]
    pub const fn from_raw(value: __m128) -> Self {
        Self(value)
    }

    /// Returns the raw SIMD register.
    #[inline]
    pub const fn raw(self) -> __m128 {
        self.0
    }

    /// Returns the four lanes.
    #[inline]
    pub fn to_array(self) -> [f32; 4] {
        unsafe {
            let mut value = [0.0; 4];
            _mm_storeu_ps(value.as_mut_ptr(), self.0);
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
        unsafe { IVec4A16(_mm_castps_si128(self.0)) }
    }

    /// Reinterprets the bits as unsigned integer lanes.
    #[inline]
    pub fn cast_u32(self) -> UVec4A16 {
        unsafe { UVec4A16(_mm_castps_si128(self.0)) }
    }

    /// Returns the absolute value of every lane.
    #[inline]
    pub fn abs(self) -> Self {
        unsafe {
            Self(_mm_and_ps(
                self.0,
                _mm_castsi128_ps(_mm_set1_epi32(0x7fff_ffff)),
            ))
        }
    }

    /// Returns the square root of every lane.
    #[inline]
    pub fn sqrt(self) -> Self {
        unsafe { Self(_mm_sqrt_ps(self.0)) }
    }

    /// Selects `rhs` where the mask is set and `self` otherwise.
    #[inline]
    pub fn select(self, rhs: Self, mask: IVec4A16) -> Self {
        unsafe {
            let mask = _mm_castsi128_ps(mask.0);

            Self(_mm_or_ps(
                _mm_andnot_ps(mask, self.0),
                _mm_and_ps(mask, rhs.0),
            ))
        }
    }

    /// Shuffles lanes using an SSE shuffle mask.
    #[inline]
    pub fn shuffle<const MASK: i32>(self) -> Self {
        unsafe { Self(_mm_shuffle_ps(self.0, self.0, MASK)) }
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
        unsafe { Self(_mm_add_ps(self.0, rhs.0)) }
    }
}

impl Sub for Vec4A16 {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self {
        unsafe { Self(_mm_sub_ps(self.0, rhs.0)) }
    }
}

impl Mul for Vec4A16 {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: Self) -> Self {
        unsafe { Self(_mm_mul_ps(self.0, rhs.0)) }
    }
}

impl Neg for Vec4A16 {
    type Output = Self;

    #[inline]
    fn neg(self) -> Self {
        unsafe { Self(_mm_xor_ps(self.0, _mm_set1_ps(-0.0))) }
    }
}

/// A 16-byte-aligned four-lane signed integer SIMD value.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct IVec4A16(__m128i);

impl IVec4A16 {
    /// Creates a four-lane signed integer SIMD value.
    #[inline]
    pub fn new(x: i32, y: i32, z: i32, w: i32) -> Self {
        unsafe { Self(_mm_set_epi32(w, z, y, x)) }
    }

    /// Creates a SIMD value with all lanes set to the same value.
    #[inline]
    pub fn splat(value: i32) -> Self {
        unsafe { Self(_mm_set1_epi32(value)) }
    }

    /// Creates a value from a raw SIMD register.
    #[inline]
    pub const fn from_raw(value: __m128i) -> Self {
        Self(value)
    }

    /// Returns the raw SIMD register.
    #[inline]
    pub const fn raw(self) -> __m128i {
        self.0
    }

    /// Returns the four signed integer lanes.
    #[inline]
    pub fn to_array(self) -> [i32; 4] {
        unsafe {
            let mut value = [0; 4];
            _mm_storeu_si128(value.as_mut_ptr().cast(), self.0);
            value
        }
    }

    /// Converts signed integer lanes to floating-point lanes.
    #[inline]
    pub fn to_f32(self) -> Vec4A16 {
        unsafe { Vec4A16(_mm_cvtepi32_ps(self.0)) }
    }

    /// Reinterprets the bits as floating-point lanes.
    #[inline]
    pub fn cast_f32(self) -> Vec4A16 {
        unsafe { Vec4A16(_mm_castsi128_ps(self.0)) }
    }

    /// Compares all lanes for equality.
    #[inline]
    pub fn cmp_eq(self, rhs: Self) -> Self {
        unsafe { Self(_mm_cmpeq_epi32(self.0, rhs.0)) }
    }

    /// Returns one signed integer lane.
    #[inline]
    pub fn lane<const INDEX: i32>(self) -> i32 {
        unsafe { _mm_extract_epi32(self.0, INDEX) }
    }

    /// Performs a logical right shift on every 32-bit lane.
    #[inline]
    pub fn shr<const N: i32>(self) -> Self {
        unsafe { Self(_mm_srli_epi32(self.0, N)) }
    }

    /// Performs an arithmetic right shift on every 32-bit lane.
    #[inline]
    pub fn sar<const N: i32>(self) -> Self {
        unsafe { Self(_mm_srai_epi32(self.0, N)) }
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
        Self(value.0)
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
        unsafe { Self(_mm_add_epi32(self.0, rhs.0)) }
    }
}

impl Sub for IVec4A16 {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self {
        unsafe { Self(_mm_sub_epi32(self.0, rhs.0)) }
    }
}

impl BitAnd for IVec4A16 {
    type Output = Self;

    #[inline]
    fn bitand(self, rhs: Self) -> Self {
        unsafe { Self(_mm_and_si128(self.0, rhs.0)) }
    }
}

impl BitOr for IVec4A16 {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        unsafe { Self(_mm_or_si128(self.0, rhs.0)) }
    }
}

impl BitXor for IVec4A16 {
    type Output = Self;

    #[inline]
    fn bitxor(self, rhs: Self) -> Self {
        unsafe { Self(_mm_xor_si128(self.0, rhs.0)) }
    }
}

/// A 16-byte-aligned four-lane unsigned integer SIMD value.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct UVec4A16(__m128i);

impl UVec4A16 {
    /// Creates a four-lane unsigned integer SIMD value.
    #[inline]
    pub fn new(x: u32, y: u32, z: u32, w: u32) -> Self {
        unsafe { Self(_mm_set_epi32(w as i32, z as i32, y as i32, x as i32)) }
    }

    /// Creates a SIMD value with all lanes set to the same value.
    #[inline]
    pub fn splat(value: u32) -> Self {
        unsafe { Self(_mm_set1_epi32(value as i32)) }
    }

    /// Creates a value from a raw SIMD register.
    #[inline]
    pub const fn from_raw(value: __m128i) -> Self {
        Self(value)
    }

    /// Returns the raw SIMD register.
    #[inline]
    pub const fn raw(self) -> __m128i {
        self.0
    }

    /// Returns the four unsigned integer lanes.
    #[inline]
    pub fn to_array(self) -> [u32; 4] {
        unsafe {
            let mut value = [0u32; 4];
            _mm_storeu_si128(value.as_mut_ptr().cast(), self.0);
            value
        }
    }

    /// Reinterprets the bits as signed integer lanes.
    #[inline]
    pub const fn cast_i32(self) -> IVec4A16 {
        IVec4A16(self.0)
    }

    /// Reinterprets the bits as floating-point lanes.
    #[inline]
    pub fn cast_f32(self) -> Vec4A16 {
        unsafe { Vec4A16(_mm_castsi128_ps(self.0)) }
    }

    /// Compares all lanes for equality.
    #[inline]
    pub fn cmp_eq(self, rhs: Self) -> Self {
        unsafe { Self(_mm_cmpeq_epi32(self.0, rhs.0)) }
    }

    /// Performs a low 32-bit multiplication on every lane.
    #[inline]
    pub fn mul_lo(self, rhs: Self) -> Self {
        unsafe { Self(_mm_mullo_epi32(self.0, rhs.0)) }
    }

    /// Performs a logical right shift on every 32-bit lane.
    #[inline]
    pub fn shr<const N: i32>(self) -> Self {
        unsafe { Self(_mm_srli_epi32(self.0, N)) }
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
        Self(value.0)
    }
}

impl BitAnd for UVec4A16 {
    type Output = Self;

    #[inline]
    fn bitand(self, rhs: Self) -> Self {
        unsafe { Self(_mm_and_si128(self.0, rhs.0)) }
    }
}

impl BitOr for UVec4A16 {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        unsafe { Self(_mm_or_si128(self.0, rhs.0)) }
    }
}

impl BitXor for UVec4A16 {
    type Output = Self;

    #[inline]
    fn bitxor(self, rhs: Self) -> Self {
        unsafe { Self(_mm_xor_si128(self.0, rhs.0)) }
    }
}

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
        unsafe {
            let product = _mm_mul_ps(self.0.raw(), rhs.0.raw());
            let shuf = _mm_movehdup_ps(product);
            let sums = _mm_add_ps(product, shuf);
            let shuf = _mm_movehl_ps(shuf, sums);
            let sums = _mm_add_ss(sums, shuf);

            _mm_cvtss_f32(sums)
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

/// The spline sub-track kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SplineTrackType {
    /// The track contains one constant value.
    Static = 0,

    /// The track contains spline control points.
    Dynamic = 1,

    /// The track has no stored value and uses its default.
    Identity = 2,
}

/// The quantization format used by a transform track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum QuantizationType {
    /// Eight-bit scalar quantization.
    Bit8 = 0,

    /// Sixteen-bit scalar quantization.
    Bit16 = 1,

    /// Thirty-two-bit quaternion encoding.
    Bit32 = 2,

    /// Forty-bit quaternion encoding.
    Bit40 = 3,

    /// Forty-eight-bit quaternion encoding.
    Bit48 = 4,

    /// Twenty-four-bit encoding.
    Bit24 = 5,

    /// Sixteen-bit quaternion encoding.
    Bit16Quat = 6,

    /// Uncompressed quaternion data.
    Uncompressed = 7,
}

/// The transform component represented by a track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TransformType {
    /// Position X.
    PosX = 0,

    /// Position Y.
    PosY = 1,

    /// Position Z.
    PosZ = 2,

    /// Rotation quaternion.
    Rotation = 3,

    /// Scale X.
    ScaleX = 4,

    /// Scale Y.
    ScaleY = 5,

    /// Scale Z.
    ScaleZ = 6,
}

/// The packed transform mask preceding each track.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TransformMask {
    pub quantization_types: u8,
    pub position_types: u8,
    pub rotation_types: u8,
    pub scale_types: u8,
}
// This guarantee is important because the Mask must be obtained in the exact same order and size as the actual binary layout,
// and its size must match that of the binary.
const _: () = assert!(core::mem::size_of::<TransformMask>() == 4);

impl TransformMask {
    /// Sets the position/scale quantization type (0 or 1; matches
    /// `position_quantization_type`'s decode).
    ///
    /// # Panics
    /// Panics if `kind` is not `Bit8` or `Bit16` — those are the only two
    /// values `position_quantization_type`/`scale_quantization_type` can
    /// decode back out.
    #[inline]
    pub const fn set_position_quantization_type(&mut self, kind: QuantizationType) {
        let bits = match kind {
            QuantizationType::Bit8 => 0,
            QuantizationType::Bit16 => 1,
            _ => return,
        };
        self.quantization_types = (self.quantization_types & !0b11) | bits;
    }

    /// Sets the rotation quantization type (packed into bits 2..=5, offset
    /// by 2; matches `rotation_quantization_type`'s decode).
    #[inline]
    pub const fn set_rotation_quantization_type(&mut self, kind: QuantizationType) {
        let value = match kind {
            QuantizationType::Bit32 => 2,
            QuantizationType::Bit40 => 3,
            QuantizationType::Bit48 => 4,
            QuantizationType::Bit24 => 5,
            QuantizationType::Bit16Quat => 6,
            QuantizationType::Uncompressed => 7,
            _ => return,
        };
        let bits = (value - 2) & 0x0f;
        self.quantization_types = (self.quantization_types & !(0x0f << 2)) | (bits << 2);
    }

    /// Sets the scale quantization type (bits 6..=7; matches
    /// `scale_quantization_type`'s decode).
    #[inline]
    pub const fn set_scale_quantization_type(&mut self, kind: QuantizationType) {
        let bits = match kind {
            QuantizationType::Bit8 => 0,
            QuantizationType::Bit16 => 1,
            _ => return,
        };
        self.quantization_types = (self.quantization_types & !(0b11 << 6)) | (bits << 6);
    }

    /// Sets the sub-track kind for a transform component.
    ///
    /// This is the exact inverse of `sub_track_type`. It always clears the
    /// bit(s) it doesn't set, so calling this repeatedly (e.g. once per
    /// axis while building a mask from scratch) never leaves stale bits
    /// from a previous kind behind.
    ///
    /// Note the same asymmetry `sub_track_type` has: for position/scale,
    /// `Static` takes priority over `Dynamic` on read, so `Dynamic` here
    /// must leave the static bit clear or the value would decode back as
    /// `Static` instead.
    #[inline]
    pub fn set_sub_track_type(&mut self, transform: TransformType, kind: SplineTrackType) {
        if transform == TransformType::Rotation {
            // `sub_track_type` only checks "is any bit in the nibble set",
            // not a specific value, so any nonzero nibble round-trips.
            // Using the full nibble (0xf) keeps this obviously non-magic.
            let (keep_mask, set_bits): (u8, u8) = match kind {
                SplineTrackType::Dynamic => (0x0f, 0xf0), // high nibble set
                SplineTrackType::Static => (0xf0, 0x0f),  // low nibble set
                SplineTrackType::Identity => (0x00, 0x00), // both clear
            };
            self.rotation_types = (self.rotation_types & keep_mask) | set_bits;
            return;
        }

        let flags = match transform {
            TransformType::PosX | TransformType::PosY | TransformType::PosZ => {
                &mut self.position_types
            }
            TransformType::ScaleX | TransformType::ScaleY | TransformType::ScaleZ => {
                &mut self.scale_types
            }
            TransformType::Rotation => unreachable!("handled above"),
        };

        let (static_bit, spline_bit) = match transform {
            TransformType::PosX | TransformType::ScaleX => (0, 4),
            TransformType::PosY | TransformType::ScaleY => (1, 5),
            TransformType::PosZ | TransformType::ScaleZ => (2, 6),
            TransformType::Rotation => unreachable!("handled above"),
        };

        let clear_mask = !((1 << static_bit) | (1 << spline_bit));
        let set_bits = match kind {
            // static_bit set, spline_bit left clear.
            SplineTrackType::Static => 1 << static_bit,
            // spline_bit set, static_bit left clear (static takes priority
            // on read, so both-set would decode as Static).
            SplineTrackType::Dynamic => 1 << spline_bit,
            // both clear.
            SplineTrackType::Identity => 0,
        };

        *flags = (*flags & clear_mask) | set_bits;
    }
}

impl TransformMask {
    /// Returns the position quantization type.
    ///
    /// # Errors
    /// neither 0, 1
    #[inline]
    pub const fn position_quantization_type(self) -> Result<QuantizationType, SplineError> {
        match self.quantization_types & 0b11 {
            0 => Ok(QuantizationType::Bit8),
            1 => Ok(QuantizationType::Bit16),
            value => Err(SplineError::InvalidQuantizationType(value)),
        }
    }

    /// Returns the rotation quantization type.
    ///
    /// # Errors
    /// out of range 2..=7
    #[inline]
    pub const fn rotation_quantization_type(self) -> Result<QuantizationType, SplineError> {
        match ((self.quantization_types >> 2) & 0x0f) + 2 {
            2 => Ok(QuantizationType::Bit32),
            3 => Ok(QuantizationType::Bit40),
            4 => Ok(QuantizationType::Bit48),
            5 => Ok(QuantizationType::Bit24),
            6 => Ok(QuantizationType::Bit16Quat),
            7 => Ok(QuantizationType::Uncompressed),
            value => Err(SplineError::InvalidQuantizationType(value)),
        }
    }

    /// Returns the scale quantization type.
    ///
    /// # Errors
    /// neither 0, 1
    #[inline]
    pub const fn scale_quantization_type(self) -> Result<QuantizationType, SplineError> {
        match (self.quantization_types >> 6) & 0b11 {
            0 => Ok(QuantizationType::Bit8),
            1 => Ok(QuantizationType::Bit16),
            value => Err(SplineError::InvalidQuantizationType(value)),
        }
    }

    /// Returns the sub-track kind for a transform component.
    #[inline]
    pub const fn sub_track_type(self, transform: TransformType) -> SplineTrackType {
        let flags = match transform {
            TransformType::PosX | TransformType::PosY | TransformType::PosZ => self.position_types,

            TransformType::ScaleX | TransformType::ScaleY | TransformType::ScaleZ => {
                self.scale_types
            }

            TransformType::Rotation => {
                return if self.rotation_types & 0xf0 != 0 {
                    SplineTrackType::Dynamic
                } else if self.rotation_types & 0x0f != 0 {
                    SplineTrackType::Static
                } else {
                    SplineTrackType::Identity
                };
            }
        };

        let (static_bit, spline_bit) = match transform {
            TransformType::PosX | TransformType::ScaleX => (0, 4),
            TransformType::PosY | TransformType::ScaleY => (1, 5),
            TransformType::PosZ | TransformType::ScaleZ => (2, 6),
            TransformType::Rotation => return SplineTrackType::Identity,
        };

        if flags & (1 << static_bit) != 0 {
            SplineTrackType::Static
        } else if flags & (1 << spline_bit) != 0 {
            SplineTrackType::Dynamic
        } else {
            SplineTrackType::Identity
        }
    }
}

/// A static spline track value.
#[derive(Clone, Debug)]
pub struct SplineStaticTrack<T> {
    pub value: T,
}

/// A dynamic scalar transform track.
#[derive(Clone, Debug)]
pub struct SplineDynamicTrackVector {
    pub tracks: [Vec<f32>; NUM_VECTOR_AXES],
    pub knots: Vec<f32>,
    pub degree: u8,
}
const NUM_VECTOR_AXES: usize = 3;
const _: () = assert!(NUM_VECTOR_AXES == 3);

/// A dynamic quaternion spline track.
#[derive(Clone, Debug)]
pub struct SplineDynamicTrackQuat {
    pub track: Vec<QuatA16>,
    pub knots: Vec<f32>,
    pub degree: u8,
}

/// A position, rotation, and scale track.
#[derive(Clone, Debug)]
pub struct TransformTrack {
    pub position: SplineTrackVector,
    pub rotation: SplineTrackQuat,
    pub scale: SplineTrackVector,
}

/// A position or scale track.
#[derive(Clone, Debug)]
pub enum SplineTrackVector {
    /// A static value.
    Static(SplineStaticTrack<Vector4>),

    /// A dynamic spline.
    Dynamic(SplineDynamicTrackVector),
}

/// A rotation track.
#[derive(Clone, Debug)]
pub enum SplineTrackQuat {
    /// A static quaternion.
    Static(SplineStaticTrack<QuatA16>),

    /// A dynamic spline.
    Dynamic(SplineDynamicTrackQuat),

    /// An identity quaternion.
    Identity,
}

/// A decoded spline block.
#[derive(Clone, Debug)]
pub struct TransformSplineBlock {
    pub masks: Vec<TransformMask>,
    pub tracks: Vec<TransformTrack>,
}
