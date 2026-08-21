// SPDX-License-Identifier: GPL-3.0-or-later
//
// Reference:
// - https://github.com/BadDogSkyrim/PyNifly/blob/7fd4644f5a6416c1502983b7d49a853eb0d24509/docs/hkx_animation_format.md

//! Encodes sampled FBX animation data into a Havok
//! `hkaSplineCompressedAnimation`.

use std::borrow::Cow;

use havok_classes::{
    AnimationType, BlendHint, Classes, hkMemoryResourceContainer, hkRootLevelContainer,
    hkRootLevelContainerNamedVariant, hkaAnimation, hkaAnimationBinding, hkaAnimationContainer,
    hkaAnnotationTrack, hkaAnnotationTrackAnnotation, hkaSplineCompressedAnimation,
};
use havok_types::{NULL_STR, Pointer, StringPtr};
use serde_hkx::HavokSort as _;
use serde_hkx_features::{ClassMap, Format, convert::serialize_class_map};

use super::{Animation, AnimationAnnotation, Skeleton};
use crate::{
    error::Error,
    spline::{SplineData, math::TransformMask},
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
    format: Format,
) -> Result<Vec<u8>, Error> {
    let animation = encode_animation(skeleton, animation, fps)?;
    let root_bone_name = skeleton
        .bones
        .first()
        .map_or("NPC Root [Root]", |bone| bone.name.as_str()); // TODO: valid fallback?
    let classes = build_class_map(animation, root_bone_name);
    Ok(serialize_class_map(classes, format, "")?)
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
fn build_class_map<'ser>(
    mut animation: hkaSplineCompressedAnimation<'ser>,
    root_bone_name: &'ser str,
) -> ClassMap<'ser> {
    const ROOT_ID: usize = 1;
    const ANIMATION_CONTAINER_ID: usize = 2;
    const ANIMATION_ID: usize = 3;
    const BINDING_ID: usize = 4;
    const RESOURCE_CONTAINER_ID: usize = 5;

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
        m_originalSkeletonName: StringPtr::new(Some(root_bone_name.into())),
        m_animation: Pointer::new(ANIMATION_ID),
        m_transformTrackToBoneIndices: Vec::new(),
        m_floatTrackToFloatSlotIndices: Vec::new(),
        m_blendHint: BlendHint::NORMAL,
        ..Default::default()
    };

    let resource_container = hkMemoryResourceContainer {
        __ptr: Some(Pointer::new(RESOURCE_CONTAINER_ID)),
        m_name: StringPtr::new(Some("".into())), // Not null. empty string
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
    classes.insert(ANIMATION_ID, {
        animation.__ptr = Some(Pointer::new(ANIMATION_ID));
        Classes::hkaSplineCompressedAnimation(animation)
    });
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

/// ref
/// - https://github.com/BadDogSkyrim/PyNifly/blob/7fd4644f5a6416c1502983b7d49a853eb0d24509/io_scene_nifly/hkx/anim_skyrim.py#L1016
/// - https://github.com/BadDogSkyrim/PyNifly/blob/7fd4644f5a6416c1502983b7d49a853eb0d24509/docs/hkx_animation_format.md
fn encode_animation<'ser>(
    skeleton: &'ser Skeleton,
    animation: &'ser Animation,
    fps: f32,
) -> Result<hkaSplineCompressedAnimation<'ser>, Error> {
    validate_fps(fps)?;
    validate_animation(skeleton, animation)?;

    const MAX_FRAMES_PER_BLOCK: u32 = 256;
    let encoded = SplineData::from_animation(skeleton, animation)?.encode()?;

    let num_frames = animation.num_frames;
    let frame_duration = 1.0 / fps;
    let block_duration = (MAX_FRAMES_PER_BLOCK - 1) as f32 * frame_duration;
    let block_inverse_duration = if block_duration > 0.0 {
        1.0 / block_duration
    } else {
        0.0
    };
    let num_blocks = ((num_frames + MAX_FRAMES_PER_BLOCK - 3) / (MAX_FRAMES_PER_BLOCK - 1)).max(1);
    let transform_tracks_len = animation.num_tracks as i32;
    let mask_and_quantization_size = transform_tracks_len * TransformMask::MASK_SIZE;
    let annotations = to_annotation_tracks(&animation.annotations, transform_tracks_len as usize);

    Ok(hkaSplineCompressedAnimation {
        __ptr: None, // Set when call build_class_map
        parent: hkaAnimation {
            m_type: AnimationType::HK_SPLINE_COMPRESSED_ANIMATION,
            m_duration: animation.duration,
            m_numberOfTransformTracks: transform_tracks_len,
            m_numberOfFloatTracks: 0,
            m_extractedMotion: Pointer::null(),
            m_annotationTracks: annotations,
            ..Default::default()
        },
        m_numFrames: animation.num_frames as i32,
        m_numBlocks: num_blocks as i32,
        m_maxFramesPerBlock: MAX_FRAMES_PER_BLOCK as i32,
        m_maskAndQuantizationSize: mask_and_quantization_size,
        m_blockDuration: block_duration,
        m_blockInverseDuration: block_inverse_duration,
        m_frameDuration: frame_duration,
        m_blockOffsets: encoded.block_offsets,
        m_floatBlockOffsets: Vec::new(),
        m_transformOffsets: Vec::new(),
        m_floatOffsets: Vec::new(),
        m_data: encoded.data,
        m_endian: 0, // little endian: 0
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

/// Converts external annotation data into Havok annotation tracks.
///
/// Annotations are grouped by their `track_index`. Empty tracks before the
/// highest referenced track are preserved so that the resulting annotation
/// track indices remain stable.
fn to_annotation_tracks(
    annotations: &[AnimationAnnotation],
    transform_tracks_len: usize,
) -> Vec<hkaAnnotationTrack<'_>> {
    let mut tracks = (0..transform_tracks_len)
        .map(|_| hkaAnnotationTrack {
            m_annotations: Vec::new(),
            m_trackName: StringPtr::new(Some("".into())), // This is intentional and is not the same as `null` -> <hkparam name="trackName"></hkparam>
            ..Default::default()
        })
        .collect::<Vec<_>>();

    for annotation in annotations {
        let track_index = annotation.track_index as usize;

        // transform_tracks_len is the hard limit.
        let Some(track) = tracks.get_mut(track_index) else {
            continue;
        };

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
