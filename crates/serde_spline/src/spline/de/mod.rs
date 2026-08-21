// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: (C) 2016-2023 Lukas Cone
//
// Reference:
// https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.hpp
// https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.cpp

//! Havok spline animation decompression.
//!
//! The binary decoding layer is implemented with `winnow` parsers operating
//! directly on `&[u8]`. Cursor management, alignment, and endian selection
//! belong to [`Deserializer`], while binary representations such as packed
//! quaternions belong to `crate::spline::parser`.
//!
//! Mathematical reconstruction is deliberately kept separate from byte
//! loading. In particular:
//!
//! - `parser::*` describes binary representations.
//! - `Deserializer` owns the input cursor and endian.
//! - `decode_*` functions perform format-specific arithmetic.
//! - spline evaluation functions operate only on decoded values.
mod packed;
mod parser;

use core::ops::{Add, Mul};

use havok_types::{QsTransform, Quaternion, Vector4};
use winnow::{
    Parser,
    binary::{self, Endianness},
    combinator::fail,
    error::{ContextError, ErrMode, StrContext::*},
    token::take,
};

use self::parser::{quat32, quat40, quat48, transform_mask, uncompressed_quat};
use super::math::{
    QuantizationType, QuatA16, SplineDynamicTrackQuat, SplineDynamicTrackVector, SplineStaticTrack,
    SplineTrackQuat, SplineTrackType, SplineTrackVector, TransformMask, TransformSplineBlock,
    TransformTrack, TransformType,
};
use crate::error::Error;

type ParseError = ErrMode<ContextError>;

/// Binary deserializer used by the spline decoder.
///
/// The input remains an ordinary `&[u8]`. `winnow` parsers consume a temporary
/// subslice and this type tracks the number of consumed bytes so that alignment
/// and error positions remain available to the caller.
#[derive(Debug)]
struct Deserializer<'a> {
    data: &'a [u8],
    position: usize,
    endian: Endianness,
}

impl<'a> Deserializer<'a> {
    /// Creates a deserializer over `data`.
    #[inline]
    const fn new(data: &'a [u8], endian: Endianness) -> Self {
        Self {
            data,
            position: 0,
            endian,
        }
    }

    /// Returns the remaining input.
    #[inline]
    fn remaining(&self) -> &'a [u8] {
        &self.data[self.position..]
    }

    /// Parses one value and advances the deserializer cursor.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnexpectedEof`] when the parser cannot consume the
    /// required bytes, or [`Error::Parse`] when `winnow` reports a malformed
    /// input.
    #[inline]
    fn parse_next<P, O>(&mut self, mut parser: P) -> Result<O, Error>
    where
        P: Parser<&'a [u8], O, ParseError>,
    {
        let before = self.position;
        let mut input = self.remaining();

        let value = parser
            .parse_next(&mut input)
            .map_err(|error| self.map_parse_error(error))?;

        let consumed = before
            .checked_add(self.remaining().len())
            .and_then(|_| self.remaining().len().checked_sub(input.len()))
            .ok_or(Error::InvalidData("parser position overflow"))?;

        self.position = self
            .position
            .checked_add(consumed)
            .ok_or(Error::InvalidData("parser position overflow"))?;

        Ok(value)
    }

    /// Advances the cursor by `count` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnexpectedEof`] if the requested range is outside the
    /// input.
    #[inline]
    fn skip(&mut self, count: usize) -> Result<(), Error> {
        let end = self
            .position
            .checked_add(count)
            .ok_or(Error::InvalidData("skip offset overflow"))?;

        if end > self.data.len() {
            return Err(self.unexpected_eof(count));
        }

        self.position = end;

        Ok(())
    }

    /// Aligns the cursor to `alignment`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnexpectedEof`] if the required padding extends past
    /// the end of the input.
    #[inline]
    fn align(&mut self, alignment: usize) -> Result<(), Error> {
        debug_assert!(alignment.is_power_of_two());

        let mask = alignment - 1;

        let aligned = self
            .position
            .checked_add(mask)
            .ok_or(Error::InvalidData("alignment overflow"))?
            & !mask;

        self.skip(aligned - self.position)
    }

    #[inline]
    fn unexpected_eof(&self, requested: usize) -> Error {
        Error::UnexpectedEof {
            position: self.position,
            requested,
            remaining: self.data.len().saturating_sub(self.position),
            context: Default::default(),
        }
    }

    #[inline]
    fn map_parse_error(&self, error: ParseError) -> Error {
        match error {
            ErrMode::Backtrack(error) | ErrMode::Cut(error) => {
                let message = error.to_string();

                if self.data.len().saturating_sub(self.position) == 0 {
                    Error::UnexpectedEof {
                        position: self.position,
                        requested: 1,
                        remaining: 0,
                        context: Default::default(),
                    }
                } else {
                    Error::InvalidData(Box::leak(message.into_boxed_str()))
                }
            }
            ErrMode::Incomplete(_) => Error::UnexpectedEof {
                position: self.position,
                requested: 1,
                remaining: self.data.len().saturating_sub(self.position),
                context: Default::default(),
            },
        }
    }
}

