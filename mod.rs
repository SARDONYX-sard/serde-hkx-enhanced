//! Math types
//!
//! # Examples
//! - `Matrix3`
mod matrix3;
mod matrix4;
mod qs_transform;
mod quaternion;
mod rotation;
mod transform;
mod vector4;

pub use matrix3::Matrix3;
pub use matrix4::Matrix4;
pub use qs_transform::QsTransform;
pub use quaternion::Quaternion;
pub use rotation::Rotation;
pub use transform::Transform;
pub use vector4::Vector4;

/// Implements component-wise arithmetic operators for a struct.
///
/// This macro generates implementations for:
///
/// - `Add<Self>`
/// - `Sub<Self>`
/// - `Mul<Self>`
/// - `Div<Self>`
///
/// Each operation is applied independently to the specified fields.
///
/// # Example
///
/// ```ignore
/// struct Vector4 {
///     x: f32,
///     y: f32,
///     z: f32,
///     w: f32,
/// }
///
/// impl_component_ops!(Vector4 {
///     x,
///     y,
///     z,
///     w,
/// });
///
/// let a = Vector4 { x: 1.0, y: 2.0, z: 3.0, w: 4.0 };
/// let b = Vector4 { x: 2.0, y: 3.0, z: 4.0, w: 5.0 };
///
/// let c = a + b;
/// assert_eq!(c.x, 3.0);
/// ```
#[macro_export]
macro_rules! impl_component_ops {
    ($type:ty { $($field:ident),+ $(,)? }) => {
        impl std::ops::Add for $type {
            type Output = Self;

            fn add(self, rhs: Self) -> Self {
                Self {
                    $(
                        $field: self.$field + rhs.$field,
                    )+
                }
            }
        }

        impl std::ops::Sub for $type {
            type Output = Self;

            fn sub(self, rhs: Self) -> Self {
                Self {
                    $(
                        $field: self.$field - rhs.$field,
                    )+
                }
            }
        }

        impl std::ops::Mul for $type {
            type Output = Self;

            fn mul(self, rhs: Self) -> Self {
                Self {
                    $(
                        $field: self.$field * rhs.$field,
                    )+
                }
            }
        }

        impl std::ops::Div for $type {
            type Output = Self;

            fn div(self, rhs: Self) -> Self {
                Self {
                    $(
                        $field: self.$field / rhs.$field,
                    )+
                }
            }
        }
    };
}

impl_component_ops!(Vector4 { x, y, z, w });
