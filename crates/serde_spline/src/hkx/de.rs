use std::path::Path;

use havok_classes::{Classes, hkaSplineCompressedAnimation};
use rayon::prelude::*;
use serde_hkx_features::ClassMap;
use serde_hkx_features::convert::process_serde_with;

use super::{Animation, AnimationAnnotation, AnimationFrame, Bone, Skeleton};
use crate::error::Error;
use crate::spline::SplineDecompressor;

impl Animation {
    /// Extract animation information from `hkaSplineCompressedAnimation` using `bytes`
    /// (such as .hkx or converted .xml file) and Skeleton information.
    ///
    /// - `path` is intended for error handling only and has no other purpose.
    ///
    /// # Errors
    /// - Unexpected data
    /// - Missing `hkaSplineCompressedAnimation`
    #[inline]
    pub fn from_bytes(skeleton: &Skeleton, bytes: &[u8], path: &Path) -> Result<Self, Error> {
        process_serde_with(
            bytes,
            path,
            |class_map| de_animation(&class_map, skeleton),
            |class_map| de_animation(&class_map, skeleton),
        )
    }
}

impl Skeleton {
    /// Extract animation information from `hkaSkeleton` using `bytes`
    /// (such as .hkx or converted .xml file)
    ///
    /// - `path` is intended for error handling only and has no other purpose.
    ///
    /// # Errors
    /// Missing `hkaSkeleton`
    #[inline]
    pub fn from_bytes(skeleton_bytes: &[u8], skeleton_path: &Path) -> Result<Self, Error> {
        process_serde_with(skeleton_bytes, skeleton_path, de_skeleton, de_skeleton)
    }
}

