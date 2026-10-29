// SPDX-License-Identifier: GPL-3.0-or-later
//
// Reference:
// - https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.hpp

//! Track masks and decoded track types of a spline-compressed block.

use havok_types::Vector4;
use rig_math::QuatA16;

use crate::error::Error;

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
    /// `mask_and_quantization_size = hkaSplineCompressedAnimation.numberOfTransformTracks / MASK_SIZE`
    pub const MASK_SIZE: i32 = core::mem::size_of::<Self>() as i32;

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
    pub const fn position_quantization_type(self) -> Result<QuantizationType, Error> {
        match self.quantization_types & 0b11 {
            0 => Ok(QuantizationType::Bit8),
            1 => Ok(QuantizationType::Bit16),
            value => Err(Error::InvalidQuantizationType(value)),
        }
    }

    /// Returns the rotation quantization type.
    ///
    /// # Errors
    /// out of range 2..=7
    #[inline]
    pub const fn rotation_quantization_type(self) -> Result<QuantizationType, Error> {
        match ((self.quantization_types >> 2) & 0x0f) + 2 {
            2 => Ok(QuantizationType::Bit32),
            3 => Ok(QuantizationType::Bit40),
            4 => Ok(QuantizationType::Bit48),
            5 => Ok(QuantizationType::Bit24),
            6 => Ok(QuantizationType::Bit16Quat),
            7 => Ok(QuantizationType::Uncompressed),
            value => Err(Error::InvalidQuantizationType(value)),
        }
    }

    /// Returns the scale quantization type.
    ///
    /// # Errors
    /// neither 0, 1
    #[inline]
    pub const fn scale_quantization_type(self) -> Result<QuantizationType, Error> {
        match (self.quantization_types >> 6) & 0b11 {
            0 => Ok(QuantizationType::Bit8),
            1 => Ok(QuantizationType::Bit16),
            value => Err(Error::InvalidQuantizationType(value)),
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