/// Loads and decodes one quaternion using the mask-selected representation.
///
/// # Errors
///
/// Returns a parser error when the selected representation cannot be read.
/// Returns [`Error::InvalidQuantizationType`] when the mask selects a
/// representation unsupported by the quaternion decoder.
fn read_quat(de: &mut Deserializer<'_>, quantization: QuantizationType) -> Result<QuatA16, Error> {
    match quantization {
        QuantizationType::Bit32 => Ok(de.parse_next(quat32(de.endian))?.decode()),
        QuantizationType::Bit40 => Ok(de.parse_next(quat40())?.decode()),
        QuantizationType::Bit48 => Ok(de.parse_next(quat48(de.endian))?.decode()),
        QuantizationType::Uncompressed => {
            de.parse_next(uncompressed_quat(de.endian).context(Label("uncompressed_quat")))
        }
        _ => Err(Error::InvalidQuantizationType(quantization as u8)),
    }
}

fn find_knot_span(
    degree: usize,
    value: f32,
    control_point_count: usize,
    knots: &[f32],
) -> Result<usize, Error> {
    if control_point_count == 0 {
        return Err(Error::InvalidControlPointCount);
    }

    if knots.len() <= control_point_count {
        return Err(Error::InvalidKnotVector);
    }

    if value >= knots[control_point_count] {
        return Ok(control_point_count - 1);
    }

    let mut low = degree;
    let mut high = control_point_count;

    if low >= knots.len() || high >= knots.len() {
        return Err(Error::InvalidKnotVector);
    }

    let mut mid = (low + high) / 2;

    while value < knots[mid] || value >= knots[mid + 1] {
        if value < knots[mid] {
            high = mid;
        } else {
            low = mid;
        }

        mid = (low + high) / 2;

        if mid + 1 >= knots.len() {
            return Err(Error::InvalidKnotVector);
        }
    }

    Ok(mid)
}

