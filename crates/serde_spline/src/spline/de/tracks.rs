// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: (C) 2016-2023 Lukas Cone
//
// Reference:
// - https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.cpp

//! Reads the position / rotation / scale tracks of one transform.

use havok_types::Vector4;

use super::quat::read_quat;
use super::reader::Reader;
use crate::error::Error;
use crate::spline::track::{
    QuantizationType, SplineDynamicTrackQuat, SplineDynamicTrackVector, SplineStaticTrack,
    SplineTrackQuat, SplineTrackType, SplineTrackVector, TransformMask, TransformTrack,
    TransformType,
};

pub(super) fn read_transform_mask(reader: &mut Reader<'_>) -> Result<TransformMask, Error> {
    #[cfg(feature = "tracing")]
    tracing::trace!("enter read_transform_mask");

    Ok(TransformMask {
        quantization_types: reader.read_u8()?,
        position_types: reader.read_u8()?,
        rotation_types: reader.read_u8()?,
        scale_types: reader.read_u8()?,
    })
}

/// The original floating-point range represented by the quantized values.
///
/// A quantized control point is stored as an integer, not as an f32:
///
///     integer -> normalized [0, 1] -> [min, max]
///
/// Without these two values the original numerical scale could not be
/// reconstructed.
#[derive(Clone, Copy)]
pub(super) struct TrackBbox {
    min: f32,
    max: f32,
}

