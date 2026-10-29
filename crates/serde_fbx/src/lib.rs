//! Convert Autodesk FBX animations into Havok HKX animations.

mod error;

pub mod de;
pub mod nif;
pub mod ser;

pub use error::Error;
pub use rig_math::Mat4;
