//! Havok animations ⇄ Gamebryo KF files.
//!
//! The KF format itself (reading and writing `NiControllerSequence`) lives in
//! [`serde_nif::kf`]; this crate maps between it and the Havok animation model:
//!
//! - [`to_kf`]: every skeleton bone becomes one controlled block whose keys are
//!   the decoded animation frames.
//! - [`from_kf`]: KF keys are sampled at a fixed rate into Havok track order and
//!   spline-compressed.
mod de;
mod error;
mod ser;

pub use self::de::{DeConfig, from_kf, kf_to_animation};
pub use self::error::Error;
pub use self::ser::to_kf;
