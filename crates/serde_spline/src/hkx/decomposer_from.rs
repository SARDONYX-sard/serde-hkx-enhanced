// SPDX-License-Identifier: GPL-3.0-or-later
//
// Reference:
// - https://github.com/BadDogSkyrim/PyNifly/blob/7fd4644f5a6416c1502983b7d49a853eb0d24509/io_scene_nifly/hkx/skeleton_hkx.py
use crate::error::Error;
use crate::spline::SplineData;
use crate::spline::math::{
    QuantizationType, QuatA16, SplineDynamicTrackQuat, SplineDynamicTrackVector, SplineStaticTrack,
    SplineTrackQuat, SplineTrackType, SplineTrackVector, TransformMask, TransformSplineBlock,
    TransformTrack, TransformType,
};

use super::{Animation, Skeleton};

const MAX_FRAMES_PER_BLOCK: usize = 256;
const SPLINE_DEGREE: u8 = 1;

impl SplineData {
    /// Builds spline-compressed animation blocks from sampled animation frames.
    ///
    /// The generated representation mirrors Soulstruct's
    /// `SplineCompressedAnimationData.pack()` input model:
    ///
    /// - identity components are omitted;
    /// - constant components are stored as static values;
    /// - varying components are stored as degree-1 spline control points;
    /// - each block contains at most 256 frames.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptyAnimation`] if the animation has no frames or the
    /// skeleton has no bones.
    ///
    /// Returns [`Error::FrameCountMismatch`] if `animation.num_frames` does not
    /// match the number of decoded frames.
    ///
    /// Returns [`Error::TransformCountMismatch`] if a frame does not contain
    /// one transform for every skeleton bone.
    pub(crate) fn from_animation(
        skeleton: &Skeleton,
        animation: &Animation,
    ) -> Result<Self, Error> {
        let bone_count = skeleton.bones.len();
        let frame_count = animation.num_frames as usize;

        if frame_count == 0 || bone_count == 0 {
            return Err(Error::EmptyAnimation);
        }

        if animation.frames.len() != frame_count {
            return Err(Error::FrameCountMismatch {
                expected: frame_count,
                actual: animation.frames.len(),
            });
        }

        let mut blocks = Vec::with_capacity(frame_count.div_ceil(MAX_FRAMES_PER_BLOCK));

        for block_start in (0..frame_count).step_by(MAX_FRAMES_PER_BLOCK) {
            let block_end = (block_start + MAX_FRAMES_PER_BLOCK).min(frame_count);
            let block_frame_count = block_end - block_start;

            let mut raw_tracks =
                vec![RawTransformTrack::with_capacity(block_frame_count); bone_count];

            for (frame_index, frame) in animation.frames[block_start..block_end].iter().enumerate()
            {
                let absolute_frame_index = block_start + frame_index;

                if frame.transforms.len() != bone_count {
                    return Err(Error::TransformCountMismatch {
                        frame_index: absolute_frame_index,
                        expected: bone_count,
                        actual: frame.transforms.len(),
                    });
                }

                for (track, transform) in raw_tracks.iter_mut().zip(&frame.transforms) {
                    track.position[0].push(transform.transition.x);
                    track.position[1].push(transform.transition.y);
                    track.position[2].push(transform.transition.z);

                    track.rotation.push([
                        transform.quaternion.x,
                        transform.quaternion.y,
                        transform.quaternion.z,
                        transform.quaternion.scaler,
                    ]);

                    track.scale[0].push(transform.scale.x);
                    track.scale[1].push(transform.scale.y);
                    track.scale[2].push(transform.scale.z);
                }
            }

            let mut masks = Vec::with_capacity(bone_count);
            let mut tracks = Vec::with_capacity(bone_count);

            for raw in &raw_tracks {
                let (position, position_types) =
                    build_vector_track(&raw.position, 0.0, block_frame_count);

                let (rotation, rotation_type) = build_rotation_track(&raw.rotation)?;
                let (scale, scale_types) = build_vector_track(&raw.scale, 1.0, block_frame_count);

                let mask = build_mask(position_types, rotation_type, scale_types);

                masks.push(mask);
                tracks.push(TransformTrack {
                    position,
                    rotation,
                    scale,
                });
            }

            blocks.push(TransformSplineBlock { masks, tracks });
        }

        Ok(Self { blocks })
    }
}

/// Raw, per-frame samples for a single transform track.
///
/// The animation is stored frame-major, while spline construction operates
/// on one transform track at a time. This structure therefore contains the
/// transposed position, rotation, and scale samples for one track.
#[derive(Clone, Debug)]
struct RawTransformTrack {
    /// X, Y, and Z position samples.
    position: [Vec<f32>; 3],

    /// Quaternion samples in `[x, y, z, w]` order.
    rotation: Vec<[f32; 4]>,

    /// X, Y, and Z scale samples.
    scale: [Vec<f32>; 3],
}

impl RawTransformTrack {
    fn with_capacity(frame_count: usize) -> Self {
        Self {
            position: core::array::from_fn(|_| Vec::with_capacity(frame_count)),
            rotation: Vec::with_capacity(frame_count),
            scale: core::array::from_fn(|_| Vec::with_capacity(frame_count)),
        }
    }
}

