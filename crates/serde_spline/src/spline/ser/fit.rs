// SPDX-License-Identifier: GPL-3.0-or-later
//
// Reference:
// - https://github.com/BadDogSkyrim/PyNifly/blob/7fd4644f5a6416c1502983b7d49a853eb0d24509/io_scene_nifly/hkx/anim_skyrim.py

//! Least-squares fitting of sampled frames to clamped B-splines (as PyNifly does).

use crate::error::Error;
use crate::spline::bspline;

/// Fits scalar frame samples to a B-spline.
///
/// This follows PyNifly's exact-interpolation approach: there is one control
/// point for every frame sample and the collocation points are `0..N-1`.
///
/// # Errors
///
/// Returns [`Error`] if the frame count is zero, the knot vector is invalid,
/// or the interpolation matrix contains a singular pivot.
pub(super) fn fit_bspline_scalar(
    degree: usize,
    knots: &[u8],
    frame_values: &[f32],
) -> Result<Vec<f32>, Error> {
    let n_cp = frame_values.len();

    if n_cp == 0 {
        return Err(Error::InvalidData("cannot fit an empty scalar spline"));
    }

    if knots.len() != n_cp + degree + 1 {
        return Err(Error::InvalidData(
            "B-spline knot count does not match frame sample count",
        ));
    }

    let knots = knots.iter().map(|&value| value as f32).collect::<Vec<_>>();

    let mut matrix = Vec::with_capacity(n_cp);

    for frame in 0..n_cp {
        matrix.push(bspline_basis_row(degree, frame as f32, n_cp, &knots)?);
    }

    let mut rhs = frame_values.to_vec();

    solve_banded(&mut matrix, &mut rhs)?;

    Ok(rhs)
}

/// Builds PyNifly's clamped uniform knot vector.
///
/// The knot range is `[0, n_cp - 1]` and interior knots are integer-valued.
///
/// # Errors
///
/// Returns [`Error`] if the control-point count is invalid or if the resulting
/// knot count cannot be represented.
pub(super) fn make_clamped_knots(n_cp: usize, degree: usize) -> Result<Vec<u8>, Error> {
    if n_cp == 0 {
        return Err(Error::InvalidData(
            "B-spline requires at least one control point",
        ));
    }

    if degree == 0 {
        return Err(Error::InvalidData("B-spline degree must not be zero"));
    }

    if n_cp <= degree {
        return Err(Error::InvalidData(
            "B-spline control point count must exceed degree",
        ));
    }

    let max_t = n_cp - 1;

    if max_t > u8::MAX as usize {
        return Err(Error::InvalidData(
            "B-spline knot values exceed the u8 representation",
        ));
    }

    let knot_count = n_cp
        .checked_add(degree)
        .and_then(|value| value.checked_add(1))
        .ok_or(Error::InvalidData("B-spline knot count overflow"))?;

    let interior_count = n_cp
        .checked_sub(degree)
        .and_then(|value| value.checked_sub(1))
        .ok_or(Error::InvalidData("invalid B-spline interior knot count"))?;

    let mut knots = Vec::with_capacity(knot_count);

    knots.extend(core::iter::repeat_n(0u8, degree + 1));

    for index in 0..interior_count {
        let value = ((index + 1) * max_t + interior_count.div_ceil(2)) / (interior_count + 1);

        let value = u8::try_from(value)
            .map_err(|_| Error::InvalidData("B-spline knot exceeds u8 range"))?;

        knots.push(value);
    }

    let max_t =
        u8::try_from(max_t).map_err(|_| Error::InvalidData("B-spline knot exceeds u8 range"))?;

    knots.extend(core::iter::repeat_n(max_t, degree + 1));

    if knots.len() != knot_count {
        return Err(Error::InvalidData(
            "generated B-spline knot count is invalid",
        ));
    }

    Ok(knots)
}