fn get_single_point<T>(
    knot_span: usize,
    degree: usize,
    frame: f32,
    knots: &[f32],
    control_points: &[T],
) -> Result<T, Error>
where
    T: Copy + Add<Output = T> + Mul<f32, Output = T>,
{
    if control_points.is_empty() {
        return Err(Error::InvalidControlPointCount);
    }

    if degree > 4 {
        return Err(Error::InvalidDegree(degree as u8));
    }

    if knot_span < degree {
        return Err(Error::InvalidControlPointIndex);
    }

    let mut basis = [0.0f32; 5];
    basis[0] = 1.0;

    for i in 1..=degree {
        for j in (0..i).rev() {
            let left_index = knot_span - j;
            let right_index = knot_span + i - j;

            if right_index >= knots.len() || left_index >= knots.len() {
                return Err(Error::InvalidKnotVector);
            }

            let denominator = knots[right_index] - knots[left_index];

            if denominator == 0.0 {
                return Err(Error::InvalidKnotVector);
            }

            let a = (frame - knots[left_index]) / denominator;
            let tmp = basis[j] * a;

            basis[j + 1] += basis[j] - tmp;
            basis[j] = tmp;
        }
    }

    let first_index = knot_span
        .checked_sub(degree)
        .ok_or(Error::InvalidControlPointIndex)?;

    let first = control_points
        .get(first_index)
        .copied()
        .ok_or(Error::InvalidControlPointIndex)?;

    let mut result = first * basis[degree];

    for i in 1..=degree {
        let index = knot_span
            .checked_sub(degree - i)
            .ok_or(Error::InvalidControlPointIndex)?;

        let control_point = control_points
            .get(index)
            .copied()
            .ok_or(Error::InvalidControlPointIndex)?;

        result = result + control_point * basis[degree - i];
    }

    Ok(result)
}

fn get_single_scalar_point(
    knot_span: usize,
    degree: usize,
    frame: f32,
    knots: &[f32],
    control_points: &[f32],
) -> Result<f32, Error> {
    get_single_point(knot_span, degree, frame, knots, control_points)
}

fn evaluate_vector_track(
    track: &SplineDynamicTrackVector,
    local_frame: f32,
) -> Result<Vector4, Error> {
    let mut result = Vector4 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 0.0,
    };

    let mut knot_span = None;

    for (control_points, axis) in
        track
            .tracks
            .iter()
            .zip([&mut result.x, &mut result.y, &mut result.z])
    {
        if control_points.len() == 1 {
            *axis = control_points[0];
            continue;
        }

        let span = match knot_span {
            Some(span) => span,
            None => {
                let span = find_knot_span(
                    track.degree as usize,
                    local_frame,
                    control_points.len(),
                    &track.knots,
                )?;

                knot_span = Some(span);
                span
            }
        };

        *axis = get_single_scalar_point(
            span,
            track.degree as usize,
            local_frame,
            &track.knots,
            control_points,
        )?;
    }

    Ok(result)
}

fn evaluate_quat_track(track: &SplineDynamicTrackQuat, local_frame: f32) -> Result<QuatA16, Error> {
    let knot_span = find_knot_span(
        track.degree as usize,
        local_frame,
        track.track.len(),
        &track.knots,
    )?;

    get_single_point(
        knot_span,
        track.degree as usize,
        local_frame,
        &track.knots,
        &track.track,
    )
}

/// The original floating-point range represented by quantized values.
#[derive(Clone, Copy)]
struct TrackBbox {
    min: f32,
    max: f32,
}

fn read_spline_header<'a>(
    endian: Endianness,
) -> impl Parser<&'a [u8], (usize, u8, Vec<f32>), ErrMode<ContextError>> {
    move |input: &mut &'a [u8]| {
        let num_items = binary::u16(endian).parse_next(input)? as usize;
        let degree = binary::u8.parse_next(input)?;

        let Some(knot_count) = num_items
            .checked_add(degree as usize)
            .and_then(|value| value.checked_add(2))
        else {
            return fail
                .context(Expected(winnow::error::StrContextValue::Description(
                    "A dynamic spline must contain at least one control point.",
                )))
                .parse_next(input)?;
        };

        let knots = take(knot_count)
            .context(Label("knot vector"))
            .parse_next(input)?
            .iter()
            .map(|&value| value as f32)
            .collect::<Vec<_>>();

        Ok((num_items, degree, knots))
    }
}

