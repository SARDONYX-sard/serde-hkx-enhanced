// SPDX-License-Identifier: GPL-3.0-or-later
//
// Reference:
// - https://github.com/BadDogSkyrim/PyNifly/blob/7fd4644f5a6416c1502983b7d49a853eb0d24509/io_scene_nifly/hkx/anim_fo4.py
// - https://github.com/BadDogSkyrim/PyNifly/blob/7fd4644f5a6416c1502983b7d49a853eb0d24509/io_scene_nifly/hkx/anim_skyrim.py
//
// Additional format references:
// - https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.hpp
// - https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.cpp

//! Encodes Havok spline-compressed animation blocks.
//!
//! The compressor follows the export implementation used by PyNifly:
//!
//! - dynamic scalar tracks are fitted to degree-1 B-splines;
//! - quaternion samples are made sign-continuous before fitting;
//! - quaternion control points are made sign-continuous and normalized;
//! - scalar tracks support 8-bit and 16-bit quantization;
//! - quaternion tracks support THREECOMP40 and THREECOMP48;
//! - block, track, spline, and quaternion alignment follows Havok's format.
//!
//! The [`SplineDynamicTrackVector`] and [`SplineDynamicTrackQuat`] values are
//! treated as sampled frame data when encoding. Their stored knot vectors are
//! not blindly reused as already-fitted control-point data.

#[cfg(feature = "tracing")]
mod debug;
mod fit;
mod rotation;
mod vector;

use self::rotation::encode_rotation;
use self::vector::{encode_position, encode_scale};
use super::{
    SplineDecompressor,
    track::{QuantizationType, TransformMask, TransformSplineBlock},
};
use crate::error::Error;

/// Encoded spline animation data.
#[derive(Clone, Debug)]
pub struct SplineEncodedData {
    /// Serialized spline-compressed animation data.
    pub data: Vec<u8>,

    /// Byte offset of every encoded spline block.
    pub block_offsets: Vec<u32>,
}

impl SplineDecompressor {
    /// Encodes all spline blocks into Havok spline-compressed animation data.
    ///
    /// Dynamic tracks are interpreted as sampled frame values and are fitted
    /// to the B-spline representation expected by Havok.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the animation contains no blocks, if block track
    /// counts differ, if a track contains invalid spline data, if a value is
    /// non-finite, or if a selected quantization format cannot be encoded.
    pub fn encode(&self) -> Result<SplineEncodedData, Error> {
        if self.blocks.is_empty() {
            return Err(Error::InvalidData("cannot encode empty spline data"));
        }

        let transform_track_count = self.blocks[0].tracks.len();

        for block in &self.blocks {
            if block.masks.len() != transform_track_count
                || block.tracks.len() != transform_track_count
            {
                return Err(Error::InvalidData(
                    "animation data blocks do not have equal transform track counts",
                ));
            }
        }

        let mut data = Vec::new();
        let mut block_offsets = Vec::with_capacity(self.blocks.len());

        for (block_index, block) in self.blocks.iter().enumerate() {
            align4(&mut data);

            let offset = u32::try_from(data.len())
                .map_err(|_| Error::InvalidData("encoded spline data is too large"))?;

            block_offsets.push(offset);

            encode_block(block, block_index, &mut data)?;
        }

        Ok(SplineEncodedData {
            data,
            block_offsets,
        })
    }
}

/// Encodes one Havok spline block.
///
/// The block layout is:
///
/// ```text
/// TransformMask[track_count]
/// align(4)
///
/// for each track:
///     position
///     align(4)
///     rotation
///     align(4)
///     scale
///
/// align(16)
/// ```
///
/// # Errors
///
/// Returns [`Error`] if any track is inconsistent with its transform mask or
/// if one of its spline payloads cannot be encoded.
fn encode_block(
    block: &TransformSplineBlock,
    block_index: usize,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    #[cfg(not(feature = "tracing"))]
    let _ = block_index;

    if block.masks.len() != block.tracks.len() {
        return Err(Error::InvalidData("mask count does not match track count"));
    }

    #[cfg(feature = "tracing")]
    let block_start = out.len();

    for mask in &block.masks {
        write_transform_mask(*mask, out);
    }

    align4(out);

    #[cfg(feature = "tracing")]
    let mut debug_tracks = Vec::with_capacity(block.tracks.len());

    for (mask, track) in block.masks.iter().zip(&block.tracks) {
        #[cfg(feature = "tracing")]
        let position_offset = out.len();

        encode_position(mask, track, out)?;

        #[cfg(feature = "tracing")]
        let position_size = out.len() - position_offset;

        align4(out);

        #[cfg(feature = "tracing")]
        let rotation_offset = out.len();

        encode_rotation(mask, track, out)?;

        #[cfg(feature = "tracing")]
        let rotation_size = out.len() - rotation_offset;

        align4(out);

        #[cfg(feature = "tracing")]
        let scale_offset = out.len();

        encode_scale(mask, track, out)?;

        #[cfg(feature = "tracing")]
        let scale_size = out.len() - scale_offset;

        #[cfg(feature = "tracing")]
        debug_tracks.push(debug::SerializeDebugTrack {
            position_offset,
            position_size,
            rotation_offset,
            rotation_size,
            scale_offset,
            scale_size,
        });
    }

    align16(out);

    #[cfg(feature = "tracing")]
    debug::log_serialized_block(block, block_index, block_start, out.len(), &debug_tracks);

    Ok(())
}

/// Writes one four-byte Havok transform mask.
#[inline]
fn write_transform_mask(mask: TransformMask, out: &mut Vec<u8>) {
    out.push(mask.quantization_types);
    out.push(mask.position_types);
    out.push(mask.rotation_types);
    out.push(mask.scale_types);
}

/// Writes the spline knot vector.
///
/// # Errors
///
/// Returns [`Error`] if a knot cannot be represented as an unsigned byte.
pub(super) fn encode_knots(knots: &[u8], out: &mut Vec<u8>) -> Result<(), Error> {
    if knots.is_empty() {
        return Err(Error::InvalidData("spline knot vector is empty"));
    }

    out.extend_from_slice(knots);

    Ok(())
}

/// Aligns a quaternion payload according to its quantization format.
#[inline]
pub(super) fn align_rotation(out: &mut Vec<u8>, quantization: QuantizationType) {
    let alignment = match quantization {
        QuantizationType::Bit40
        | QuantizationType::Bit24
        | QuantizationType::Bit8
        | QuantizationType::Bit16 => 1,

        QuantizationType::Bit48 | QuantizationType::Bit16Quat => 2,

        QuantizationType::Bit32 | QuantizationType::Uncompressed => 4,
    };

    align_to(out, alignment);
}

#[inline]
pub(super) fn align4(out: &mut Vec<u8>) {
    align_to(out, 4);
}

#[inline]
pub(super) fn align16(out: &mut Vec<u8>) {
    align_to(out, 16);
}

#[inline]
pub(super) fn align_to(out: &mut Vec<u8>, alignment: usize) {
    if alignment <= 1 {
        return;
    }

    let remainder = out.len() % alignment;

    if remainder != 0 {
        out.resize(out.len() + alignment - remainder, 0);
    }
}