/// Evaluates every B-spline basis function at `t`: one row of the fitting
/// matrix (only `degree + 1` entries are non-zero).
///
/// # Errors
///
/// Returns [`Error`] if the knot vector does not match `n_cp` and `degree`.
pub(super) fn bspline_basis_row(degree: usize, t: f32, n_cp: usize, knots: &[f32]) -> Result<Vec<f32>, Error> {
    if degree == 0 {
        return Err(Error::InvalidData("B-spline degree must not be zero"));
    }
    if knots.len() != n_cp + degree + 1 {
        return Err(Error::InvalidData("invalid B-spline knot vector length"));
    }

    let span = bspline::find_knot_span(degree, t, n_cp, knots)?;
    let weights = bspline::basis(span, degree, t, knots)?;

    let mut row = vec![0.0f32; n_cp];
    for (i, &weight) in weights.iter().enumerate().take(degree + 1) {
        row[span - i] = weight;
    }
    Ok(row)
}
/// Solves the interpolation system using PyNifly's banded Gaussian
/// elimination with partial pivoting.
///
/// # Errors
///
/// Returns [`Error`] if the matrix is empty or contains a singular pivot.
pub(super) fn solve_banded(matrix: &mut [Vec<f32>], rhs: &mut [f32]) -> Result<(), Error> {
    let n = rhs.len();

    if n == 0 || matrix.len() != n {
        return Err(Error::InvalidData("invalid B-spline interpolation matrix"));
    }

    for row in matrix.iter() {
        if row.len() != n {
            return Err(Error::InvalidData(
                "B-spline interpolation matrix is not square",
            ));
        }
    }

    for col in 0..n {
        let mut max_row = col;
        let mut max_value = matrix[col][col].abs();

        #[expect(clippy::needless_range_loop)]
        for row in (col + 1)..n.min(col + 8) {
            let value = matrix[row][col].abs();

            if value > max_value {
                max_value = value;
                max_row = row;
            }
        }

        if max_row != col {
            matrix.swap(col, max_row);
            rhs.swap(col, max_row);
        }

        let pivot = matrix[col][col];

        if pivot.abs() < 1.0e-30 {
            return Err(Error::InvalidData(
                "B-spline interpolation matrix is singular",
            ));
        }

        for row in (col + 1)..n.min(col + 8) {
            let factor = matrix[row][col] / pivot;

            if factor.abs() < 1.0e-30 {
                continue;
            }

            #[expect(clippy::needless_range_loop)]
            for k in col..n.min(col + 8) {
                matrix[row][k] = factor.mul_add(-matrix[col][k], matrix[row][k]);
            }

            rhs[row] = factor.mul_add(-rhs[col], rhs[row]);
        }
    }

    for row in (0..n).rev() {
        let mut value = rhs[row];

        for column in (row + 1)..n.min(row + 8) {
            value = matrix[row][column].mul_add(-rhs[column], value);
        }

        let pivot = matrix[row][row];

        if pivot.abs() < 1.0e-30 {
            return Err(Error::InvalidData(
                "B-spline back substitution encountered a singular pivot",
            ));
        }

        rhs[row] = value / pivot;
    }

    Ok(())
}


/// Fits four quaternion components independently to the same B-spline.
///
/// # Errors
///
/// Returns [`Error`] if any scalar component cannot be fitted.
pub(super) fn fit_bspline_quaternion(
    degree: usize,
    knots: &[u8],
    samples: &[[f32; 4]],
) -> Result<Vec<[f32; 4]>, Error> {
    let mut components = [
        Vec::<f32>::with_capacity(samples.len()),
        Vec::<f32>::with_capacity(samples.len()),
        Vec::<f32>::with_capacity(samples.len()),
        Vec::<f32>::with_capacity(samples.len()),
    ];

    for sample in samples {
        for component in 0..4 {
            components[component].push(sample[component]);
        }
    }

    let fitted = [
        fit_bspline_scalar(degree, knots, &components[0])?,
        fit_bspline_scalar(degree, knots, &components[1])?,
        fit_bspline_scalar(degree, knots, &components[2])?,
        fit_bspline_scalar(degree, knots, &components[3])?,
    ];

    let mut result = Vec::with_capacity(samples.len());

    #[expect(clippy::needless_range_loop)]
    for index in 0..samples.len() {
        result.push([
            fitted[0][index],
            fitted[1][index],
            fitted[2][index],
            fitted[3][index],
        ]);
    }

    Ok(result)
}

