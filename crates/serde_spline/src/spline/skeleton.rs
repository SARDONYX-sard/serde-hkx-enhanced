use havok_classes::Classes;
use rayon::prelude::*;
use serde_hkx_features::{ClassMap, Result};

use havok_types::QsTransform;

use crate::spline::bail;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Skeleton {
    pub bone_names: Vec<String>,
    pub parent_indices: Vec<i16>,
    /// Local space reference pose.
    pub reference_pose: Vec<QsTransform>,
}

pub(crate) fn into_skeleton(class_map: ClassMap) -> Result<Skeleton> {
    let Some((_, Classes::hkaSkeleton(skeleton))) = class_map
        .into_iter()
        .find(|(_, class)| matches!(class, Classes::hkaSkeleton(_)))
    else {
        bail!("not found hkaSkeleton");
    };

    let bone_names = skeleton
        .m_bones
        .iter()
        .map(|bone| bone.m_name.to_string())
        .collect::<Vec<_>>();

    let parent_indices = skeleton.m_parentIndices;

    Ok(Skeleton {
        bone_names,
        parent_indices,
        reference_pose: skeleton.m_referencePose,
    })
}

// ---------------------------------------------------------------------

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct AnimationAnnotation {
    pub time: f32,
    pub text: String,
    pub track_index: usize,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct AnimationClip {
    pub duration: f32,
    pub num_frames: usize,
    pub num_tracks: usize,

    /// `[frame][bone]`
    pub frames: Vec<Vec<QsTransform>>,
    pub annotations: Vec<AnimationAnnotation>,
    pub track_count_exceeds_bones: bool,
}

impl AnimationClip {
    pub fn frame_at(&self, time_seconds: f64) -> usize {
        if self.num_frames <= 1 {
            return 0;
        }

        let duration = if self.duration > 0.0 {
            self.duration as f64
        } else {
            1.0
        };

        let mut r = time_seconds % duration;

        if r < 0.0 {
            r += duration;
        }

        let frame = ((r / duration) * self.num_frames as f64) as usize;

        frame.min(self.num_frames - 1)
    }
}

pub(super) fn transpose_tracks(
    tracks: &[Vec<QsTransform>],
    num_frames: usize,
) -> Vec<Vec<QsTransform>> {
    if tracks.is_empty() {
        return Vec::new();
    }

    let mut frames = vec![Vec::with_capacity(tracks.len()); num_frames];

    for track in tracks {
        for f in 0..num_frames {
            frames[f].push(track[f].clone());
        }
    }

    frames
}

pub(super) fn find_track_to_bone(class_map: &ClassMap) -> Option<Vec<usize>> {
    let (_, Classes::hkaAnimationBinding(binding)) = class_map
        .iter()
        .find(|(_, c)| matches!(c, Classes::hkaAnimationBinding(_)))?
    else {
        return None;
    };

    let values = &binding.m_transformTrackToBoneIndices;

    if values.is_empty() {
        return None;
    }

    Some(values.iter().map(|v| *v as usize).collect())
}

pub(super) fn apply_skeleton(
    frame_tracks: Vec<Vec<QsTransform>>,
    skeleton: &Skeleton,
    track_to_bone: Option<&[usize]>,
    num_tracks: usize,
) -> Vec<Vec<QsTransform>> {
    let bone_count = skeleton.reference_pose.len();

    frame_tracks
        .into_par_iter()
        .map(|decoded| {
            let mut local = skeleton.reference_pose.clone();

            let tracks_len = num_tracks.min(decoded.len());

            #[expect(clippy::needless_range_loop)]
            for track_index in 0..tracks_len {
                let bone = track_to_bone
                    .and_then(|m| m.get(track_index))
                    .copied()
                    .unwrap_or(track_index);

                if bone < bone_count {
                    local[bone] = decoded[track_index].clone();
                }
            }

            local
        })
        .collect()
}
