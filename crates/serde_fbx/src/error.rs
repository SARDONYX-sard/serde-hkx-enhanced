use std::path::PathBuf;

use serde_hkx_features::error::Error as SerdeHkxFeaturesError;
use snafu::Snafu;

/// Errors produced while converting FBX animations.
#[derive(Debug, Snafu)]
pub enum Error {
    /// An error occurred while deserializing an HKX/XML animation or
    /// skeleton through `serde_hkx_features`.
    #[snafu(transparent)]
    SerdeHkx { source: SerdeHkxFeaturesError },

    /// An error occurred while decoding spline-compressed animation data.
    #[snafu(transparent)]
    Spline {
        source: serde_spline::spline::SplineError,
    },

    /// The expected `hkaSplineCompressedAnimation` class was not found.
    SplineAnimationNotFound,

    /// The animation contains multiple spline blocks, which are not
    /// supported by the current conversion implementation.
    MultipleSplineBlocks { count: i32 },

    /// The number of transform tracks declared by the animation does not
    /// match the data required to construct the intermediate representation.
    InvalidTrackCount { expected: usize, actual: usize },

    /// A track refers to a bone index that does not exist in the skeleton.
    InvalidBoneIndex {
        track_index: usize,
        bone_index: usize,
        bone_count: usize,
    },

    /// The animation contains a malformed or otherwise unusable skeleton.
    InvalidSkeleton { message: String },

    /// An animation has no spline blocks even though spline animation data
    /// was expected.
    EmptySplineData,

    /// A required animation class was found, but its structure is invalid.
    InvalidSplineAnimation { message: String },

    /// An input path was required for HKX/XML error reporting.
    InvalidInputPath { path: PathBuf },

    /// The requested animation frame rate is invalid.
    #[snafu(display("invalid animation FPS: {fps}"))]
    InvalidFps {
        /// Requested animation frame rate.
        fps: f32,
    },

    /// Failed to load the FBX document.
    #[snafu(display("failed to load FBX: {message}"))]
    LoadFbx {
        /// Error reported by the FBX loader.
        message: String,
    },

    /// The FBX document contains no animation stacks.
    #[snafu(display("FBX document contains no animation stacks"))]
    NoAnimationStacks,

    /// The requested animation stack could not be found.
    #[snafu(display("FBX animation stack not found: {name}"))]
    AnimationStackNotFound {
        /// Requested animation stack name.
        name: String,
    },

    /// An FBX bone could not be mapped to the Havok skeleton.
    #[snafu(display("FBX bone is missing from the Havok skeleton: {name}"))]
    BoneNotFound {
        /// FBX bone name.
        name: String,
    },

    /// The FBX skeleton contains duplicate bone names.
    #[snafu(display("FBX skeleton contains duplicate bone name: {name}"))]
    DuplicateBone {
        /// Duplicated FBX bone name.
        name: String,
    },

    /// The FBX animation has an invalid duration.
    #[snafu(display("invalid FBX animation duration: {duration}"))]
    InvalidDuration {
        /// Animation duration in seconds.
        duration: f32,
    },

    /// The number of generated animation frames is invalid.
    #[snafu(display("invalid animation frame count: {count}"))]
    InvalidFrameCount {
        /// Generated frame count.
        count: u64,
    },

    /// HKX encoding failed.
    #[snafu(display("failed to encode HKX animation: {message}"))]
    Encode {
        /// Error returned by the HKX encoder.
        message: String,
    },

    // ser error ---
    /// The declared animation frame count does not match the number of
    /// frames supplied by the FFI intermediate representation.
    EncoderFrameCountMismatch { expected: usize, actual: usize },

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
    EncoderInvalidFps { fps: f32 },

    /// {message}
    ExportFbx { message: String },

    /// Multiple animations failed during parallel conversion.
    #[snafu(display("{} animation conversion(s) failed", errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(",\n- ")))]
    Errors {
        /// Individual conversion errors.
        errors: Vec<Self>,
    },
}
