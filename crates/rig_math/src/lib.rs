//! Math shared by the Havok, FBX and NIF crates.
//!
//! - [`Vec4A16`], [`IVec4A16`], [`UVec4A16`], [`QuatA16`]: 4-lane SIMD values
//!   used by the spline codec, where the bit-exact quantization tricks of
//!   Havok are easiest to express as lane operations.
//! - [`Mat4`]: `f64` affine matrices for scene graphs (FBX), where precision
//!   matters more than speed because transforms are composed many times.
//! - [`Mat3`] helpers: NIF's 3x3 rotations.
//! - [`quat_to_euler_xyz`] / [`compatible_euler`]: for formats animating Euler angles.
mod euler;
mod ivec4;
mod mat3;
mod mat4;
mod quat;
mod simd;
mod uvec4;
mod vec4;

pub use self::euler::{compatible_euler, quat_to_euler_xyz};
pub use self::ivec4::IVec4A16;
pub use self::mat3::{MAT3_IDENTITY, Mat3, mat3_mul, mat3_mul_vec, mat3_transpose};
pub use self::mat4::{Mat4, rotation_to_quat};
pub use self::quat::QuatA16;
pub use self::uvec4::UVec4A16;
pub use self::vec4::Vec4A16;
