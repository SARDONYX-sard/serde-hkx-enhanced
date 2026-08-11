use havok_types::{QsTransform, Quaternion, Vector4};
use rayon::{iter::Either, prelude::*};
use serde_hkx_features::Format;
use serde_spline::hkx::ser::to_hkx;
use std::collections::HashMap;
use std::path::Path;

use serde_spline::hkx::{Animation, AnimationAnnotation, AnimationFrame, Skeleton};

use crate::Error;

/// Input FBX animation.
pub struct AnimationInput<'a> {
    /// FBX document bytes.
    pub bytes: &'a [u8],

    /// Source FBX path.
    pub path: &'a Path,

    /// Optional animation stack name.
    ///
    /// When `None`, the first animation stack is used.
    pub animation_stack: Option<&'a str>,

    /// Additional Havok annotation tracks.
    pub annotations: Vec<AnimationAnnotation>,
}

/// Converts FBX animations into Havok HKX animations.
///
/// # Errors
/// If the skeleton cannot be decoded, an FBX animation
/// cannot be loaded or sampled, or the resulting HKX animation cannot be
/// encoded.
pub fn fbx_to_hkx_bytes_vec<P>(
    skeleton_bytes: &[u8],
    skeleton_path: P,
    fbx_animations: Vec<AnimationInput<'_>>,
    fps: f32,
    format: Format,
) -> Result<Vec<Vec<u8>>, Error>
where
    P: AsRef<Path>,
{
    let skeleton = Skeleton::from_bytes(skeleton_bytes, skeleton_path.as_ref())?;

    let (outputs, errors): (Vec<Vec<u8>>, Vec<Error>) =
        fbx_animations.into_par_iter().partition_map(|animation| {
            match fbx_to_hkx(&skeleton, animation, fps, format) {
                Ok(output) => Either::Left(output),
                Err(error) => Either::Right(error),
            }
        });

    if errors.is_empty() {
        Ok(outputs)
    } else {
        Err(Error::Errors { errors })
    }
}

/// Converts one or more FBX animations into Havok HKX animation buffers.
///
/// The skeleton is supplied separately because the target Havok skeleton
/// determines the track ordering of the resulting animation.
///
/// # Errors
///
/// Returns [`Error`] if the FPS is invalid, an FBX document cannot be
/// loaded, an animation stack cannot be found, an FBX bone cannot be mapped
/// to the target skeleton, or HKX encoding fails.
pub(crate) fn fbx_to_hkx(
    skeleton: &Skeleton,
    input: AnimationInput<'_>,
    fps: f32,
    format: Format,
) -> Result<Vec<u8>, Error> {
    validate_fps(fps)?;

    let doc = load_fbx(input.bytes)?;
    let scene_root = &doc.scene;

    let animation = select_animation(scene_root, input.animation_stack)?;
    let anim = create_animation(scene_root, &animation)?;
    let animation = sample_animation(
        scene_root,
        &animation,
        &anim,
        skeleton,
        fps,
        input.annotations,
    )?;

    Ok(to_hkx(skeleton, &animation, fps, format)?)
}

fn validate_fps(fps: f32) -> Result<(), Error> {
    if !fps.is_finite() || fps <= 0.0 {
        return Err(Error::InvalidFps { fps });
    }

    Ok(())
}

struct FbxDocument {
    scene: ufbx::SceneRoot,
}

fn load_fbx(bytes: &[u8]) -> Result<FbxDocument, Error> {
    let scene =
        ufbx::load_memory(bytes, ufbx::LoadOpts::default()).map_err(|error| Error::LoadFbx {
            message: error.info().to_string(),
        })?;

    Ok(FbxDocument { scene })
}

struct BoneMapping<'a> {
    node: &'a ufbx::Node,
    track_index: usize,
}

fn build_bone_mapping<'a>(
    scene: &'a ufbx::Scene,
    skeleton: &Skeleton,
) -> Result<Vec<BoneMapping<'a>>, Error> {
    let mut nodes_by_name = HashMap::new();

    for node in &scene.nodes {
        let name = node.element.name.as_ref();

        if nodes_by_name.insert(name, node).is_some() {
            return Err(Error::DuplicateBone {
                name: name.to_string(),
            });
        }
    }

    let mut mapping = Vec::with_capacity(skeleton.bones.len());

    for (track_index, bone) in skeleton.bones.iter().enumerate() {
        let node = nodes_by_name
            .get(bone.name.as_str())
            .ok_or_else(|| Error::BoneNotFound {
                name: bone.name.clone(),
            })?;

        mapping.push(BoneMapping { node, track_index });
    }

    Ok(mapping)
}

