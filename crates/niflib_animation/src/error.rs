use std::path::PathBuf;

use serde_hkx_features::error::Error as SerdeHkxFeaturesError;
use serde_spline::spline::SplineError;

#[derive(Debug)]
pub enum Error {
    Errors {
        errors: Vec<Self>,
    },

    /// An error occurred while deserializing an HKX/XML animation or
    /// skeleton through `serde_hkx_features`.
    SerdeHkx(SerdeHkxFeaturesError),

    /// An error occurred while decoding spline-compressed animation data.
    Spline(SplineError),

    /// The expected `hkaSplineCompressedAnimation` class was not found.
    SplineAnimationNotFound,

    /// The animation contains multiple spline blocks, which are not
    /// supported by the current conversion implementation.
    MultipleSplineBlocks {
        count: i32,
    },

    /// The number of transform tracks declared by the animation does not
    /// match the data required to construct the intermediate representation.
    InvalidTrackCount {
        expected: usize,
        actual: usize,
    },

    /// A track refers to a bone index that does not exist in the skeleton.
    InvalidBoneIndex {
        track_index: usize,
        bone_index: usize,
        bone_count: usize,
    },

    /// The animation contains a malformed or otherwise unusable skeleton.
    InvalidSkeleton {
        message: String,
    },

    /// An animation has no spline blocks even though spline animation data
    /// was expected.
    EmptySplineData,

    /// A required animation class was found, but its structure is invalid.
    InvalidSplineAnimation {
        message: String,
    },

    /// An input path was required for HKX/XML error reporting.
    InvalidInputPath {
        path: PathBuf,
    },

    /// An error reported by the native niflib conversion.(C++ FFI)
    Niflib {
        message: String,
    },

    // ser error ---
    /// The declared animation frame count does not match the number of
    /// frames supplied by the FFI intermediate representation.
    EncoderFrameCountMismatch {
        expected: usize,
        actual: usize,
    },

    /// The declared transform-track count does not match the number of
    /// transforms in an animation frame.
    EncoderTrackCountMismatch {
        frame_index: usize,
        expected: usize,
        actual: usize,
    },

    /// The animation contains no frames even though transform animation data
    /// is required.
    EncoderEmptyAnimation,

    /// An animation frame contains a different number of transforms from
    /// the declared transform-track count.
    EncoderTransformCountMismatch {
        frame_index: usize,
        expected: usize,
        actual: usize,
    },

    /// The requested animation frame rate is not finite or is not positive.
    EncoderInvalidFps {
        fps: f32,
    },
}

impl From<SerdeHkxFeaturesError> for Error {
    fn from(error: SerdeHkxFeaturesError) -> Self {
        Self::SerdeHkx(error)
    }
}

impl From<SplineError> for Error {
    fn from(error: SplineError) -> Self {
        Self::Spline(error)
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

            Self::SerdeHkx(error) => write!(f, "{error}"),

            Self::Spline(error) => write!(f, "spline decoding failed: {error}"),

            Self::SplineAnimationNotFound => {
                f.write_str("hkaSplineCompressedAnimation was not found")
            }

            Self::MultipleSplineBlocks { count } => {
                write!(f, "multiple spline blocks are unsupported: {count}")
            }

            Self::InvalidTrackCount { expected, actual } => {
                write!(
                    f,
                    "invalid transform track count: expected {expected}, got {actual}"
                )
            }

            Self::InvalidBoneIndex {
                track_index,
                bone_index,
                bone_count,
            } => {
                write!(
                    f,
                    "track {track_index} refers to invalid bone index {bone_index} \
                     (bone count: {bone_count})"
                )
            }

            Self::InvalidSkeleton { message } => {
                write!(f, "invalid skeleton: {message}")
            }

            Self::EmptySplineData => f.write_str("spline animation contains no spline data"),

            Self::InvalidSplineAnimation { message } => {
                write!(f, "invalid spline animation: {message}")
            }

            Self::InvalidInputPath { path } => {
                write!(f, "invalid input path: {}", path.display())
            }

            Self::Niflib { message } => {
                write!(f, "niflib conversion failed: {message}")
            }

            Self::EncoderFrameCountMismatch { expected, actual } => {
                write!(
                    f,
                    "encoder frame count mismatch: expected {expected}, got {actual}"
                )
            }

            Self::EncoderTrackCountMismatch {
                frame_index,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "encoder track count mismatch at frame {frame_index}: \
         expected {expected}, got {actual}"
                )
            }

            Self::EncoderEmptyAnimation => f.write_str("cannot encode an animation with no frames"),

            Self::EncoderTransformCountMismatch {
                frame_index,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "encoder transform count mismatch at frame {frame_index}: \
         expected {expected}, got {actual}"
                )
            }

            Self::EncoderInvalidFps { fps } => {
                write!(f, "invalid animation frame rate for encoder: {fps}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SerdeHkx(error) => Some(error),
            Self::Spline(error) => Some(error),
            _ => None,
        }
    }
}
