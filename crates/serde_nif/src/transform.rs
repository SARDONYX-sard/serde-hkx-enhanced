use rig_math::{MAT3_IDENTITY, Mat3, mat3_mul, mat3_mul_vec, mat3_transpose};

/// NIF `NiTransform`: `v' = rotation * (scale * v) + translation`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub translation: [f32; 3],
    /// Row-major.
    pub rotation: Mat3,
    pub scale: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: MAT3_IDENTITY,
            scale: 1.0,
        }
    }
}

impl Transform {
    /// Inverse transform (rotation is assumed orthonormal).
    pub fn inverse(&self) -> Self {
        let rt = mat3_transpose(&self.rotation);
        let inv_scale = if self.scale.abs() > f32::EPSILON {
            1.0 / self.scale
        } else {
            0.0
        };
        let t = mat3_mul_vec(&rt, self.translation);
        Self {
            translation: t.map(|v| -v * inv_scale),
            rotation: rt,
            scale: inv_scale,
        }
    }

    /// `self * child` (apply `child` first).
    pub fn compose(&self, child: &Self) -> Self {
        let t = mat3_mul_vec(&self.rotation, child.translation);
        Self {
            translation: core::array::from_fn(|i| t[i].mul_add(self.scale, self.translation[i])),
            rotation: mat3_mul(&self.rotation, &child.rotation),
            scale: self.scale * child.scale,
        }
    }
}