struct FbxAnimation<'a> {
    stack: &'a ufbx::AnimStack,
}

fn select_animation<'a>(
    scene: &'a ufbx::SceneRoot,
    requested_name: Option<&str>,
) -> Result<FbxAnimation<'a>, Error> {
    let stacks = &scene.anim_stacks;

    if stacks.is_empty() {
        return Err(Error::NoAnimationStacks);
    }

    let stack = match requested_name {
        Some(name) => stacks
            .iter()
            .find(|stack| stack.element.name == name)
            .ok_or_else(|| Error::AnimationStackNotFound {
                name: name.to_owned(),
            })?,

        None => &stacks[0],
    };

    Ok(FbxAnimation { stack })
}

fn create_animation(
    scene: &ufbx::Scene,
    animation: &FbxAnimation,
) -> Result<ufbx::AnimRoot, Error> {
    let mut layer_ids = Vec::with_capacity(animation.stack.layers.len());

    for layer in &animation.stack.layers {
        let layer_id = layer.element.typed_id as usize;

        if layer_id >= scene.anim_layers.len() {
            return Err(Error::LoadFbx {
                message: format!(
                    "Animation layer typed_id {} is out of bounds for scene.anim_layers (len={})",
                    layer_id,
                    scene.anim_layers.len(),
                ),
            });
        }

        layer_ids.push(layer_id as u32);
    }

    let opts = ufbx::AnimOpts {
        layer_ids: ufbx::ListOpt::Owned(layer_ids),
        ..Default::default()
    };

    ufbx::create_anim(scene, opts).map_err(|error| Error::LoadFbx {
        message: error.description.as_ref().to_string(),
    })
}

fn sample_animation(
    scene: &ufbx::Scene,
    animation: &FbxAnimation,
    anim: &ufbx::AnimRoot,
    skeleton: &Skeleton,
    fps: f32,
    annotations: Vec<AnimationAnnotation>,
) -> Result<Animation, Error> {
    let duration = (animation.stack.time_end - animation.stack.time_begin) as f32;
    if !duration.is_finite() || duration < 0.0 {
        return Err(Error::InvalidDuration { duration });
    }

    let num_frames = (duration * fps).ceil() as u32 + 1;
    if num_frames == 0 {
        return Err(Error::InvalidFrameCount {
            count: num_frames as u64,
        });
    }

    let mapping = build_bone_mapping(scene, skeleton)?;

    let mut frames = Vec::with_capacity(num_frames as usize);
    for frame_index in 0..num_frames {
        let time = frame_index as f64 / fps as f64;
        let transforms = sample_frame(anim, &mapping, skeleton, time);
        frames.push(AnimationFrame { transforms });
    }

    Ok(Animation {
        duration,
        num_frames,
        num_tracks: skeleton.bones.len() as u32,
        frames,
        annotations,
    })
}

fn sample_frame(
    anim: &ufbx::AnimRoot,
    mapping: &[BoneMapping],
    skeleton: &Skeleton,
    time: f64,
) -> Vec<QsTransform> {
    const QS_TRANSFORM_IDENTITY: QsTransform = QsTransform {
        transition: Vector4 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 0.0,
        },
        quaternion: Quaternion {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            scaler: 1.0,
        },
        scale: Vector4 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
            w: 1.0,
        },
    };

    let mut transforms = vec![QS_TRANSFORM_IDENTITY; skeleton.bones.len()];
    for mapping in mapping {
        let transform = mapping.node.evaluate_transform(anim, time);
        transforms[mapping.track_index] = convert_transform(transform);
    }

    transforms
}

const fn convert_transform(transform: ufbx::Transform) -> QsTransform {
    QsTransform {
        transition: Vector4 {
            x: transform.translation.x as f32,
            y: transform.translation.y as f32,
            z: transform.translation.z as f32,
            w: 0.0,
        },
        quaternion: Quaternion {
            x: transform.rotation.x as f32,
            y: transform.rotation.y as f32,
            z: transform.rotation.z as f32,
            scaler: transform.rotation.w as f32,
        },
        scale: Vector4 {
            x: transform.scale.x as f32,
            y: transform.scale.y as f32,
            z: transform.scale.z as f32,
            w: 1.0,
        },
    }
}
