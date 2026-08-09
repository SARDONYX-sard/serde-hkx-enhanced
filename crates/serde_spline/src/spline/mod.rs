mod de;
mod de_alt;
pub mod math;
pub mod ser;
pub mod skeleton;

pub use self::de::{SplineDecompressor, SplineError};
pub use self::de_alt::de_spline_from_hkx_or_xml;

// ---------------------------------------------------------------------------
// Quantization types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub enum ScalarQuantizationType {
    Bits8 = 0,
    Bits16 = 1,
}

#[derive(Debug, Clone, Copy)]
pub enum RotationQuantizationType {
    /// 4 bytes, align 4
    Polar32 = 0,
    /// 5 bytes, align 1
    ThreeComp40 = 1,
    /// 6 bytes, align 2
    ThreeComp48 = 2,
    /// 3 bytes, align 1 – not implemented
    ThreeComp24 = 3,
    /// 2 bytes, align 2 – not implemented
    Straight16 = 4,
    /// 16 bytes, align 4
    Uncompressed = 5,
}

impl RotationQuantizationType {
    /// Returns the byte-alignment required before reading quantized quaternions,
    /// or `None` for unsupported variants.
    ///
    /// # Unimplemented variants (ThreeComp24 / Straight16).
    #[inline]
    pub(crate) const fn rotation_align(self) -> Option<usize> {
        Some(match self {
            Self::ThreeComp40 => 1,
            Self::ThreeComp48 => 2,
            Self::Polar32 | Self::Uncompressed => 4,
            _ => return None, // Unsupported
        })
    }
}

// ---------------------------------------------------------------------------
// FlagOffset bitmask
// ---------------------------------------------------------------------------

bitflags::bitflags! {
    /// Per-axis channel flags packed into one byte.
    ///
    /// Lower nibble = static (constant) channels.
    /// Upper nibble = spline (animated) channels.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FlagOffset: u8 {
        const STATIC_X = 0b0000_0001;
        const STATIC_Y = 0b0000_0010;
        const STATIC_Z = 0b0000_0100;
        const STATIC_W = 0b0000_1000;
        const SPLINE_X = 0b0001_0000;
        const SPLINE_Y = 0b0010_0000;
        const SPLINE_Z = 0b0100_0000;
        const SPLINE_W = 0b1000_0000;
    }
}

macro_rules! bail {
    ($msg:expr) => {{
        return Err(serde_hkx_features::error::Error::SerError {
            input: std::path::PathBuf::from("test"),
            source: Box::new(serde_hkx_features::serde::ser::SerError::Hkx {
                source: <serde_hkx::errors::ser::Error as havok_serde::ser::Error>::custom($msg),
                location: snafu::location!(),
            }),
        });
    }};
}
pub(crate) use bail;
