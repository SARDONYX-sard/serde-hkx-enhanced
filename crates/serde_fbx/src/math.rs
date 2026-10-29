//! Minimal affine math for FBX/NIF transforms.
//!
//! Matrices act on column vectors (`v' = M * v`) and are stored row-major:
//! `m[row][col]`, translation in `m[0..3][3]`.
#![allow(clippy::suboptimal_flops, reason = "matrix formulas stay readable as written")]

use havok_types::{QsTransform, Quaternion, Vector4};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat4(pub [[f64; 4]; 4]);

impl Default for Mat4 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mat4 {
    pub const IDENTITY: Self = Self([
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]);

    pub const fn translation(t: [f64; 3]) -> Self {
        let mut m = Self::IDENTITY;
        m.0[0][3] = t[0];
        m.0[1][3] = t[1];
        m.0[2][3] = t[2];
        m
    }

    pub const fn scale(s: [f64; 3]) -> Self {
        let mut m = Self::IDENTITY;
        m.0[0][0] = s[0];
        m.0[1][1] = s[1];
        m.0[2][2] = s[2];
        m
    }

    /// Rotation from a 3x3 row-major matrix.
    pub const fn from_rotation(r: [[f64; 3]; 3]) -> Self {
        Self([
            [r[0][0], r[0][1], r[0][2], 0.0],
            [r[1][0], r[1][1], r[1][2], 0.0],
            [r[2][0], r[2][1], r[2][2], 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    /// Unit quaternion `(x, y, z, w)` to rotation.
    pub fn from_quat([x, y, z, w]: [f64; 4]) -> Self {
        let len = (x * x + y * y + z * z + w * w).sqrt();
        let (x, y, z, w) = if len > f64::EPSILON {
            (x / len, y / len, z / len, w / len)
        } else {
            (0.0, 0.0, 0.0, 1.0)
        };

        Self::from_rotation([
            [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
            [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
            [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
        ])
    }

    /// FBX Euler rotation in degrees.
    ///
    /// `order` is FBX `RotationOrder`: 0 = XYZ (X applied first, `Rz*Ry*Rx`),
    /// 1 = XZY, 2 = YZX, 3 = YXZ, 4 = ZXY, 5 = ZYX, 6 = spheric XYZ.
    pub fn from_euler_degrees(euler: [f64; 3], order: i64) -> Self {
        let [x, y, z] = euler.map(f64::to_radians);
        let rx = Self::axis_rotation(0, x);
        let ry = Self::axis_rotation(1, y);
        let rz = Self::axis_rotation(2, z);

        match order {
            1 => ry * rz * rx,
            2 => rx * rz * ry,
            3 => rz * rx * ry,
            4 => ry * rx * rz,
            5 => rx * ry * rz,
            _ => rz * ry * rx,
        }
    }

    fn axis_rotation(axis: usize, angle: f64) -> Self {
        let (s, c) = angle.sin_cos();
        match axis {
            0 => Self::from_rotation([[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]),
            1 => Self::from_rotation([[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]),
            _ => Self::from_rotation([[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]),
        }
    }

    pub fn from_qs_transform(t: &QsTransform) -> Self {
        let tr = &t.transition;
        let q = &t.quaternion;
        let s = &t.scale;
        Self::translation([f64::from(tr.x), f64::from(tr.y), f64::from(tr.z)])
            * Self::from_quat([q.x, q.y, q.z, q.scaler].map(f64::from))
            * Self::scale([f64::from(s.x), f64::from(s.y), f64::from(s.z)])
    }

    pub const fn get_translation(&self) -> [f64; 3] {
        [self.0[0][3], self.0[1][3], self.0[2][3]]
    }

    /// Splits into translation, unit quaternion `(x, y, z, w)`, and scale.
    ///
    /// Shear is discarded.
    pub fn decompose(&self) -> ([f64; 3], [f64; 4], [f64; 3]) {
        let m = &self.0;
        let column = |c: usize| [m[0][c], m[1][c], m[2][c]];
        let length = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();

        let mut scale = [length(column(0)), length(column(1)), length(column(2))];
        if self.determinant3() < 0.0 {
            scale[0] = -scale[0];
        }

        let mut r = [[0.0; 3]; 3];
        for (c, &s) in scale.iter().enumerate() {
            for (row, r_row) in r.iter_mut().enumerate() {
                r_row[c] = if s.abs() > f64::EPSILON { m[row][c] / s } else { 0.0 };
            }
        }

        (self.get_translation(), rotation_to_quat(r), scale)
    }

    pub fn to_qs_transform(&self) -> QsTransform {
        let (t, q, s) = self.decompose();
        QsTransform {
            transition: Vector4 {
                x: t[0] as f32,
                y: t[1] as f32,
                z: t[2] as f32,
                w: 0.0,
            },
            quaternion: Quaternion {
                x: q[0] as f32,
                y: q[1] as f32,
                z: q[2] as f32,
                scaler: q[3] as f32,
            },
            scale: Vector4 {
                x: s[0] as f32,
                y: s[1] as f32,
                z: s[2] as f32,
                w: 1.0,
            },
        }
    }

    fn determinant3(&self) -> f64 {
        let m = &self.0;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    /// Inverse of an affine matrix (last row `0 0 0 1`).
    pub fn inverse(&self) -> Self {
        let m = &self.0;
        let det = self.determinant3();
        if det.abs() < f64::MIN_POSITIVE {
            return Self::IDENTITY;
        }
        let inv_det = 1.0 / det;

        let mut r = [[0.0; 3]; 3];
        r[0][0] = (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * inv_det;
        r[0][1] = (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inv_det;
        r[0][2] = (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv_det;
        r[1][0] = (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * inv_det;
        r[1][1] = (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv_det;
        r[1][2] = (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * inv_det;
        r[2][0] = (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * inv_det;
        r[2][1] = (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * inv_det;
        r[2][2] = (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inv_det;

        let t = self.get_translation();
        let mut out = Self::from_rotation(r);
        for (row, r_row) in r.iter().enumerate() {
            out.0[row][3] = -(r_row[0] * t[0] + r_row[1] * t[1] + r_row[2] * t[2]);
        }
        out
    }

    /// Column-major flat array, the FBX `Matrix` / `Transform` layout.
    pub fn to_fbx_array(&self) -> Vec<f64> {
        (0..4)
            .flat_map(|c| (0..4).map(move |r| (r, c)))
            .map(|(r, c)| self.0[r][c])
            .collect()
    }
}

impl core::ops::Mul for Mat4 {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self {
        let mut out = [[0.0; 4]; 4];
        for (r, out_row) in out.iter_mut().enumerate() {
            for (c, out) in out_row.iter_mut().enumerate() {
                *out = (0..4).map(|k| self.0[r][k] * rhs.0[k][c]).sum();
            }
        }
        Self(out)
    }
}

/// Rotation matrix to unit quaternion `(x, y, z, w)`.
pub fn rotation_to_quat(r: [[f64; 3]; 3]) -> [f64; 4] {
    let trace = r[0][0] + r[1][1] + r[2][2];
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [
            (r[2][1] - r[1][2]) / s,
            (r[0][2] - r[2][0]) / s,
            (r[1][0] - r[0][1]) / s,
            0.25 * s,
        ]
    } else if r[0][0] > r[1][1] && r[0][0] > r[2][2] {
        let s = (1.0 + r[0][0] - r[1][1] - r[2][2]).sqrt() * 2.0;
        [
            0.25 * s,
            (r[0][1] + r[1][0]) / s,
            (r[0][2] + r[2][0]) / s,
            (r[2][1] - r[1][2]) / s,
        ]
    } else if r[1][1] > r[2][2] {
        let s = (1.0 + r[1][1] - r[0][0] - r[2][2]).sqrt() * 2.0;
        [
            (r[0][1] + r[1][0]) / s,
            0.25 * s,
            (r[1][2] + r[2][1]) / s,
            (r[0][2] - r[2][0]) / s,
        ]
    } else {
        let s = (1.0 + r[2][2] - r[0][0] - r[1][1]).sqrt() * 2.0;
        [
            (r[0][2] + r[2][0]) / s,
            (r[1][2] + r[2][1]) / s,
            0.25 * s,
            (r[1][0] - r[0][1]) / s,
        ]
    };

    let len = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    q.map(|v| v / len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: &Mat4, b: &Mat4) -> bool {
        a.0.iter()
            .flatten()
            .zip(b.0.iter().flatten())
            .all(|(x, y)| (x - y).abs() < 1.0e-9)
    }

    #[test]
    fn decompose_roundtrip() {
        let m = Mat4::translation([1.0, 2.0, 3.0])
            * Mat4::from_euler_degrees([30.0, -45.0, 120.0], 0)
            * Mat4::scale([2.0, 2.0, 2.0]);
        let (t, q, s) = m.decompose();
        let rebuilt = Mat4::translation(t) * Mat4::from_quat(q) * Mat4::scale(s);
        assert!(near(&m, &rebuilt));
        assert!(near(&(m * m.inverse()), &Mat4::IDENTITY));
    }
}