fn build_vector_track(
    samples: &[Vec<f32>; 3],
    identity_value: f32,
    frame_count: usize,
) -> (SplineTrackVector, [SplineTrackType; 3]) {
    let kinds = core::array::from_fn(|axis| classify_axis(&samples[axis], identity_value));

    if !kinds.contains(&SplineTrackType::Dynamic) {
        return (
            SplineTrackVector::Static(SplineStaticTrack {
                value: havok_types::Vector4 {
                    x: static_value(&samples[0], kinds[0], identity_value),
                    y: static_value(&samples[1], kinds[1], identity_value),
                    z: static_value(&samples[2], kinds[2], identity_value),
                    w: 0.0,
                },
            }),
            kinds,
        );
    }

    let tracks = core::array::from_fn(|axis| match kinds[axis] {
        SplineTrackType::Dynamic => samples[axis].clone(),
        SplineTrackType::Static => vec![samples[axis][0]],
        SplineTrackType::Identity => vec![identity_value],
    });

    (
        SplineTrackVector::Dynamic(SplineDynamicTrackVector {
            tracks,
            knots: clamped_uniform_knots(frame_count, SPLINE_DEGREE as usize),
            degree: SPLINE_DEGREE,
        }),
        kinds,
    )
}

fn static_value(values: &[f32], kind: SplineTrackType, identity_value: f32) -> f32 {
    match kind {
        SplineTrackType::Static => values[0],
        SplineTrackType::Identity => identity_value,
        SplineTrackType::Dynamic => unreachable!(),
    }
}

const CLASSIFICATION_EPSILON: f32 = 1.0e-5;
fn classify_axis(values: &[f32], identity_value: f32) -> SplineTrackType {
    if values
        .iter()
        .all(|&value| (value - identity_value).abs() < CLASSIFICATION_EPSILON)
    {
        SplineTrackType::Identity
    } else {
        let first = values[0];

        if values
            .iter()
            .all(|&value| (value - first).abs() < CLASSIFICATION_EPSILON)
        {
            SplineTrackType::Static
        } else {
            SplineTrackType::Dynamic
        }
    }
}

fn build_rotation_track(samples: &[[f32; 4]]) -> Result<(SplineTrackQuat, SplineTrackType), Error> {
    let is_identity = samples.iter().all(|q| {
        q[0].abs() < CLASSIFICATION_EPSILON
            && q[1].abs() < CLASSIFICATION_EPSILON
            && q[2].abs() < CLASSIFICATION_EPSILON
            && (q[3].abs() - 1.0).abs() < CLASSIFICATION_EPSILON
    });

    if is_identity {
        return Ok((SplineTrackQuat::Identity, SplineTrackType::Identity));
    }

    let first = samples[0];

    let is_static = samples.iter().all(|q| {
        (q[0] - first[0]).abs() < CLASSIFICATION_EPSILON
            && (q[1] - first[1]).abs() < CLASSIFICATION_EPSILON
            && (q[2] - first[2]).abs() < CLASSIFICATION_EPSILON
            && (q[3] - first[3]).abs() < CLASSIFICATION_EPSILON
    });

    if is_static {
        return Ok((
            SplineTrackQuat::Static(SplineStaticTrack {
                value: QuatA16::new(first[0], first[1], first[2], first[3]),
            }),
            SplineTrackType::Static,
        ));
    }

    let track = samples
        .iter()
        .map(|&[x, y, z, w]| QuatA16::new(x, y, z, w))
        .collect();

    Ok((
        SplineTrackQuat::Dynamic(SplineDynamicTrackQuat {
            track,
            knots: Vec::new(), // ser unused
            degree: SPLINE_DEGREE,
        }),
        SplineTrackType::Dynamic,
    ))
}

fn build_mask(
    position_kinds: [SplineTrackType; 3],
    rotation_kind: SplineTrackType,
    scale_kinds: [SplineTrackType; 3],
) -> TransformMask {
    let mut mask = TransformMask::default();

    mask.set_position_quantization_type(QuantizationType::Bit16);
    mask.set_rotation_quantization_type(QuantizationType::Bit40);
    mask.set_scale_quantization_type(QuantizationType::Bit16);

    mask.set_sub_track_type(TransformType::PosX, position_kinds[0]);
    mask.set_sub_track_type(TransformType::PosY, position_kinds[1]);
    mask.set_sub_track_type(TransformType::PosZ, position_kinds[2]);

    mask.set_sub_track_type(TransformType::Rotation, rotation_kind);

    mask.set_sub_track_type(TransformType::ScaleX, scale_kinds[0]);
    mask.set_sub_track_type(TransformType::ScaleY, scale_kinds[1]);
    mask.set_sub_track_type(TransformType::ScaleZ, scale_kinds[2]);

    mask
}

fn clamped_uniform_knots(control_point_count: usize, degree: usize) -> Vec<f32> {
    assert!(control_point_count > 0);
    assert!(degree > 0);
    assert!(control_point_count > degree);

    let knot_count = control_point_count + degree + 1;
    let interior_count = knot_count - 2 * (degree + 1);

    let mut knots = Vec::with_capacity(knot_count);

    knots.extend(core::iter::repeat_n(0.0, degree + 1));

    for value in 1..=interior_count {
        knots.push(value as f32);
    }

    let last = (control_point_count - degree) as f32;
    knots.extend(core::iter::repeat_n(last, degree + 1));

    knots
}