fn read_dynamic_vector_track(
    de: &mut Deserializer<'_>,
    mask: TransformMask,
    quantization: QuantizationType,
    default_value: f32,
    transform_types: [TransformType; 3],
) -> Result<SplineTrackVector, Error> {
    let (num_items, degree, knots) = de.parse_next(read_spline_header(de.endian))?;

    de.align(4)?;

    let mut extremes = [
        TrackBbox { min: 0.0, max: 0.0 },
        TrackBbox { min: 0.0, max: 0.0 },
        TrackBbox { min: 0.0, max: 0.0 },
    ];

    let mut tracks = [Vec::<f32>::new(), Vec::<f32>::new(), Vec::<f32>::new()];

    for axis in 0..3 {
        match mask.sub_track_type(transform_types[axis]) {
            SplineTrackType::Dynamic => {
                extremes[axis] = TrackBbox {
                    min: de.parse_next(binary::f32(de.endian))?,
                    max: de.parse_next(binary::f32(de.endian))?,
                };

                tracks[axis].resize(num_items + 1, 0.0);
            }
            SplineTrackType::Static => {
                tracks[axis].push(de.parse_next(binary::f32(de.endian))?);
            }
            SplineTrackType::Identity => {
                tracks[axis].push(default_value);
            }
        }
    }

    #[expect(clippy::needless_range_loop)]
    for item in 0..=num_items {
        for axis in 0..3 {
            let transform_type = transform_types[axis];

            if mask.sub_track_type(transform_type) != SplineTrackType::Dynamic {
                continue;
            }

            let value = match quantization {
                QuantizationType::Bit8 => {
                    let value = de.parse_next(binary::u8)? as f32;
                    value / 255.0
                }

                QuantizationType::Bit16 => {
                    let value = de.parse_next(binary::u16(de.endian))? as f32;
                    value / 65535.0
                }

                _ => {
                    return Err(Error::InvalidQuantizationType(quantization as u8));
                }
            };

            tracks[axis][item] =
                (extremes[axis].max - extremes[axis].min).mul_add(value, extremes[axis].min);
        }
    }

    de.align(4)?;

    Ok(SplineTrackVector::Dynamic(SplineDynamicTrackVector {
        tracks,
        knots,
        degree,
    }))
}

fn read_vector_track(
    de: &mut Deserializer<'_>,
    mask: TransformMask,
    quantization: QuantizationType,
    default_value: f32,
    transform_types: [TransformType; 3],
) -> Result<SplineTrackVector, Error> {
    let dynamic = transform_types
        .iter()
        .copied()
        .any(|ty| mask.sub_track_type(ty) == SplineTrackType::Dynamic);

    if dynamic {
        return read_dynamic_vector_track(de, mask, quantization, default_value, transform_types);
    }

    let mut value = Vector4 {
        x: default_value,
        y: default_value,
        z: default_value,
        w: 0.0,
    };

    for (axis, value) in [&mut value.x, &mut value.y, &mut value.z]
        .iter_mut()
        .enumerate()
    {
        if mask.sub_track_type(transform_types[axis]) == SplineTrackType::Static {
            let component = de.parse_next(binary::f32(de.endian))?;
            **value = component;
        }
    }

    Ok(SplineTrackVector::Static(SplineStaticTrack { value }))
}

fn read_rotation_track(
    de: &mut Deserializer<'_>,
    mask: TransformMask,
) -> Result<SplineTrackQuat, Error> {
    match mask.sub_track_type(TransformType::Rotation) {
        SplineTrackType::Dynamic => {
            let (num_items, degree, knots) = de.parse_next(read_spline_header(de.endian))?;

            let quantization = mask.rotation_quantization_type()?;
            match quantization {
                QuantizationType::Bit48 | QuantizationType::Bit16Quat => de.align(2)?,
                QuantizationType::Bit32 | QuantizationType::Uncompressed => de.align(4)?,
                _ => {}
            }

            let mut track = Vec::with_capacity(num_items + 1);
            for _ in 0..=num_items {
                track.push(read_quat(de, quantization)?);
            }

            Ok(SplineTrackQuat::Dynamic(SplineDynamicTrackQuat {
                track,
                knots,
                degree,
            }))
        }

        SplineTrackType::Static => {
            let quantization = mask.rotation_quantization_type()?;
            match quantization {
                QuantizationType::Bit48 | QuantizationType::Bit16Quat => de.align(2)?,
                QuantizationType::Bit32 | QuantizationType::Uncompressed => de.align(4)?,
                _ => {}
            }

            let value = read_quat(de, quantization)?;
            Ok(SplineTrackQuat::Static(SplineStaticTrack { value }))
        }

        SplineTrackType::Identity => Ok(SplineTrackQuat::Identity),
    }
}

