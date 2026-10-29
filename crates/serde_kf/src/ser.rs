// SPDX-FileCopyrightText: Copyright (c) 2011 figment
// SPDX-License-Identifier: BSD-3-Clause
//
// https://github.com/jgernandt/hkxcmd/tree/51260f796d1d19255936b2e35f33fa752391083a/Addins/ExportKF.cpp

//! Havok animation -> KF.

use std::borrow::Cow;

use havok_types::QsTransform;
use serde_nif::kf::{
    ControlledBlock, CycleType, Key, KeyGroup, KeyType, Quaternion, Rotations, Sequence, TextKey,
    TransformData, TransformInterpolator,
};
use serde_spline::hkx::{Animation, Skeleton};

use crate::Error;

/// Converts a decoded Havok animation into KF file bytes.
///
/// Every skeleton bone gets one key per frame, as hkxcmd does; frames are
/// spaced evenly over the animation duration.
///
/// # Errors
/// If the animation has no frames or its frames do not match the skeleton.
pub fn to_kf(animation: &Animation, skeleton: &Skeleton) -> Result<Vec<u8>, Error> {
    validate(animation, skeleton)?;

    let frame_count = animation.frames.len();
    let frame_time = if frame_count > 1 {
        animation.duration / (frame_count - 1) as f32
    } else {
        0.0
    };

    let controlled_blocks = skeleton
        .bones
        .iter()
        .enumerate()
        .map(|(bone_index, bone)| {
            let track = animation
                .frames
                .iter()
                .enumerate()
                .map(|(frame, transforms)| {
                    (
                        frame as f32 * frame_time,
                        &transforms.transforms[bone_index],
                    )
                });
            let (translation, rotation, scale) = trs(&bone.reference_pose);
            ControlledBlock {
                node_name: Cow::Borrowed(bone.name.as_str()),
                priority: 0,
                interpolator: Some(TransformInterpolator {
                    translation,
                    rotation,
                    scale,
                    data: Some(transform_data(track)),
                }),
            }
        })
        .collect();

    let sequence = Sequence {
        name: Cow::Borrowed("animation"),
        cycle_type: CycleType::Clamp,
        stop_time: animation.duration,
        controlled_blocks,
        text_keys: animation
            .annotations
            .iter()
            .map(|annotation| TextKey {
                time: annotation.time,
                text: Cow::Borrowed(annotation.text.as_str()),
            })
            .collect(),
        ..Sequence::default()
    };
    Ok(sequence.to_bytes())
}

fn validate(animation: &Animation, skeleton: &Skeleton) -> Result<(), Error> {
    if skeleton.bones.is_empty() {
        return Err(Error::invalid("cannot convert KF with an empty skeleton"));
    }
    if animation.frames.is_empty() {
        return Err(Error::invalid("animation contains no frames"));
    }
    if animation.num_frames as usize != animation.frames.len() {
        return Err(Error::invalid(
            "animation num_frames does not match the frame count",
        ));
    }
    if animation.num_tracks as usize > skeleton.bones.len() {
        return Err(Error::invalid(
            "animation track count exceeds skeleton bone count",
        ));
    }
    if animation
        .frames
        .iter()
        .any(|frame| frame.transforms.len() != skeleton.bones.len())
    {
        return Err(Error::invalid(
            "animation frame transform count does not match skeleton bone count",
        ));
    }
    Ok(())
}

/// One bone's keys. Key types follow hkxcmd: linear translation/scale and
/// quadratic rotations (which, for quaternions, store no tangents).
fn transform_data<'t>(track: impl Iterator<Item = (f32, &'t QsTransform)>) -> TransformData {
    let mut rotations = Vec::new();
    let mut translations = Vec::new();
    let mut scales = Vec::new();
    for (time, transform) in track {
        let (translation, rotation, scale) = trs(transform);
        translations.push(Key::new(time, translation));
        rotations.push(Key::new(time, rotation));
        scales.push(Key::new(time, scale));
    }
    TransformData {
        rotations: Rotations::Quaternion(KeyGroup {
            key_type: KeyType::Quadratic,
            keys: rotations,
        }),
        translations: KeyGroup {
            key_type: KeyType::Linear,
            keys: translations,
        },
        scales: KeyGroup {
            key_type: KeyType::Linear,
            keys: scales,
        },
    }
}

/// KF scale is uniform, so only Havok's `scale.x` survives.
const fn trs(transform: &QsTransform) -> ([f32; 3], Quaternion, f32) {
    let t = &transform.transition;
    let q = &transform.quaternion;
    (
        [t.x, t.y, t.z],
        Quaternion {
            x: q.x,
            y: q.y,
            z: q.z,
            w: q.scaler,
        },
        transform.scale.x,
    )
}
