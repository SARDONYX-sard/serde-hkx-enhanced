//! Encodes sampled FBX animation data into a Havok
//! `hkaSplineCompressedAnimation`.

use std::borrow::Cow;

use havok_classes::{
    BlendHint, Classes, hkMemoryResourceContainer, hkRootLevelContainer,
    hkRootLevelContainerNamedVariant, hkaAnimationBinding, hkaAnimationContainer,
    hkaAnnotationTrack, hkaAnnotationTrackAnnotation, hkaSplineCompressedAnimation,
};
use havok_types::{NULL_STR, Pointer, StringPtr};
use serde_hkx::{HavokSort as _, bytes::serde::hkx_header::HkxHeader};
use serde_hkx_features::ClassMap;

use super::{Animation, AnimationAnnotation, Skeleton};
use crate::{
    error::Error,
    spline::{SplineDecompressor, math::TransformMask},
};

/// Encodes a sampled FBX animation into a spline-compressed Havok animation.
///
/// The FBX-specific work must already have been completed before this
/// function is called. In particular, `animation` must contain transforms
/// ordered according to the supplied Havok skeleton.
///
/// # Errors
///
/// Returns [`Error`] when the animation dimensions are inconsistent, spline
/// compression fails, the HKX class graph cannot be constructed, or HKX
/// serialization fails.
pub fn to_hkx(
    skeleton: &Skeleton,
    animation: &Animation,
    fps: f32,
    annotations: &[AnimationAnnotation],
) -> Result<Vec<u8>, Error> {
    let animation = encode_animation(skeleton, animation, fps, annotations)?;
    let classes = build_class_map(animation);

    Ok(
        serde_hkx::to_bytes(&classes, &HkxHeader::new_skyrim_se()).map_err(|error| {
            serde_hkx_features::error::Error::SerError {
                input: "".into(),
                source: Box::new(serde_hkx_features::serde::ser::SerError::Hkx {
                    source: error,
                    location: snafu::location!(),
                }),
            }
        })?,
    )
}

