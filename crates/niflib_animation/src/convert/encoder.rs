//! Encodes the FFI animation representation into a Havok
//! `hkaSplineCompressedAnimation`.

use std::borrow::Cow;

use havok_classes::{
    Classes, hkRootLevelContainer, hkRootLevelContainerNamedVariant, hkaAnnotationTrack,
    hkaAnnotationTrackAnnotation, hkaSplineCompressedAnimation,
};
use havok_types::{NULL_STR, Pointer, StringPtr};
use serde_hkx::{HavokSort as _, bytes::serde::hkx_header::HkxHeader};
use serde_hkx_features::ClassMap;
use serde_spline::spline::SplineDecompressor;

use crate::convert::ser_builder;
use crate::error::Error;
use crate::ffi::{Animation, AnimationAnnotation, Skeleton};

/// Encodes an FFI animation into a spline-compressed Havok animation.
///
/// The transform frames are converted back into spline transform tracks.
/// Annotation tracks are supplied separately because KF animations do not
/// provide Havok annotation data.
///
/// # Errors
///
/// Returns [`Error`] when the animation dimensions are inconsistent, spline
/// compression fails, the HKX class graph cannot be constructed, or HKX
/// serialization fails.
pub(crate) fn encode<'ser>(
    skeleton: &'ser Skeleton,
    animation: &'ser Animation,
    fps: f32,
    annotations: &'ser [AnimationAnnotation],
) -> Result<Vec<u8>, Error> {
    let classes = {
        let animation = encode_animation(skeleton, animation, fps, annotations)?;
        build_class_map(animation)
    };

    serde_hkx::to_bytes(&classes, &HkxHeader::new_skyrim_se()).map_err(|e| {
        Error::SerdeHkx(serde_hkx_features::error::Error::SerError {
            input: "animation.hkx".into(),
            source: Box::new(serde_hkx_features::serde::ser::SerError::Hkx {
                source: e,
                location: snafu::location!(),
            }),
        })
    })
}

/// Builds the HKX class map containing the root-level container and the
/// spline-compressed animation.
///
/// The root-level container owns the animation through its named variant.
/// The animation itself is stored as a top-level class so that the serializer
/// can resolve the pointer relationship.
fn build_class_map<'ser>(animation: hkaSplineCompressedAnimation<'ser>) -> ClassMap<'ser> {
    const ROOT_ID: usize = 0;
    const ANIMATION_ID: usize = 1;

    let root = hkRootLevelContainer {
        __ptr: Some(Pointer::new(ROOT_ID)),
        m_namedVariants: vec![hkRootLevelContainerNamedVariant {
            __ptr: None,
            m_name: "Animation".into(),
            m_className: "hkaSplineCompressedAnimation".into(),
            m_variant: Pointer::new(ANIMATION_ID),
        }],
    };

    let mut classes = ClassMap::new();
    classes.insert(ROOT_ID, Classes::hkRootLevelContainer(root));
    classes.insert(
        ANIMATION_ID,
        Classes::hkaSplineCompressedAnimation(animation),
    );

    #[allow(clippy::expect_used)]
    classes.sort_for_bytes().expect("need hkRootLevelContainer");

    classes
}

