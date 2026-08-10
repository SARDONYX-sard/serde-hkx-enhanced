use serde_hkx_features::error::Error as SerdeHkxFeaturesError;

#[derive(Debug)]
pub enum Error {
    Errors {
        errors: Vec<Self>,
    },

    /// An error occurred while deserializing an HKX/XML animation or
    /// skeleton through `serde_hkx_features`.
    SerdeHkx {
        source: SerdeHkxFeaturesError,
    },

    /// An error occurred while decoding spline-compressed animation data.
    Spline {
        source: serde_spline::error::Error,
    },

    /// An error reported by the native niflib conversion.(C++ FFI)
    Niflib {
        message: String,
    },
}

impl From<SerdeHkxFeaturesError> for Error {
    #[inline]
    fn from(source: SerdeHkxFeaturesError) -> Self {
        Self::SerdeHkx { source }
    }
}

impl From<serde_spline::error::Error> for Error {
    #[inline]
    fn from(source: serde_spline::error::Error) -> Self {
        Self::Spline { source }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Errors { errors } => {
                write!(
                    f,
                    "{}",
                    errors
                        .iter()
                        .map(|e| e.to_string())
                        .collect::<Vec<String>>()
                        .join(",\n- ")
                )
            }
            Self::SerdeHkx { source } => write!(f, "{source}"),
            Self::Spline { source } => write!(f, "spline decoding failed: {source}"),
            Self::Niflib { message } => write!(f, "niflib conversion failed: {message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SerdeHkx { source } => Some(source),
            Self::Spline { source } => Some(source),
            _ => None,
        }
    }
}
