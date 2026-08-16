/// serde_spline error
#[derive(Debug)]
pub enum Error {
    /// The input ended before the requested number of bytes was available.
    ///
    /// This usually means that the block is truncated or that an earlier
    /// value was decoded with the wrong size/alignment.
    UnexpectedEof {
        position: usize,
        requested: usize,
        remaining: usize,
        context: ReadContext,
    },

    /// The input contains an invalid quantization type.
    ///
    /// Quantization types determine how the following bytes are interpreted.
    /// An invalid value means the decoder cannot determine the layout of the
    /// following data.
    InvalidQuantizationType(u8),

    /// The input contains an invalid spline degree.
    ///
    /// The degree controls how many control points participate in the
    /// evaluation of a spline segment. This implementation supports degrees
    /// up to four.
    InvalidDegree(u8),

    /// The spline knot vector is malformed.
    ///
    /// A knot vector must contain enough entries for the declared degree and
    /// control-point count, and its denominators must be valid during basis
    /// function evaluation.
    InvalidKnotVector,

    /// The spline control-point count is invalid.
    ///
    /// A dynamic spline must contain at least one control point.
    InvalidControlPointCount,

    /// A spline evaluation index is outside the control-point array.
    ///
    /// This indicates an inconsistency between the declared degree, knot
    /// vector, and number of control points.
    InvalidControlPointIndex,

    /// The decompressed block does not contain the requested track.
    TrackOutOfRange,

    /// The input data cannot be represented by the spline format.
    ///
    /// This is primarily used by the encoder when a requested representation
    /// conflicts with the information expressible by the transform mask.
    InvalidData(&'static str),
    // -----------------------------------------------------------------------------------------------------------------
    // -- serde-hkx serialize
    /// The animation contains no frames or the skeleton contains no bones.
    EmptyAnimation,

    /// The animation frame rate is not finite or is not greater than zero.
    InvalidFps { fps: f32 },

    /// The declared animation frame count does not match the number of
    /// decoded animation frames.
    FrameCountMismatch { expected: usize, actual: usize },

    /// The animation contains more transform tracks than the skeleton
    /// contains bones.
    TrackCountExceedsBoneCount {
        track_count: usize,
        bone_count: usize,
    },

    /// An animation frame does not contain exactly one transform for each
    /// skeleton bone.
    TransformCountMismatch {
        frame_index: usize,
        expected: usize,
        actual: usize,
    },

    // -----------------------------------------------------------------------------------------------------------------
    // -- serde-hkx deserialize
    /// No `hkaSkeleton` class was found in the decoded class map.
    ///
    /// A skeleton is required to construct the intermediate skeleton
    /// representation.
    MissingSkeleton,

    /// The number of parent indices does not match the number of bones.
    ///
    /// Each bone must have exactly one corresponding parent index.
    SkeletonBoneParentIndexCountMismatch {
        bone_count: usize,
        parent_index_count: usize,
    },

    /// The number of reference-pose transforms does not match the number
    /// of bones.
    ///
    /// Each bone must have exactly one corresponding reference-pose
    /// transform.
    SkeletonReferencePoseCountMismatch {
        bone_count: usize,
        reference_pose_count: usize,
    },

    /// A reference-pose transform for a bone was not available.
    ///
    /// This indicates an inconsistent skeleton representation even when
    /// the reference-pose array length was previously validated.
    MissingReferencePose { bone_index: usize },

    /// No `hkaSplineCompressedAnimation` class was found in the decoded
    /// class map.
    MissingSplineCompressedAnimation,

    /// The spline animation declares no blocks.
    EmptySplineBlocks,

    /// The spline animation contains no data bytes.
    EmptySplineData,

    /// The number of block offsets does not match the declared number of
    /// blocks.
    SplineBlockOffsetCountMismatch {
        block_count: usize,
        block_offset_count: i32,
    },

    /// A transform track refers to a bone that does not exist.
    InvalidBoneIndex {
        track_index: usize,
        bone_index: usize,
        bone_count: usize,
    },

    /// The number of transform tracks cannot be mapped to the skeleton.
    ///
    /// This is used when the track-to-bone mapping itself is structurally
    /// inconsistent with the animation or skeleton.
    InvalidTrackMapping {
        track_count: usize,
        bone_count: usize,
    },

    // -----------------------------------------------------------------------------------------------------------------
    /// An error occurred while deserializing an HKX/XML animation or
    /// skeleton through `serde_hkx_features`.
    SerdeHkx {
        source: serde_hkx_features::error::Error,
    },
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnexpectedEof {
                position,
                requested,
                remaining,
                context,
            } => write!(
                f,
                "unexpected end of input at {position:#x}: requested {requested} bytes, \
     but only {remaining} bytes remain ({context})",
            ),
            Self::InvalidQuantizationType(value) => {
                write!(f, "invalid quantization type: {value}")
            }
            Self::InvalidDegree(value) => {
                write!(f, "invalid spline degree: {value}")
            }
            Self::InvalidKnotVector => f.write_str("invalid spline knot vector"),
            Self::InvalidControlPointCount => f.write_str("invalid spline control-point count"),
            Self::InvalidControlPointIndex => f.write_str("invalid spline control-point index"),
            Self::TrackOutOfRange => f.write_str("track index is out of range"),
            Self::InvalidData(message) => f.write_str(message),

            // -----------------------------------------------------------------------------------------------------------------
            // -- serde-hkx serialize
            Self::EmptyAnimation => f.write_str("animation contains no frames or transform tracks"),
            Self::InvalidFps { fps } => {
                write!(f, "invalid animation frame rate: {fps}")
            }
            Self::FrameCountMismatch { expected, actual } => {
                write!(
                    f,
                    "animation declares {expected} frames but contains {actual}"
                )
            }
            Self::TrackCountExceedsBoneCount {
                track_count,
                bone_count,
            } => {
                write!(
                    f,
                    "animation contains {track_count} transform tracks but \
                     the skeleton contains only {bone_count} bones"
                )
            }
            Self::TransformCountMismatch {
                frame_index,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "animation frame {frame_index} contains {actual} transforms \
                     but the skeleton requires {expected}"
                )
            }