fn encode_animation<'ser>(
    skeleton: &'ser Skeleton,
    animation: &'ser Animation,
    fps: f32,
    annotations: &'ser [AnimationAnnotation],
) -> Result<hkaSplineCompressedAnimation<'ser>, Error> {
    validate_fps(fps)?;
    validate_animation(skeleton, animation)?;

    let encoded = SplineDecompressor {
        blocks: ser_builder::from_transform_tracks_decomposer(skeleton, animation)?,
    }
    .encode(0)?;

    let frame_duration = 1.0 / fps;
    let block_duration = animation.duration;

    let block_inverse_duration = if block_duration > 0.0 {
        1.0 / block_duration
    } else {
        0.0
    };

    let max_frames_per_block = animation.num_frames as i32;
    let mask_and_quantization_size = encoded.data.first().map_or(0, |_| 0);

    Ok(hkaSplineCompressedAnimation {
        parent: havok_classes::hkaAnimation {
            m_duration: animation.duration,
            m_numberOfTransformTracks: animation.num_tracks as i32,
            m_numberOfFloatTracks: 0,
            m_annotationTracks: encode_annotations(annotations),
            ..Default::default()
        },
        m_numFrames: animation.num_frames as i32,
        m_numBlocks: encoded.block_offsets.len() as i32,
        m_maxFramesPerBlock: max_frames_per_block,
        m_maskAndQuantizationSize: mask_and_quantization_size,
        m_blockDuration: block_duration,
        m_blockInverseDuration: block_inverse_duration,
        m_frameDuration: frame_duration,
        m_blockOffsets: encoded.block_offsets,
        m_floatBlockOffsets: Vec::new(),
        m_transformOffsets: Vec::new(),
        m_floatOffsets: Vec::new(),
        m_data: encoded.data,
        m_endian: 0,
        ..Default::default()
    })
}

fn validate_fps(fps: f32) -> Result<(), Error> {
    if !fps.is_finite() || fps <= 0.0 {
        return Err(Error::EncoderInvalidFps { fps });
    }

    Ok(())
}

fn validate_animation(skeleton: &Skeleton, animation: &Animation) -> Result<(), Error> {
    let frame_count = animation.num_frames as usize;
    let track_count = animation.num_tracks as usize;
    let bone_count = skeleton.bones.len();

    if frame_count != animation.frames.len() {
        return Err(Error::EncoderFrameCountMismatch {
            expected: frame_count,
            actual: animation.frames.len(),
        });
    }

    if track_count > bone_count {
        return Err(Error::InvalidTrackCount {
            expected: bone_count,
            actual: track_count,
        });
    }

    for (frame_index, frame) in animation.frames.iter().enumerate() {
        if frame.transforms.len() != bone_count {
            return Err(Error::EncoderTransformCountMismatch {
                frame_index,
                expected: bone_count,
                actual: frame.transforms.len(),
            });
        }
    }

    Ok(())
}

/// Raw, per-frame samples for a single transform track, transposed from the
/// animation's frame-major storage into track-major arrays so each
/// component's spline can be built independently.
///
/// This is the direct counterpart of `TransformTrack` in the decoder: instead
/// of already-encoded spline data, it holds the plain floats that will be
/// turned into control points.
pub(crate) struct RawTransformTrack {
    /// [X, Y, Z], one value per frame.
    pub position: [Vec<f32>; 3],
    /// One quaternion per frame, as `[x, y, z, w]`.
    pub rotation: Vec<[f32; 4]>,
    /// [X, Y, Z], one value per frame.
    pub scale: [Vec<f32>; 3],
}

/// Converts external annotation data into Havok annotation tracks.
///
/// KF conversion does not create annotations. The caller therefore supplies
/// them independently and this function groups them by `track_index`.
fn encode_annotations(annotations: &[AnimationAnnotation]) -> Vec<hkaAnnotationTrack<'_>> {
    let max_track = annotations
        .iter()
        .map(|annotation| annotation.track_index as usize)
        .max();

    let Some(max_track) = max_track else {
        return Vec::new();
    };

    let mut tracks = (0..=max_track)
        .map(|_| hkaAnnotationTrack {
            m_annotations: Vec::new(),
            ..Default::default()
        })
        .collect::<Vec<_>>();

    for annotation in annotations {
        let track = &mut tracks[annotation.track_index as usize];

        track.m_annotations.push(hkaAnnotationTrackAnnotation {
            m_time: annotation.time,
            m_text: StringPtr::new(if annotation.text == NULL_STR {
                None
            } else {
                Some(Cow::Borrowed(annotation.text.as_str()))
            }),
            ..Default::default()
        });
    }

    tracks
}
