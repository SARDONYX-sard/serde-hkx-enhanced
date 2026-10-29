// SPDX-License-Identifier: GPL-3.0-or-later
//
// Reference:
// - https://github.com/BadDogSkyrim/PyNifly/blob/7fd4644f5a6416c1502983b7d49a853eb0d24509/io_scene_nifly/hkx/anim_skyrim.py

//! Position and scale tracks: static values and quantized B-spline control points.

use havok_types::Vector4;

use super::fit::{fit_bspline_scalar, make_clamped_knots};
use super::{align4, encode_knots};
use crate::error::Error;
use crate::spline::track::{
    QuantizationType, SplineDynamicTrackVector, SplineTrackType, SplineTrackVector,
    TransformMask, TransformTrack, TransformType,
};

/// Encodes a position track.
///
/// # Errors
///
/// Returns [`Error`] if the position quantization type is invalid or if the
/// track representation does not match its mask.
pub(super) fn encode_position(
    mask: &TransformMask,
    track: &TransformTrack,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    let quantization = mask.position_quantization_type()?;

    encode_vector_track(
        mask,
        &track.position,
        [
            TransformType::PosX,
            TransformType::PosY,
            TransformType::PosZ,
        ],
        quantization,
        0.0,
        out,
    )
}

/// Encodes a scale track.
///
/// # Errors
///
/// Returns [`Error`] if the scale quantization type is invalid or if the
/// track representation does not match its mask.
pub(super) fn encode_scale(
    mask: &TransformMask,
    track: &TransformTrack,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    let quantization = mask.scale_quantization_type()?;

    encode_vector_track(
        mask,
        &track.scale,
        [
            TransformType::ScaleX,
            TransformType::ScaleY,
            TransformType::ScaleZ,
        ],
        quantization,
        1.0,
        out,
    )
}

/// Encodes a position or scale track.
///
/// # Errors
///
/// Returns [`Error`] if the track representation is inconsistent with the
/// mask, if values are invalid, or if the scalar quantization type is not
/// supported.
pub(super) fn encode_vector_track(
    mask: &TransformMask,
    source: &SplineTrackVector,
    types: [TransformType; 3],
    quantization: QuantizationType,
    default: f32,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    let dynamic = types
        .iter()
        .any(|&ty| mask.sub_track_type(ty) == SplineTrackType::Dynamic);

    match source {
        SplineTrackVector::Static(track) => {
            if dynamic {
                return Err(Error::InvalidData(
                    "vector track is static while its mask contains a dynamic axis",
                ));
            }

            encode_static_vector(
                mask,
                types,
                vector4_to_f32_array(&track.value),
                default,
                out,
            )
        }

        SplineTrackVector::Dynamic(track) => {
            if !dynamic {
                return Err(Error::InvalidData(
                    "vector track is dynamic while its mask contains no dynamic axis",
                ));
            }

            encode_dynamic_vector(mask, types, track, quantization, default, out)
        }
    }
}

/// Converts the three relevant components of a Havok vector.
#[inline]
pub(super) const fn vector4_to_f32_array(value: &Vector4) -> [f32; 3] {
    [value.x, value.y, value.z]
}

/// Encodes a vector containing only static and identity axes.
///
/// # Errors
///
/// Returns [`Error`] if a static component is non-finite, an identity
/// component does not equal its Havok default, or a dynamic component is
/// present in a static vector representation.
pub(super) fn encode_static_vector(
    mask: &TransformMask,
    types: [TransformType; 3],
    values: [f32; 3],
    default: f32,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    for axis in 0..3 {
        match mask.sub_track_type(types[axis]) {
            SplineTrackType::Static => {
                let value = values[axis];

                if !value.is_finite() {
                    return Err(Error::InvalidData(
                        "static vector contains a non-finite value",
                    ));
                }

                out.extend_from_slice(&value.to_le_bytes());
            }

            SplineTrackType::Identity =>
            {
                #[expect(
                    clippy::float_cmp,
                    reason = "Identity tracks must contain the exact Havok default value."
                )]
                if values[axis] != default {
                    return Err(Error::InvalidData(
                        "identity vector component differs from its default value",
                    ));
                }
            }

            SplineTrackType::Dynamic => {
                return Err(Error::InvalidData(
                    "static vector contains a dynamic mask component",
                ));
            }
        }
    }

    Ok(())
}