            // -----------------------------------------------------------------------------------------------------------------
            // -- serde-hkx deserialize
            Self::MissingSkeleton => f.write_str("required class hkaSkeleton was not found"),
            Self::SkeletonBoneParentIndexCountMismatch {
                bone_count,
                parent_index_count,
            } => {
                write!(
                    f,
                    "hkaSkeleton contains {bone_count} bones but \
                     {parent_index_count} parent indices"
                )
            }
            Self::SkeletonReferencePoseCountMismatch {
                bone_count,
                reference_pose_count,
            } => {
                write!(
                    f,
                    "hkaSkeleton contains {bone_count} bones but \
                     {reference_pose_count} reference-pose transforms"
                )
            }
            Self::MissingReferencePose { bone_index } => {
                write!(
                    f,
                    "hkaSkeleton is missing the reference pose for bone {bone_index}"
                )
            }
            Self::MissingSplineCompressedAnimation => {
                f.write_str("required class hkaSplineCompressedAnimation was not found")
            }
            Self::EmptySplineBlocks => {
                f.write_str("hkaSplineCompressedAnimation contains no spline blocks")
            }
            Self::EmptySplineData => {
                f.write_str("hkaSplineCompressedAnimation contains no spline data")
            }
            Self::SplineBlockOffsetCountMismatch {
                block_count,
                block_offset_count,
            } => {
                write!(
                    f,
                    "hkaSplineCompressedAnimation declares {block_count} \
                     blocks but contains {block_offset_count} block offsets"
                )
            }
            Self::InvalidBoneIndex {
                track_index,
                bone_index,
                bone_count,
            } => {
                write!(
                    f,
                    "transform track {track_index} refers to bone {bone_index}, \
                     but the skeleton contains {bone_count} bones"
                )
            }
            Self::InvalidTrackMapping {
                track_count,
                bone_count,
            } => {
                write!(
                    f,
                    "cannot map {track_count} transform tracks to \
                     {bone_count} skeleton bones"
                )
            }
            Self::SerdeHkx { source } => write!(f, "{source}"),
        }
    }
}

impl core::error::Error for Error {}

impl From<serde_hkx_features::error::Error> for Error {
    #[inline]
    fn from(source: serde_hkx_features::error::Error) -> Self {
        Self::SerdeHkx { source }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ReadContext {
    pub block_index: Option<usize>,
    pub track_index: Option<usize>,
    pub track_type: Option<crate::spline::math::SplineTrackType>,
    pub quantization: Option<crate::spline::math::QuantizationType>,
}

impl core::fmt::Display for ReadContext {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "block={:?}", self.block_index)?;

        if let Some(track_index) = self.track_index {
            write!(f, ", track={track_index}")?;
        }

        if let Some(track_type) = self.track_type {
            write!(f, ", type={track_type:?}")?;
        }

        if let Some(quantization) = self.quantization {
            write!(f, ", quantization={quantization:?}")?;
        }

        Ok(())
    }
}
