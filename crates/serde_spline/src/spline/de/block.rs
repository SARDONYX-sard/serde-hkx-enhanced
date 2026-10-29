// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: (C) 2016-2023 Lukas Cone
//
// Reference:
// - https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.cpp

//! Decoding and sampling of one [`TransformSplineBlock`].

use core::mem::size_of;

use havok_types::{QsTransform, Quaternion};
use rig_math::QuatA16;

use super::reader::Reader;
use super::tracks::{read_transform_mask, read_transform_track};
use crate::error::{Error, ReadContext};
use crate::spline::bspline::{evaluate_quat_track, evaluate_vector_track};
use crate::spline::track::{
    SplineTrackQuat, SplineTrackVector, TransformMask, TransformSplineBlock,
};

impl TransformSplineBlock {
    /// Decompresses one transform spline block.
    ///
    /// A block is decoded in the same order in which the binary stores it:
    ///
    /// ```text
    /// +-----------------------------+
    /// | TransformMask × num_tracks  |
    /// +-----------------------------+
    /// | float-track region          |
    /// +-----------------------------+
    /// | alignment padding            |
    /// +-----------------------------+
    /// | transform track data        |
    /// |                             |
    /// |   position                  |
    /// |   rotation                  |
    /// |   alignment                 |
    /// |   scale                     |
    /// |                             |
    /// |   ... next track ...        |
    /// +-----------------------------+
    /// ```
    ///
    /// The masks must be read first because they determine the representation
    /// and therefore the number of bytes belonging to every subsequent track.
    ///
    /// # Errors
    ///
    /// Returns [`SplineError::UnexpectedEof`] if the block ends before the
    /// declared data is available. Other [`SplineError`] variants indicate
    /// malformed mask, quantization, spline, or track data.
    pub fn decode(data: &[u8], num_tracks: usize, num_float_tracks: usize) -> Result<Self, Error> {
        #[cfg(feature = "tracing")]
        tracing::debug!(
            data_len = format_args!("{:#06X}", data.len()),
            num_tracks,
            num_float_tracks,
            "SplineDecompressor::decode"
        );

        // Every transform track begins with a four-byte mask.
        num_tracks
            .checked_mul(size_of::<TransformMask>())
            .ok_or(Error::InvalidData("num tracks overflow multiply"))?;

        let mut reader = Reader::new(data);

        let mut masks = Vec::with_capacity(num_tracks);

        for _ in 0..num_tracks {
            masks.push(read_transform_mask(&mut reader)?);
        }

        reader.skip(num_float_tracks)?; // TODO: Support float_tracks
        reader.align(4)?; // The first transform track starts on a four-byte boundary.

        let mut tracks = Vec::with_capacity(num_tracks);
        for (track_index, mask) in masks.iter().copied().enumerate() {
            reader.context = ReadContext {
                block_index: None,
                track_index: Some(track_index),
                track_type: None,
                quantization: None,
            };

            tracks.push(read_transform_track(&mut reader, mask)?);
        }

        Ok(Self { masks, tracks })
    }

    /// Evaluates one transform track at the specified local frame.
    ///
    /// # Errors
    /// If not found track_id
    pub fn get_value(&self, track_id: usize, time: f32) -> Result<QsTransform, Error> {
        let track = self.tracks.get(track_id).ok_or(Error::TrackOutOfRange)?;

        let transition = match &track.position {
            SplineTrackVector::Static(track) => track.value.clone(),
            SplineTrackVector::Dynamic(track) => evaluate_vector_track(track, time)?,
        };

        let rotation = match &track.rotation {
            SplineTrackQuat::Static(track) => track.value,
            SplineTrackQuat::Dynamic(track) => evaluate_quat_track(track, time)?,
            SplineTrackQuat::Identity => QuatA16::identity(),
        };

        let scale = match &track.scale {
            SplineTrackVector::Static(track) => track.value.clone(),
            SplineTrackVector::Dynamic(track) => evaluate_vector_track(track, time)?,
        };

        Ok(QsTransform {
            transition,
            quaternion: Quaternion::from(rotation),
            scale,
        })
    }
}
