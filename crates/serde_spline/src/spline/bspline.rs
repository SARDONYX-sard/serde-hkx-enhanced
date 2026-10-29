// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: (C) 2016-2023 Lukas Cone
//
// Reference:
// - https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.cpp

//! B-spline evaluation shared by the decoder (sampling) and the encoder
//! (least-squares fitting needs the same basis rows).
//!
//! A degree-`p` spline value at `t` is a weighted sum of the `p + 1` control
//! points around the knot span containing `t`:
//!
//! ```text
//!   P(span-p) ... P(span-1) P(span)
//!         \         |       /
//!          weights from the Cox-de Boor recurrence
//! ```

use core::ops::{Add, Mul};

use havok_types::Vector4;
use rig_math::QuatA16;

use crate::error::Error;
use crate::spline::track::{SplineDynamicTrackQuat, SplineDynamicTrackVector};

/// Havok stores the degree in a byte but only ever uses 1..=3; 4 bounds the
/// fixed-size basis array.
pub const MAX_DEGREE: usize = 4;

/// Index `span` such that `knots[span] <= value < knots[span + 1]`
/// (NURBS Book, algorithm A2.1). Values past the end clamp to the last span.
///
/// # Errors
/// If there are no control points or the knot vector is too short.
pub fn find_knot_span(
    degree: usize,
    value: f32,
    control_point_count: usize,
    knots: &[f32],
) -> Result<usize, Error> {
    if control_point_count == 0 {
        return Err(Error::InvalidControlPointCount);
    }
    if knots.len() <= control_point_count || degree >= control_point_count {
        return Err(Error::InvalidKnotVector);
    }
    if value >= knots[control_point_count] {
        return Ok(control_point_count - 1);
    }

    let (mut low, mut high) = (degree, control_point_count);
    let mut mid = (low + high) / 2;
    while value < knots[mid] || value >= knots[mid + 1] {
        if value < knots[mid] {
            high = mid;
        } else {
            low = mid;
        }
        mid = (low + high) / 2;
    }
    Ok(mid)
}

/// Non-zero basis weights at `t` for `span`: `weights[i]` belongs to control
/// point `span - i`.
///
/// Repeated knots give `0 / 0` terms; they are treated as 0, the usual
/// convention, which leaves the partition of unity intact.
///
/// # Errors
/// If the degree is unsupported or `span` does not fit the knot vector.
pub fn basis(
    span: usize,
    degree: usize,
    t: f32,
    knots: &[f32],
) -> Result<[f32; MAX_DEGREE + 1], Error> {
    if degree > MAX_DEGREE {
        return Err(Error::InvalidDegree(degree as u8));
    }
    if span < degree || span + degree >= knots.len() {
        return Err(Error::InvalidKnotVector);
    }

    let mut weights = [0.0f32; MAX_DEGREE + 1];
    weights[0] = 1.0;
    for i in 1..=degree {
        for j in (0..i).rev() {
            let (left, right) = (knots[span - j], knots[span + i - j]);
            let denominator = right - left;
            let a = if denominator.abs() > f32::EPSILON {
                (t - left) / denominator
            } else {
                0.0
            };
            let tmp = weights[j] * a;
            weights[j + 1] += weights[j] - tmp;
            weights[j] = tmp;
        }
    }
    Ok(weights)
}

/// Spline value at `t` from the control points around `span`.
///
/// # Errors
/// If the span, degree or knot vector does not match the control points.
pub fn evaluate<T>(
    span: usize,
    degree: usize,
    t: f32,
    knots: &[f32],
    control_points: &[T],
) -> Result<T, Error>
where
    T: Copy + Add<Output = T> + Mul<f32, Output = T>,
{
    let weights = basis(span, degree, t, knots)?;
    let point = |i: usize| {
        control_points
            .get(span - i)
            .copied()
            .ok_or(Error::InvalidControlPointIndex)
    };

    let mut value = point(0)? * weights[0];
    for (i, &weight) in weights.iter().enumerate().take(degree + 1).skip(1) {
        value = value + point(i)? * weight;
    }
    Ok(value)
}

/// Position or scale at `t`. X/Y/Z are independent scalar splines sharing
/// one knot vector, so the span is searched once; an axis with a single
/// control point is constant.
///
/// # Errors
/// See [`evaluate`].
pub fn evaluate_vector_track(track: &SplineDynamicTrackVector, t: f32) -> Result<Vector4, Error> {
    let degree = usize::from(track.degree);
    let mut span = None;
    let mut axes = [0.0f32; 3];

    for (axis, control_points) in axes.iter_mut().zip(&track.tracks) {
        if let [constant] = control_points.as_slice() {
            *axis = *constant;
            continue;
        }
        let span = match span {
            Some(span) => span,
            None => *span.insert(find_knot_span(
                degree,
                t,
                control_points.len(),
                &track.knots,
            )?),
        };
        *axis = evaluate(span, degree, t, &track.knots, control_points)?;
    }

    let [x, y, z] = axes;
    Ok(Vector4 { x, y, z, w: 0.0 })
}

/// Rotation at `t`.
///
/// Blending control points linearly does not keep unit length, so the result
/// is normalized as Havok does; Euler conversion downstream relies on it.
///
/// # Errors
/// See [`evaluate`].
pub fn evaluate_quat_track(track: &SplineDynamicTrackQuat, t: f32) -> Result<QuatA16, Error> {
    let degree = usize::from(track.degree);
    let span = find_knot_span(degree, t, track.track.len(), &track.knots)?;
    evaluate(span, degree, t, &track.knots, &track.track).map(QuatA16::normalize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_spline_interpolates_control_points() {
        // Degree 1, clamped: the spline passes through every control point.
        let knots = [0.0, 0.0, 1.0, 2.0, 2.0];
        let points = [10.0f32, 20.0, 40.0];
        for (t, expected) in [
            (0.0, 10.0),
            (0.5, 15.0),
            (1.0, 20.0),
            (1.5, 30.0),
            (2.0, 40.0),
        ] {
            let span = find_knot_span(1, t, points.len(), &knots).unwrap();
            assert_eq!(
                evaluate(span, 1, t, &knots, &points).unwrap(),
                expected,
                "t = {t}"
            );
        }
    }

    #[test]
    fn basis_is_partition_of_unity() {
        let knots = [0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0, 3.0, 3.0];
        for t in [0.0, 0.3, 1.7, 2.9] {
            let span = find_knot_span(3, t, 6, &knots).unwrap();
            let sum: f32 = basis(span, 3, t, &knots).unwrap().iter().sum();
            assert!((sum - 1.0).abs() < 1.0e-6, "t = {t}: {sum}");
        }
    }
}