fn de_skeleton(class_map: ClassMap<'_>) -> Result<Skeleton, Error> {
    let Some((_, Classes::hkaSkeleton(skeleton))) = class_map
        .into_iter()
        .find(|(_, class)| matches!(class, Classes::hkaSkeleton(_)))
    else {
        return Err(Error::MissingSplineCompressedAnimation);
    };

    let bone_count = skeleton.m_bones.len();

    if skeleton.m_parentIndices.len() != bone_count {
        return Err(Error::SkeletonBoneParentIndexCountMismatch {
            bone_count,
            parent_index_count: skeleton.m_parentIndices.len(),
        });
    }

    if skeleton.m_referencePose.len() != bone_count {
        return Err(Error::SkeletonReferencePoseCountMismatch {
            bone_count,
            reference_pose_count: skeleton.m_referencePose.len(),
        });
    }

    let bones = skeleton
        .m_bones
        .iter()
        .enumerate()
        .map(|(index, bone)| {
            let parent_index = skeleton.m_parentIndices[index];

            let reference_pose = skeleton
                .m_referencePose
                .get(index)
                .ok_or_else(|| Error::MissingReferencePose { bone_index: index })?;

            Ok(Bone {
                name: bone.m_name.to_string(),
                parent_index,
                reference_pose: reference_pose.clone(),
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;

    Ok(Skeleton { bones })
}

/// Converts one spline-compressed animation class into the FFI animation
/// representation.
///
/// The spline bytes stored in `m_data` are passed directly to
/// [`SplineDecompressor`]. No alternate spline parser is used here.
fn de_animation(class_map: &ClassMap<'_>, skeleton: &Skeleton) -> Result<Animation, Error> {
    let Some((_, Classes::hkaSplineCompressedAnimation(spline))) = class_map
        .iter()
        .find(|(_, class)| matches!(class, Classes::hkaSplineCompressedAnimation(_)))
    else {
        return Err(Error::MissingSplineCompressedAnimation);
    };

    let num_frames = spline.m_numFrames as usize;
    let num_tracks = spline.parent.m_numberOfTransformTracks as usize;
    let num_float_tracks = spline.parent.m_numberOfFloatTracks as usize;

    let num_blocks = spline.m_numBlocks;

    if num_blocks == 0 {
        return Err(Error::EmptySplineData);
    }

    let data = spline.m_data.as_slice();

    if data.is_empty() {
        return Err(Error::EmptySplineData);
    }

    let block_offsets = spline.m_blockOffsets.as_slice();
    if block_offsets.len() != num_blocks as usize {
        return Err(Error::SplineBlockOffsetCountMismatch {
            block_count: block_offsets.len(),
            block_offset_count: num_blocks,
        });
    }

    let decompressor =
        SplineDecompressor::decode(data, block_offsets, num_tracks, num_float_tracks)?;

    let track_to_bone = find_track_to_bone(class_map);

    validate_track_mapping(&track_to_bone, num_tracks, skeleton.bones.len())?;

    let frames = decode_frames(
        &decompressor,
        num_frames,
        num_tracks,
        &track_to_bone,
        skeleton,
    )?;

    let annotations = decode_annotations(spline);

    Ok(Animation {
        duration: spline.parent.m_duration,
        num_frames: num_frames as u32,
        num_tracks: num_tracks as u32,
        frames,
        annotations,
    })
}

/// Evaluates every transform track for every frame.
///
/// The skeleton reference pose is used as the base frame. Decoded transform
/// tracks overwrite the corresponding bone transforms.
fn decode_frames(
    decompressor: &SplineDecompressor,
    num_frames: usize,
    num_tracks: usize,
    track_to_bone: &[usize],
    skeleton: &Skeleton,
) -> Result<Vec<AnimationFrame>, Error> {
    let mut frames = Vec::with_capacity(num_frames);

    for frame_index in 0..num_frames {
        let time = if num_frames <= 1 {
            0.0
        } else {
            frame_index as f32
        };

        let mut transforms = skeleton
            .bones
            .iter()
            .map(|bone| bone.reference_pose.clone())
            .collect::<Vec<_>>();

        for track_index in 0..num_tracks {
            let bone_index = track_to_bone
                .get(track_index)
                .copied()
                .unwrap_or(track_index);

            if bone_index >= transforms.len() {
                return Err(Error::InvalidBoneIndex {
                    track_index,
                    bone_index,
                    bone_count: transforms.len(),
                });
            }

            let transform = decompressor.get_value(0, track_index, time)?;
            transforms[bone_index] = transform;
        }

        frames.push(AnimationFrame { transforms });
    }

    Ok(frames)
}

/// Finds the transform-track-to-bone mapping from the animation binding.
///
/// If there is no binding, tracks are mapped directly to bones by index.
fn find_track_to_bone(class_map: &ClassMap<'_>) -> Vec<usize> {
    let Some((_, Classes::hkaAnimationBinding(binding))) = class_map
        .iter()
        .find(|(_, class)| matches!(class, Classes::hkaAnimationBinding(_)))
    else {
        return Vec::new();
    };

    binding
        .m_transformTrackToBoneIndices
        .iter()
        .map(|index| *index as usize)
        .collect()
}

/// Validates the optional animation binding.
///
/// An empty mapping means direct track-to-bone mapping and is therefore
/// accepted.
fn validate_track_mapping(
    track_to_bone: &[usize],
    num_tracks: usize,
    bone_count: usize,
) -> Result<(), Error> {
    if track_to_bone.is_empty() {
        if num_tracks > bone_count {
            return Err(Error::InvalidTrackMapping {
                track_count: num_tracks,
                bone_count,
            });
        }

        return Ok(());
    }

    if track_to_bone.len() < num_tracks {
        return Err(Error::InvalidTrackMapping {
            track_count: num_tracks,
            bone_count: track_to_bone.len(),
        });
    }

    for (track_index, &bone_index) in track_to_bone.iter().take(num_tracks).enumerate() {
        if bone_index >= bone_count {
            return Err(Error::InvalidBoneIndex {
                track_index,
                bone_index,
                bone_count,
            });
        }
    }

    Ok(())
}

/// Converts Havok annotation tracks into the FFI annotation representation.
fn decode_annotations(spline: &hkaSplineCompressedAnimation<'_>) -> Vec<AnimationAnnotation> {
    let mut annotations = Vec::new();

    for (track_index, track) in spline.parent.m_annotationTracks.iter().enumerate() {
        for annotation in track.m_annotations.iter() {
            if annotation.m_text.is_null() {
                continue;
            }

            annotations.push(AnimationAnnotation {
                time: annotation.m_time,
                text: annotation.m_text.to_string(),
                track_index: track_index as u32,
            });
        }
    }

    annotations.par_sort_by(|a, b| {
        a.time
            .partial_cmp(&b.time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    annotations
}