/// Encodes a dynamic vector spline.
///
/// The values in `track.tracks` are frame samples. They are fitted to a
/// degree-1 B-spline before quantization, matching PyNifly's exporter.
///
/// # Errors
///
/// Returns [`Error`] if the dynamic axes contain different sample counts, if
/// there are too many samples, if a sample is non-finite, if the B-spline
/// system cannot be solved, or if the quantization type is unsupported.
pub(super) fn encode_dynamic_vector(
    mask: &TransformMask,
    types: [TransformType; 3],
    track: &SplineDynamicTrackVector,
    quantization: QuantizationType,
    default: f32,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    let sample_count = types
        .iter()
        .enumerate()
        .find_map(|(axis, &ty)| {
            (mask.sub_track_type(ty) == SplineTrackType::Dynamic)
                .then_some(track.tracks[axis].len())
        })
        .ok_or(Error::InvalidData(
            "dynamic vector contains no dynamic axis",
        ))?;

    if sample_count == 0 {
        return Err(Error::InvalidData(
            "dynamic vector contains no frame samples",
        ));
    }

    if sample_count <= 1 {
        return Err(Error::InvalidData(
            "dynamic vector requires at least two frame samples",
        ));
    }

    if sample_count > u16::MAX as usize + 1 {
        return Err(Error::InvalidData("too many vector spline frame samples"));
    }

    #[expect(clippy::needless_range_loop)]
    for axis in 0..3 {
        match mask.sub_track_type(types[axis]) {
            SplineTrackType::Dynamic => {
                if track.tracks[axis].len() != sample_count {
                    return Err(Error::InvalidData(
                        "dynamic vector axes have different frame sample counts",
                    ));
                }

                if track.tracks[axis].iter().any(|value| !value.is_finite()) {
                    return Err(Error::InvalidData(
                        "dynamic vector contains a non-finite value",
                    ));
                }
            }

            SplineTrackType::Static => {
                if track.tracks[axis].len() != 1 {
                    return Err(Error::InvalidData(
                        "static vector axis must contain exactly one value",
                    ));
                }

                if !track.tracks[axis][0].is_finite() {
                    return Err(Error::InvalidData(
                        "static vector contains a non-finite value",
                    ));
                }
            }

            SplineTrackType::Identity => {
                #[expect(
                    clippy::float_cmp,
                    reason = "Identity tracks must contain the exact Havok default value."
                )]
                if track.tracks[axis].iter().any(|&value| value != default) {
                    return Err(Error::InvalidData(
                        "identity vector component differs from its default value",
                    ));
                }
            }
        }
    }

    let degree = 1usize;
    let knots = make_clamped_knots(sample_count, degree)?;

    let num_items = u16::try_from(sample_count - 1)
        .map_err(|_| Error::InvalidData("too many vector spline frame samples"))?;

    out.extend_from_slice(&num_items.to_le_bytes());
    out.push(degree as u8);

    encode_knots(&knots, out)?;
    align4(out);

    let mut bounds = [[0.0f32; 2]; 3];
    let mut fitted = [None, None, None];

    for axis in 0..3 {
        match mask.sub_track_type(types[axis]) {
            SplineTrackType::Dynamic => {
                let values = &track.tracks[axis];

                let min = values
                    .iter()
                    .copied()
                    .reduce(f32::min)
                    .ok_or(Error::InvalidData("dynamic vector axis is empty"))?;

                let max = values
                    .iter()
                    .copied()
                    .reduce(f32::max)
                    .ok_or(Error::InvalidData("dynamic vector axis is empty"))?;

                let max = if (max - min).abs() < 1.0e-30 {
                    min + 1.0e-6
                } else {
                    max
                };

                bounds[axis] = [min, max];

                out.extend_from_slice(&min.to_le_bytes());
                out.extend_from_slice(&max.to_le_bytes());

                fitted[axis] = Some(fit_bspline_scalar(degree, &knots, values)?);
            }

            SplineTrackType::Static => {
                out.extend_from_slice(&track.tracks[axis][0].to_le_bytes());
            }

            SplineTrackType::Identity => {}
        }
    }

    for control_point in 0..sample_count {
        for axis in 0..3 {
            if mask.sub_track_type(types[axis]) != SplineTrackType::Dynamic {
                continue;
            }

            let values = fitted[axis]
                .as_ref()
                .ok_or(Error::InvalidData("missing fitted vector control points"))?;

            let value = *values.get(control_point).ok_or(Error::InvalidData(
                "vector control point index is out of bounds",
            ))?;

            let [min, max] = bounds[axis];

            write_quantized_scalar(out, value, min, max, quantization)?;
        }
    }

    align4(out);

    Ok(())
}


/// Writes one scalar using Havok's scalar quantization.
///
/// # Errors
///
/// Returns [`Error`] if the value or bounds are non-finite, if the bounds are
/// invalid, or if the requested quantization is not an 8-bit or 16-bit scalar
/// format.
pub(super) fn write_quantized_scalar(
    out: &mut Vec<u8>,
    value: f32,
    min: f32,
    max: f32,
    quantization: QuantizationType,
) -> Result<(), Error> {
    if !value.is_finite() || !min.is_finite() || !max.is_finite() {
        return Err(Error::InvalidData("cannot quantize a non-finite scalar"));
    }

    let range = max - min;

    let normalized = if range.abs() < 1.0e-30 {
        0.0
    } else {
        ((value - min) / range).clamp(0.0, 1.0)
    };

    match quantization {
        QuantizationType::Bit8 => {
            let encoded = (normalized * 255.0).round() as u8;
            out.push(encoded);
        }

        QuantizationType::Bit16 => {
            let encoded = (normalized * 65535.0).round() as u16;
            out.extend_from_slice(&encoded.to_le_bytes());
        }

        _ => {
            return Err(Error::InvalidData("invalid scalar quantization type"));
        }
    }

    Ok(())
}

