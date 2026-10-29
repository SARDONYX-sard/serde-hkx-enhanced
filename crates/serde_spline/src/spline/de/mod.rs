// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: (C) 2016-2023 Lukas Cone
//
// Reference:
// https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.hpp
// https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.cpp

//! Havok spline animation decompression.
//!
//! # Overview
//!
//! A Havok spline block is a compressed representation of animation tracks.
//! It does not normally store one complete `QsTransform` for every frame.
//!
//! Instead, a track can store:
//!
//! - `Static`: one value which is used for the entire track.
//! - `Dynamic`: a small number of spline control points.
//! - `Identity`: no value at all; the component uses its predefined default.
//!
//! For a dynamic track, the stored control points are evaluated with a
//! B-spline to obtain the value at a particular frame.
//!
//! Conceptually:
//!
//! ```text
//!
//!   animation frames
//!   0   1   2   3   4   5   6   7
//!   |   |   |   |   |   |   |   |
//!   |   |   |   |   |   |   |   |
//!   +---+---+---+---+---+---+---+
//!              B-spline
//!                  |
//!                  v
//!          evaluated animation value
//!
//! ```
//!
//! The important distinction is that the binary does not necessarily contain
//! the value for each frame. It contains enough information to reconstruct a
//! smooth curve.
//!
//! # B-spline
//!
//! A B-spline is a curve defined by control points and a knot vector.
//!
//! For example, instead of storing:
//!
//! ```text
//! frame:          0     1     2     3     4     5
//! value:         1.0   1.2   1.8   2.7   3.1   3.0
//! ```
//!
//! the format can store a much smaller set of control points:
//!
//! ```text
//! control points:
//!
//!       P0        P1        P2        P3
//!        *---------*---------*---------*
//!          \      / \       /
//!           \    /   \     /
//!            \__/     \___/
//!
//!              B-spline
//! ```
//!
//! The knot vector determines which control points influence each part of
//! the curve. `degree` determines the polynomial degree of the spline.
//!
//! During decoding, `find_knot_span()` finds the part of the knot vector
//! containing the requested frame, and `get_single_point()` evaluates the
//! corresponding B-spline basis functions.
//!
//! # Quantization
//!
//! Dynamic scalar tracks usually do not store their control points directly
//! as `f32` values.
//!
//! Instead, the encoder first chooses a real-valued range:
//
//! ```text
//!             min                         max
//!              |---------------------------|
//!              0                           1
//! ```
//!
//! and stores each value as a small integer representing its position inside
//! that range.
//!
//! For 8-bit quantization:
//
//! ```text
//! stored value = 0..=255
//!
//! normalized = stored / 255.0
//!
//! decoded = min + normalized * (max - min)
//! ```
//!
//! Therefore:
//!
//! ```text
//! stored integer
//!       |
//!       v
//!   0 .. 255
//!       |
//!       | normalize
//!       v
//!   0.0 .. 1.0
//!       |
//!       | restore range
//!       v
//!   min .. max
//! ```
//!
//! Sixteen-bit quantization performs the same operation with `0..=65535`.
//!
//! This is intentionally lossy. The decoded value is an approximation of
//! the original value because many possible `f32` values map to the same
//! quantized integer.
//!
//! `TrackBbox { min, max }` stores the information required to map the
//! normalized integer back into the original numerical range.
//!
//! # Quaternion compression
//!
//! Rotation tracks are different from position and scale tracks.
//!
//! A quaternion contains four components, but a normalized quaternion has
//! only three independent degrees of freedom. The fourth component can be
//! reconstructed from the unit-length constraint:
//!
//! ```text
//! x² + y² + z² + w² = 1
//! ```
//!
//! Havok therefore provides specialized quaternion encodings such as
//! 32-bit, 40-bit, and 48-bit representations.
//!
//! These are not ordinary floating-point quantization formats. They pack
//! quaternion information into a fixed number of bits and reconstruct an
//! approximate normalized quaternion during decoding.
//!
//! The `read32_quat()`, `read40_quat()`, and `read48_quat()` functions are
//! therefore intentionally specialized and should be kept close to the
//! corresponding binary specification/reference implementation.
//!
//! `Uncompressed` is the simple case: four IEEE-754 `f32` values are stored
//! directly.
//!
//! # Track layout
//!
//! Each transform track contains three logical components:
//!
//! ```text
//! TransformTrack
//! ├── position
//! │   ├── X
//! │   ├── Y
//! │   └── Z
//! ├── rotation
//! │   └── quaternion
//! └── scale
//!     ├── X
//!     ├── Y
//!     └── Z
//! ```
//!
//! Each component is independently classified as `Static`, `Dynamic`, or
//! `Identity` by `TransformMask`.
//!
//! This is important because the binary does not have a fixed representation
//! for every transform. The mask determines which bytes exist after it.
//!
//! For example:
//
//! ```text
//! position X = Static     -> one f32 exists
//! position Y = Identity   -> no bytes exist
//! position Z = Dynamic    -> control-point data exists
//! ```
//!
//! The decoder must therefore inspect the mask before deciding what to read.
//!
//! # Alignment
//!
//! Some parts of the Havok format are aligned to 2- or 4-byte boundaries.
//! `Reader::align()` advances over padding bytes; those bytes are not part of
//! the logical animation data.
//!
//! Alignment must be handled at the same places as the reference format.
//! Reading the correct value with the wrong alignment will shift every
//! subsequent read and corrupt the remainder of the block.
//!
//! # Decode pipeline
//!
//! The complete process is:
//
//! ```text
//! binary block
//!      |
//!      v
//! TransformMask
//!      |
//!      +--------------------+
//!      |                    |
//!      v                    v
//! position/scale        rotation
//!      |                    |
//!      |                    +--> quaternion decoding
//!      |
//!      +--> static value
//!      |
//!      +--> identity/default value
//!      |
//!      +--> quantized control points
//!                    |
//!                    v
//!                B-spline
//!                    |
//!                    v
//!             evaluated value
//!                    |
//!                    +---------+
//!                              |
//!                              v
//!                        QsTransform
//! ```
//!
//! The purpose of this module is therefore not to reproduce the original
//! floating-point values bit-for-bit. The binary format itself deliberately
//! trades precision for a smaller representation.
//!
//! The decoder reconstructs the value represented by the compressed spline,
//! within the precision allowed by the selected quantization format.