/// Builds the HKX class map containing the root-level container and the
/// spline-compressed animation.
///
/// The root-level container owns the animation through its named variant.
/// The animation itself is stored as a top-level class so that the serializer
/// can resolve the pointer relationship.
///
/// ```txt
/// #0001 hkRootLevelContainer
/// ├── namedVariants[0]
/// │   ├── name      = "Merged Animation Container"
/// │   ├── className = "hkaAnimationContainer"
/// │   └── variant   = #0002
/// │
/// └── namedVariants[1]
///     ├── name      = "Resource Data"
///     ├── className = "hkMemoryResourceContainer"
///     └── variant   = #0005
///
/// #0002 hkaAnimationContainer
/// ├── animations = [#0003]
/// └── bindings   = [#0004]
///
/// #0003 hkaSplineCompressedAnimation
///
/// #0004 hkaAnimationBinding
/// ├── animation                    = #0003
/// ├── transformTrackToBoneIndices = []
/// ├── floatTrackToFloatSlotIndices = []
/// └── blendHint                   = NORMAL
///
/// #0005 hkMemoryResourceContainer
/// ├── resourceHandles = []
/// └── children        = []
/// ```
fn build_class_map<'ser>(animation: hkaSplineCompressedAnimation<'ser>) -> ClassMap<'ser> {
    const ROOT_ID: usize = 0;
    const ANIMATION_CONTAINER_ID: usize = 1;
    const ANIMATION_ID: usize = 2;
    const BINDING_ID: usize = 3;
    const RESOURCE_CONTAINER_ID: usize = 4;

    let root = hkRootLevelContainer {
        __ptr: Some(Pointer::new(ROOT_ID)),
        m_namedVariants: vec![
            hkRootLevelContainerNamedVariant {
                __ptr: None,
                m_name: "Merged Animation Container".into(),
                m_className: "hkaAnimationContainer".into(),
                m_variant: Pointer::new(ANIMATION_CONTAINER_ID),
            },
            hkRootLevelContainerNamedVariant {
                __ptr: None,
                m_name: "Resource Data".into(),
                m_className: "hkMemoryResourceContainer".into(),
                m_variant: Pointer::new(RESOURCE_CONTAINER_ID),
            },
        ],
    };

    let animation_container = hkaAnimationContainer {
        __ptr: Some(Pointer::new(ANIMATION_CONTAINER_ID)),
        m_animations: vec![Pointer::new(ANIMATION_ID)],
        m_bindings: vec![Pointer::new(BINDING_ID)],
        ..Default::default()
    };

    let binding = hkaAnimationBinding {
        __ptr: Some(Pointer::new(BINDING_ID)),
        m_animation: Pointer::new(ANIMATION_ID),
        m_transformTrackToBoneIndices: Vec::new(),
        m_floatTrackToFloatSlotIndices: Vec::new(),
        m_blendHint: BlendHint::NORMAL,
        ..Default::default()
    };

    let resource_container = hkMemoryResourceContainer {
        __ptr: Some(Pointer::new(RESOURCE_CONTAINER_ID)),
        m_name: StringPtr::new(None),
        m_resourceHandles: Vec::new(),
        m_children: Vec::new(),
        ..Default::default()
    };

    let mut classes = ClassMap::new();

    classes.insert(ROOT_ID, Classes::hkRootLevelContainer(root));
    classes.insert(
        ANIMATION_CONTAINER_ID,
        Classes::hkaAnimationContainer(animation_container),
    );
    classes.insert(
        ANIMATION_ID,
        Classes::hkaSplineCompressedAnimation(animation),
    );
    classes.insert(BINDING_ID, Classes::hkaAnimationBinding(binding));
    classes.insert(
        RESOURCE_CONTAINER_ID,
        Classes::hkMemoryResourceContainer(resource_container),
    );

    // Since we know that no circular references will occur, there is no need to call `check_sort_for_bytes()`.
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

    let encoded = SplineDecompressor::from_animation(skeleton, animation)?.encode(0)?;

    let block_duration = 8.5; // TODO: valid?

    let block_inverse_duration = if block_duration > 0.0 {
        1.0 / block_duration
    } else {
        0.0
    };

    let transform_tracks_len = animation.num_tracks as i32;
    let mask_and_quantization_size = transform_tracks_len * TransformMask::MASK_SIZE;

    Ok(hkaSplineCompressedAnimation {
        parent: havok_classes::hkaAnimation {
            m_duration: animation.duration,
            m_numberOfTransformTracks: transform_tracks_len,
            m_numberOfFloatTracks: 0,
            m_annotationTracks: to_annotation_tracks(annotations),
            ..Default::default()
        },
        m_numFrames: animation.num_frames as i32,
        m_numBlocks: encoded.block_offsets.len() as i32,
        m_maxFramesPerBlock: 256, // TODO: valid?
        m_maskAndQuantizationSize: mask_and_quantization_size,
        m_blockDuration: block_duration,
        m_blockInverseDuration: block_inverse_duration,
        m_frameDuration: 1.0 / fps,
        m_blockOffsets: encoded.block_offsets,
        m_floatBlockOffsets: Vec::new(),
        m_transformOffsets: Vec::new(),
        m_floatOffsets: Vec::new(),
        m_data: encoded.data,
        m_endian: 0, // little endian: 0
        ..Default::default()
    })
}

fn validate_fps(fps: f32) -> Result<(), Error> {
    if !fps.is_finite() || fps <= 0.0 {
        return Err(Error::InvalidFps { fps });
    }

    Ok(())
}

fn validate_animation(skeleton: &Skeleton, animation: &Animation) -> Result<(), Error> {
    let frame_count = animation.num_frames as usize;
    let track_count = animation.num_tracks as usize;
    let bone_count = skeleton.bones.len();

    if frame_count != animation.frames.len() {
        return Err(Error::FrameCountMismatch {
            expected: frame_count,
            actual: animation.frames.len(),
        });
    }

    if track_count > bone_count {
        return Err(Error::TrackCountExceedsBoneCount {
            bone_count,
            track_count,
        });
    }

    for (frame_index, frame) in animation.frames.iter().enumerate() {
        if frame.transforms.len() != bone_count {
            return Err(Error::TransformCountMismatch {
                frame_index,
                expected: bone_count,
                actual: frame.transforms.len(),
            });
        }
    }

    Ok(())
}

/// Raw, per-frame samples for a single transform track.
///
/// The animation is stored frame-major, while spline construction operates
/// on one transform track at a time. This structure therefore contains the
/// transposed position, rotation, and scale samples for one track.
pub(crate) struct RawTransformTrack {
    /// X, Y, and Z position samples.
    pub position: [Vec<f32>; 3],

    /// Quaternion samples in `[x, y, z, w]` order.
    pub rotation: Vec<[f32; 4]>,

    /// X, Y, and Z scale samples.
    pub scale: [Vec<f32>; 3],
}

/// Converts external annotation data into Havok annotation tracks.
///
/// Annotations are grouped by their `track_index`. Empty tracks before the
/// highest referenced track are preserved so that the resulting annotation
/// track indices remain stable.
fn to_annotation_tracks(annotations: &[AnimationAnnotation]) -> Vec<hkaAnnotationTrack<'_>> {
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