fn read_transform_track(
    de: &mut Deserializer<'_>,
    mask: TransformMask,
) -> Result<TransformTrack, Error> {
    let position = read_vector_track(
        de,
        mask,
        mask.position_quantization_type()?,
        0.0,
        [
            TransformType::PosX,
            TransformType::PosY,
            TransformType::PosZ,
        ],
    )?;

    let rotation = read_rotation_track(de, mask)?;

    de.align(4)?;

    let scale = read_vector_track(
        de,
        mask,
        mask.scale_quantization_type()?,
        1.0,
        [
            TransformType::ScaleX,
            TransformType::ScaleY,
            TransformType::ScaleZ,
        ],
    )?;

    Ok(TransformTrack {
        position,
        rotation,
        scale,
    })
}

impl TransformSplineBlock {
    /// Decompresses one transform spline block.
    ///
    /// The block starts with transform masks followed by the optional
    /// float-track region and aligned transform-track data.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnexpectedEof`] when the block is truncated.
    /// Returns [`Error::InvalidQuantizationType`] when a mask selects an
    /// unsupported quantization representation.
    /// Returns spline-specific errors when the declared control-point,
    /// knot-vector, or degree information is invalid.
    pub fn decode(
        data: &[u8],
        num_tracks: usize,
        num_float_tracks: usize,
        endian: Endianness,
    ) -> Result<Self, Error> {
        #[cfg(feature = "tracing")]
        tracing::debug!(
            data_len = format_args!("{:#06X}", data.len()),
            num_tracks,
            num_float_tracks,
            "TransformSplineBlock::decode"
        );

        num_tracks
            .checked_mul(core::mem::size_of::<TransformMask>())
            .ok_or(Error::InvalidData("num tracks overflow multiply"))?;

        let mut de = Deserializer::new(data, endian);

        let mut masks = Vec::with_capacity(num_tracks);

        for _ in 0..num_tracks {
            masks.push(de.parse_next(transform_mask())?);
        }

        de.skip(num_float_tracks)?;
        de.align(4)?;

        let mut tracks = Vec::with_capacity(num_tracks);

        for mask in masks.iter().copied() {
            tracks.push(read_transform_track(&mut de, mask)?);
        }

        Ok(Self { masks, tracks })
    }

    /// Evaluates one transform track at the specified local frame.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TrackOutOfRange`] when `track_id` does not exist.
    /// Returns spline evaluation errors when the selected dynamic track has an
    /// invalid knot vector, degree, or control-point layout.
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

/// A decompressed Havok spline animation.
#[derive(Clone, Debug, Default)]
pub struct SplineData {
    pub blocks: Vec<TransformSplineBlock>,
}

impl SplineData {
    /// Creates an empty spline data.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes all spline blocks from their block offsets.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptySplineData`] when an offset points outside the
    /// supplied data.
    /// Returns [`Error::UnexpectedEof`] when a block is truncated.
    /// Returns spline-specific errors when a block contains invalid data.
    pub fn decode(
        data: &[u8],
        block_offsets: &[u32],
        num_tracks: usize,
        num_float_tracks: usize,
        endian: Endianness,
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
                endian,
            )?);
        }

        Ok(Self { blocks })
    }

    /// Evaluates one track in one decompressed block.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TrackOutOfRange`] when `block_id` does not exist.
    /// The selected block propagates all errors produced by
    /// [`TransformSplineBlock::get_value`].
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
