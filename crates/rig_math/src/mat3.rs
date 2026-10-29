//! 3x3 `f32` rotation matrices, as stored by NIF transforms.
//!
//! Plain arrays rather than a newtype: NIF blocks are read straight into
//! `[[f32; 3]; 3]`, and these few helpers are all the scene graph needs.

/// Row-major 3x3 matrix acting on column vectors (`v' = M * v`).
pub type Mat3 = [[f32; 3]; 3];

pub const MAT3_IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// `a * b`
pub fn mat3_mul(a: &Mat3, b: &Mat3) -> Mat3 {
    core::array::from_fn(|r| core::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}

/// `m * v`
pub fn mat3_mul_vec(m: &Mat3, v: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|r| (0..3).map(|k| m[r][k] * v[k]).sum())
}

/// Transpose, which is the inverse for the orthonormal rotations NIF stores.
pub fn mat3_transpose(m: &Mat3) -> Mat3 {
    core::array::from_fn(|r| core::array::from_fn(|c| m[c][r]))
}
