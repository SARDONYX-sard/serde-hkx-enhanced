//! Havok animation -> KF -> Havok animation on a synthetic skeleton.
//!
//! Built from structs rather than game files, so it runs everywhere and
//! checks only this crate's conversion.

use havok_types::{QsTransform, Quaternion, Vector4};
use serde_kf::{kf_to_animation, to_kf};
use serde_nif::kf::Sequence;
use serde_spline::hkx::{Animation, AnimationAnnotation, AnimationFrame, Bone, Skeleton};

const FPS: f32 = 30.0;

fn transform(translation: [f32; 3], axis: [f32; 3], angle: f32) -> QsTransform {
    let (s, c) = (angle * 0.5).sin_cos();
    QsTransform {
        transition: Vector4 {
            x: translation[0],
            y: translation[1],
            z: translation[2],
            w: 0.0,
        },
        quaternion: Quaternion {
            x: axis[0] * s,
            y: axis[1] * s,
            z: axis[2] * s,
            scaler: c,
        },
        scale: Vector4 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
            w: 1.0,
        },
    }
}

fn skeleton() -> Skeleton {
    let bone = |name: &str, parent_index, reference_pose| Bone {
        name: name.to_owned(),
        parent_index,
        reference_pose,
    };
    Skeleton {
        bones: vec![
            bone(
                "NPC Root [Root]",
                -1,
                transform([0.0; 3], [0.0, 0.0, 1.0], 0.0),
            ),
            bone(
                "Pelvis",
                0,
                transform([0.0, 0.0, 60.0], [0.0, 0.0, 1.0], 0.3),
            ),
            bone(
                "Spine",
                1,
                transform([0.0, 5.0, 8.0], [1.0, 0.0, 0.0], -0.2),
            ),
        ],
    }
}

/// One second of motion; the third bone has a rotation passing through
/// `w < 0` to exercise the quaternion sign handling.
fn animation(skeleton: &Skeleton) -> Animation {
    let num_frames = 31;
    let frames = (0..num_frames)
        .map(|frame| {
            let t = frame as f32 / FPS;
            AnimationFrame {
                transforms: vec![
                    transform([t * 10.0, 0.0, 0.0], [0.0, 0.0, 1.0], t),
                    transform(
                        [0.0, (t * 6.0).sin(), 60.0],
                        [0.0, 1.0, 0.0],
                        (t * 4.0).sin(),
                    ),
                    transform([0.0, 5.0, 8.0], [1.0, 0.0, 0.0], t * 5.0),
                ],
            }
        })
        .collect();
    Animation {
        duration: (num_frames - 1) as f32 / FPS,
        num_frames,
        num_tracks: skeleton.bones.len() as u32,
        frames,
        annotations: vec![AnimationAnnotation {
            track_index: 0,
            time: 0.5,
            text: "HitFrame".to_owned(),
        }],
    }
}

/// Largest component difference of the translations and of the rotations
/// (`q` and `-q` are the same rotation). Components rather than an angle:
/// `acos` near 1 is too coarse in `f32` to measure small errors.
fn difference(a: &QsTransform, b: &QsTransform) -> (f32, f32) {
    let max_diff = |a: &[f32], b: &[f32]| {
        a.iter()
            .zip(b)
            .fold(0.0_f32, |max, (a, b)| max.max((a - b).abs()))
    };
    let (ta, tb) = (&a.transition, &b.transition);
    let translation = max_diff(&[ta.x, ta.y, ta.z], &[tb.x, tb.y, tb.z]);

    let qa = [
        a.quaternion.x,
        a.quaternion.y,
        a.quaternion.z,
        a.quaternion.scaler,
    ];
    let qb = [
        b.quaternion.x,
        b.quaternion.y,
        b.quaternion.z,
        b.quaternion.scaler,
    ];
    let dot: f32 = qa.iter().zip(&qb).map(|(a, b)| a * b).sum();
    let rotation = max_diff(&qa, &qb.map(|v| v * dot.signum()));
    (translation, rotation)
}

#[test]
fn animation_survives_kf() {
    let skeleton = skeleton();
    let expected = animation(&skeleton);

    let bytes = to_kf(&expected, &skeleton).unwrap();
    let actual = kf_to_animation(&bytes, &skeleton, FPS).unwrap();

    assert_eq!(actual.num_frames, expected.num_frames);
    assert!((actual.duration - expected.duration).abs() < 1.0e-6);
    for (frame, (a, e)) in actual.frames.iter().zip(&expected.frames).enumerate() {
        for (bone, (a, e)) in a.transforms.iter().zip(&e.transforms).enumerate() {
            let (translation, rotation) = difference(a, e);
            assert!(
                translation < 1.0e-4 && rotation < 1.0e-5,
                "frame {frame} bone {bone}: translation {translation}, rotation {rotation}"
            );
        }
    }

    let [annotation] = actual.annotations.as_slice() else {
        panic!("expected one annotation, got {:?}", actual.annotations);
    };
    assert_eq!(
        (annotation.time, annotation.text.as_str()),
        (0.5, "HitFrame")
    );
}

#[test]
fn kf_rewrites_byte_for_byte() {
    let skeleton = skeleton();
    let bytes = to_kf(&animation(&skeleton), &skeleton).unwrap();
    let sequence = Sequence::from_bytes(&bytes).unwrap();
    assert_eq!(sequence.controlled_blocks.len(), skeleton.bones.len());
    assert_eq!(sequence.to_bytes(), bytes);
}

#[test]
fn missing_bones_keep_the_reference_pose() {
    let skeleton = skeleton();
    let bytes = to_kf(&animation(&skeleton), &skeleton).unwrap();

    let mut extended = skeleton;
    let extra = transform([1.0, 2.0, 3.0], [0.0, 0.0, 1.0], 1.0);
    extended.bones.push(Bone {
        name: "Tail".to_owned(),
        parent_index: 2,
        reference_pose: extra.clone(),
    });

    let actual = kf_to_animation(&bytes, &extended, FPS).unwrap();
    assert!(
        actual
            .frames
            .iter()
            .all(|frame| frame.transforms[3] == extra)
    );
}
