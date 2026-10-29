//! Convert Autodesk FBX animations into Havok HKX animations.

mod error;
mod math;

pub mod de;
pub mod nif;
pub mod ser;

pub use error::Error;
pub use math::Mat4;