mod block;
mod quat;
mod reader;
mod tracks;

use havok_types::QsTransform;

use crate::error::Error;
use crate::spline::track::TransformSplineBlock;

/// A decompressed Havok spline animation.
#[derive(Clone, Debug, Default)]
pub struct SplineDecompressor {
    pub blocks: Vec<TransformSplineBlock>,
}

impl SplineDecompressor {
    /// Creates an empty spline decompressor.
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes all spline blocks from their block offsets.
    ///
    /// # Errors
    /// Unexpected data.
    pub fn decode(
        data: &[u8],
        block_offsets: &[u32],
        num_tracks: usize,
        num_float_tracks: usize,
    ) -> Result<Self, Error> {
        if block_offsets.is_empty() {
            return Ok(Self { blocks: Vec::new() });
        }

        let mut blocks = Vec::with_capacity(block_offsets.len());

        for &offset in block_offsets {
            let offset = offset as usize;

            let block_data = data.get(offset..).ok_or(Error::EmptySplineData)?;

            blocks.push(TransformSplineBlock::decode(
                block_data,
                num_tracks,
                num_float_tracks,
            )?);
        }

        Ok(Self { blocks })
    }

    /// Evaluates one track in one decompressed block.
    ///
    /// # Errors
    /// Out of range block id
    pub fn get_value(
        &self,
        block_id: usize,
        track_id: usize,
        time: f32,
    ) -> Result<QsTransform, Error> {
        self.blocks
            .get(block_id)
            .ok_or(Error::TrackOutOfRange)?
            .get_value(track_id, time)
    }
}