pub(super) fn read_dynamic_vector_track(
    reader: &mut Reader<'_>,
    mask: TransformMask,
    quantization: QuantizationType,
    default_value: f32,
    transform_types: [TransformType; 3],
) -> Result<SplineTrackVector, Error> {
    // `num_items` is one less than the number of control points.
    //
    // The actual number of control points is therefore:
    //
    //     control_points = num_items + 1
    //
    // This convention is used by the binary format and is also why the
    // evaluation loop later uses `0..=num_items`.
    let num_items = reader.read_u16_le()? as usize;
    // The byte after `numItems` is not a reserved byte.
    // The C++ code reads `numItems` through `buffer++`, then advances `buffer` by one more byte.
    // - test: https://godbolt.org/z/M47ez1dfY
    let degree = reader.read_u8()?;

    // For a B-spline, the knot vector must contain enough entries to describe
    // both the control points and the polynomial degree.
    //
    // Havok's representation uses:
    //
    //     knot_count = num_items + degree + 2
    //
    // Do not derive this from a generic B-spline implementation without also
    // checking the reference binary format: the serialized count is part of
    // Havok's representation.
    let knot_count = num_items
        .checked_add(degree as usize)
        .and_then(|value| value.checked_add(2))
        .ok_or(Error::InvalidControlPointCount)?;

    // Knots are stored as bytes in this representation.
    //
    // They are promoted to f32 because spline evaluation operates in the
    // floating-point domain. The binary representation is therefore not a
    // general-purpose f32 knot vector.
    let knots = reader
        .read_bytes(knot_count)?
        .iter()
        .map(|&value| value as f32)
        .collect::<Vec<_>>();

    reader.align(4)?;

    let mut extremes = [
        TrackBbox { min: 0.0, max: 0.0 },
        TrackBbox { min: 0.0, max: 0.0 },
        TrackBbox { min: 0.0, max: 0.0 },
    ];

    let mut tracks = [Vec::<f32>::new(), Vec::<f32>::new(), Vec::<f32>::new()];

    for axis in 0..3 {
        match mask.sub_track_type(transform_types[axis]) {
            SplineTrackType::Dynamic => {
                // A dynamic component stores its control points in normalized
                // quantized form. These two floats restore the component's
                // original numerical range.
                //
                // Example:
                //
                //     stored = 128
                //     normalized = 128 / 255
                //     value = min + normalized * (max - min)
                extremes[axis] = TrackBbox {
                    min: reader.read_f32_le()?,
                    max: reader.read_f32_le()?,
                };

                tracks[axis].resize(num_items + 1, 0.0);
            }

            SplineTrackType::Static => {
                // A static component has exactly one f32 value. It does not
                // need a spline because the value is constant for every frame.
                tracks[axis].push(reader.read_f32_le()?);
            }

            SplineTrackType::Identity => {
                // Identity components are omitted from the binary completely.
                // Their value is supplied by the caller as `default_value`
                // (0.0 for position and 1.0 for scale).
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

            // The control point is stored as a normalized integer.
            //
            // Quantization deliberately reduces precision in exchange for
            // fewer bytes. This is therefore reconstruction, not a bit-exact
            // recovery of the original f32.
            let value = match quantization {
                QuantizationType::Bit8 => {
                    let value = reader.read_u8()? as f32;
                    value / 255.0
                }

                QuantizationType::Bit16 => {
                    let value = reader.read_u16_le()? as f32;
                    value / 65535.0
                }

                _ => {
                    return Err(Error::InvalidQuantizationType(quantization as u8));
                }
            };

            // Convert the normalized quantized value back into the original numerical range.
            tracks[axis][item] =
                (extremes[axis].max - extremes[axis].min).mul_add(value, extremes[axis].min);
        }
    }

    reader.align(4)?;

    Ok(SplineTrackVector::Dynamic(SplineDynamicTrackVector {
        tracks,
        knots,
        degree,
    }))
}

pub(super) fn read_vector_track(
    reader: &mut Reader<'_>,
    mask: TransformMask,
    quantization: QuantizationType,
    default_value: f32,
    transform_types: [TransformType; 3],
) -> Result<SplineTrackVector, Error> {
    // C++ `useSpline`
    let dynamic = transform_types
        .iter()
        .copied()
        .any(|ty| mask.sub_track_type(ty) == SplineTrackType::Dynamic);

    if dynamic {
        return read_dynamic_vector_track(
            reader,
            mask,
            quantization,
            default_value,
            transform_types,
        );
    }

    let mut value = Vector4 {
        x: default_value,
        y: default_value,
        z: default_value,
        w: 0.0,
    };

    for axis in [0, 1, 2] {
        if mask.sub_track_type(transform_types[axis]) == SplineTrackType::Static {
            let component = reader.read_f32_le()?;

            match axis {
                0 => value.x = component,
                1 => value.y = component,
                2 => value.z = component,
                _ => unreachable!(),
            }
        }
    }

    Ok(SplineTrackVector::Static(SplineStaticTrack { value }))
}

pub(super) fn read_rotation_track(
    reader: &mut Reader<'_>,
    mask: TransformMask,
) -> Result<SplineTrackQuat, Error> {
    match mask.sub_track_type(TransformType::Rotation) {
        SplineTrackType::Dynamic => {
            let num_items = reader.read_u16_le()? as usize;
            let degree = reader.read_u8()?;

            let knot_count = num_items
                .checked_add(degree as usize)
                .and_then(|value| value.checked_add(2))
                .ok_or(Error::InvalidControlPointCount)?;

            let knots = reader
                .read_bytes(knot_count)?
                .iter()
                .map(|&value| value as f32)
                .collect::<Vec<_>>();

            let quantization = mask.rotation_quantization_type()?;

            match quantization {
                QuantizationType::Bit48 | QuantizationType::Bit16Quat => reader.align(2)?,
                QuantizationType::Bit32 | QuantizationType::Uncompressed => reader.align(4)?,
                _ => {}
            }

            let mut track = Vec::with_capacity(num_items + 1);

            for _ in 0..=num_items {
                track.push(read_quat(reader, quantization)?);
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
                QuantizationType::Bit48 | QuantizationType::Bit16Quat => reader.align(2)?,
                QuantizationType::Bit32 | QuantizationType::Uncompressed => reader.align(4)?,
                _ => {}
            }

            let value = read_quat(reader, quantization)?;
            Ok(SplineTrackQuat::Static(SplineStaticTrack { value }))
        }

        SplineTrackType::Identity => Ok(SplineTrackQuat::Identity),
    }
}

pub(super) fn read_transform_track(
    reader: &mut Reader<'_>,
    mask: TransformMask,
) -> Result<TransformTrack, Error> {
    let position = read_vector_track(
        reader,
        mask,
        mask.position_quantization_type()?,
        0.0,
        [
            TransformType::PosX,
            TransformType::PosY,
            TransformType::PosZ,
        ],
    )?;

    let rotation = read_rotation_track(reader, mask)?;

    reader.align(4)?;

    let scale = read_vector_track(
        reader,
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
