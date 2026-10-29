use serde_hkx_features::error::Error as SerdeHkxFeaturesError;

#[derive(Debug)]
pub enum Error {
    /// An error occurred while encoding the HKX animation through `serde_hkx_features`.
    SerdeHkx { source: SerdeHkxFeaturesError },

    /// An error occurred while encoding spline-compressed animation data.
    Spline { source: serde_spline::error::Error },

    /// The KF file could not be read.
    Nif { source: serde_nif::Error },

    /// The animation, skeleton or KF content cannot be converted.
    Invalid { message: String },
}

impl Error {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid {
            message: message.into(),
        }
    }
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

impl From<serde_nif::Error> for Error {
    #[inline]
    fn from(source: serde_nif::Error) -> Self {
        Self::Nif { source }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SerdeHkx { source } => write!(f, "{source}"),
            Self::Spline { source } => write!(f, "spline encoding failed: {source}"),
            Self::Nif { source } => write!(f, "failed to read KF: {source}"),
            Self::Invalid { message } => write!(f, "KF conversion failed: {message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SerdeHkx { source } => Some(source),
            Self::Spline { source } => Some(source),
            Self::Nif { source } => Some(source),
            Self::Invalid { .. } => None,
        }
    }
}
