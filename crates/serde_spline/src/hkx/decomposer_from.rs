use crate::spline::SplineDecompressor;
use crate::spline::math::{
    QuatA16, SplineDynamicTrackQuat, SplineDynamicTrackVector, SplineStaticTrack, SplineTrackQuat,
    SplineTrackType, SplineTrackVector, TransformMask, TransformSplineBlock, TransformTrack,
    TransformType,
};
use havok_types::Vector4;

use super::ser::RawTransformTrack;
use super::{Animation, Skeleton};
use crate::error::Error;

impl SplineDecompressor {
    /// Builds fully-evaluable spline blocks directly from an `Animation` and
    /// its `Skeleton`.
    ///
    /// # Errors
    /// - If `animation.frames.len()` doesn't match `animation.num_frames`
    /// - If any frame's transform count doesn't match `skeleton.bones.len()`, or if the animation has zero frames or the skeleton has zero bones.
    pub(crate) fn from_animation(
        skeleton: &Skeleton,
        animation: &Animation,
    ) -> Result<Self, Error> {
        let num_tracks = skeleton.bones.len();
        let num_frames = animation.num_frames as usize;

        if num_frames == 0 || num_tracks == 0 {
            return Err(Error::EmptyAnimation);
        }

        if animation.frames.len() != num_frames {
            return Err(Error::FrameCountMismatch {
                expected: num_frames,
                actual: animation.frames.len(),
            });
        }

        // Transpose frame-major storage (`frames[frame].transforms[track]`)
        // into track-major raw samples, one entry per bone.
        let mut raw_tracks: Vec<RawTransformTrack> = (0..num_tracks)
            .map(|_| RawTransformTrack {
                position: core::array::from_fn(|_| Vec::with_capacity(num_frames)),
                rotation: Vec::with_capacity(num_frames),
                scale: core::array::from_fn(|_| Vec::with_capacity(num_frames)),
            })
            .collect();

        for (frame_index, frame) in animation.frames.iter().enumerate() {
            if frame.transforms.len() != num_tracks {
                return Err(Error::TransformCountMismatch {
                    frame_index,
                    expected: num_tracks,
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

        // Classify every bone's components and build the evaluable spline data
        // (masks + tracks) in one pass.
        let mut masks = Vec::with_capacity(num_tracks);
        let mut tracks = Vec::with_capacity(num_tracks);

        for raw in &raw_tracks {
            let (position, position_kinds) = build_vector_track(&raw.position, 0.0, num_frames);
            let (rotation, rotation_kind) = build_rotation_track(&raw.rotation, num_frames);
            let (scale, scale_kinds) = build_vector_track(&raw.scale, 1.0, num_frames);

            masks.push(build_mask(position_kinds, rotation_kind, scale_kinds));
            tracks.push(TransformTrack {
                position,
                rotation,
                scale,
            });
        }

        Ok(Self {
            blocks: vec![TransformSplineBlock { masks, tracks }],
        })
    }
}

/// Classifies one axis as Identity / Static / Dynamic and builds the
/// corresponding evaluable `SplineTrackVector` for a full [X, Y, Z] triple.
///
/// Mirrors `read_vector_track`'s "any axis dynamic → whole triple stored as
/// `Dynamic`" rule: `SplineDynamicTrackVector` can hold mixed-length axes
/// (length 1 for a static/identity axis, `num_frames` for a dynamic one), so
/// there's no need to force every axis dynamic just because one of them is.
fn build_vector_track(
    samples: &[Vec<f32>; 3],
    identity_value: f32,
    num_frames: usize,
) -> (SplineTrackVector, [SplineTrackType; 3]) {
    let kinds: [SplineTrackType; 3] =
        core::array::from_fn(|axis| classify_axis(&samples[axis], identity_value));

    let any_dynamic = kinds.contains(&SplineTrackType::Dynamic);

    if !any_dynamic {
        let value = Vector4 {
            x: axis_static_value(&samples[0], kinds[0], identity_value),
            y: axis_static_value(&samples[1], kinds[1], identity_value),
            z: axis_static_value(&samples[2], kinds[2], identity_value),
            w: 0.0,
        };

        return (
            SplineTrackVector::Static(SplineStaticTrack { value }),
            kinds,
        );
    }

    let degree: u8 = 1;
    let num_items = num_frames - 1;
    let knots = clamped_uniform_knots(num_items, degree as usize);

    let tracks: [Vec<f32>; 3] = core::array::from_fn(|axis| match kinds[axis] {
        SplineTrackType::Dynamic => samples[axis].clone(),
        SplineTrackType::Static => vec![samples[axis][0]],
        SplineTrackType::Identity => vec![identity_value],
    });

    (
        SplineTrackVector::Dynamic(SplineDynamicTrackVector {
            tracks,
            knots,
            degree,
        }),
        kinds,
    )
}

fn axis_static_value(values: &[f32], kind: SplineTrackType, identity_value: f32) -> f32 {
    match kind {
        SplineTrackType::Static => values[0],
        SplineTrackType::Identity => identity_value,
        SplineTrackType::Dynamic => {
            unreachable!("caller only reaches here when no axis is Dynamic")
        }
    }
}

/// A component is `Identity` when every sample equals the type's neutral
/// value (0.0 for position, 1.0 for scale), `Static` when every sample is
/// equal to some other constant, and `Dynamic` otherwise.
fn classify_axis(values: &[f32], identity_value: f32) -> SplineTrackType {
    #[expect(
        clippy::float_cmp,
        reason = "Track classification requires exact f32 equality."
    )]
    if values.iter().all(|&value| value == identity_value) {
        // Identity is a semantic representation. A tolerance here would
        // discard small but non-zero values during encoding.
        SplineTrackType::Identity
    } else if values.windows(2).all(|window| window[0] == window[1]) {
        // Static means that every sample has the same f32 value. Do not use
        // `f32::EPSILON` as an implicit tolerance for representation choice.
        SplineTrackType::Static
    } else {
        SplineTrackType::Dynamic
    }
}

fn build_rotation_track(
    samples: &[[f32; 4]],
    num_frames: usize,
) -> (SplineTrackQuat, SplineTrackType) {
    #[expect(clippy::float_cmp)]
    if samples.windows(2).all(|w| w[0] == w[1]) {
        let [x, y, z, w] = samples[0];

        return (
            SplineTrackQuat::Static(SplineStaticTrack {
                value: QuatA16::new(x, y, z, w),
            }),
            SplineTrackType::Static,
        );
    }

    let degree: u8 = 1;
    let num_items = num_frames - 1;
    let knots = clamped_uniform_knots(num_items, degree as usize);

    let track = samples
        .iter()
        .map(|&[x, y, z, w]| QuatA16::new(x, y, z, w))
        .collect();

    (
        SplineTrackQuat::Dynamic(SplineDynamicTrackQuat {
            track,
            knots,
            degree,
        }),
        SplineTrackType::Dynamic,
    )
}

/// Combines each component's classification into one `TransformMask`.
///
/// Quantization-type bits are left at their default (0); they're only
/// meaningful once a block is serialized to bytes, which this function
/// doesn't do.
fn build_mask(
    position_kinds: [SplineTrackType; 3],
    rotation_kind: SplineTrackType,
    scale_kinds: [SplineTrackType; 3],
) -> TransformMask {
    let mut mask = TransformMask {
        quantization_types: 0,
        position_types: 0,
        rotation_types: 0,
        scale_types: 0,
    };

    mask.set_sub_track_type(TransformType::PosX, position_kinds[0]);
    mask.set_sub_track_type(TransformType::PosY, position_kinds[1]);
    mask.set_sub_track_type(TransformType::PosZ, position_kinds[2]);

    mask.set_sub_track_type(TransformType::Rotation, rotation_kind);

    mask.set_sub_track_type(TransformType::ScaleX, scale_kinds[0]);
    mask.set_sub_track_type(TransformType::ScaleY, scale_kinds[1]);
    mask.set_sub_track_type(TransformType::ScaleZ, scale_kinds[2]);

    mask
}

/// A clamped-uniform knot vector `[0,0,...,0, 1, 2, ..., n-1, n-1,...,n-1]`
/// for a degree-`degree` B-spline with `num_items + 1` control points.
///
/// With `degree == 1`, evaluating at integer frame `t` returns exactly
/// `control_points[t]` — this is why raw per-frame samples can be used
/// directly as control points without any curve fitting.
fn clamped_uniform_knots(num_items: usize, degree: usize) -> Vec<f32> {
    let mut knots = vec![0.0f32; degree + 1];
    knots.extend((1..num_items).map(|i| i as f32));
    knots.extend(vec![num_items as f32; degree + 1]);
    knots
}
