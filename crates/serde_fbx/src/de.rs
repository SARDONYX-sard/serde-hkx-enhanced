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

/// Converts FBX animations into Havok HKX animation buffers.
///
/// # Errors
///
/// Returns [`Error`] if the skeleton cannot be decoded, an FBX document
/// cannot be loaded or sampled, an animation stack cannot be found, an FBX
/// bone cannot be mapped to the target skeleton, or HKX encoding fails.
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
/// to the target skeleton, the animation duration is invalid, or HKX
/// encoding fails.
pub(crate) fn fbx_to_hkx(
    skeleton: &Skeleton,
    input: AnimationInput<'_>,
    fps: f32,
    format: Format,
) -> Result<Vec<u8>, Error> {
    validate_fps(fps)?;

    let doc = load_fbx(input.bytes)?;
    let scene = &doc.scene;

    let animation = select_animation(scene, input.animation_stack)?;
    let animation = sample_animation(scene, &animation, skeleton, fps, input.annotations)?;

    #[cfg(feature = "tracing")]
    tracing::debug!(
        skeleton_bones = skeleton.bones.len(),
        animation_tracks = animation.num_tracks,
        frame_count = animation.num_frames,
        first_frame_transforms = animation
            .frames
            .first()
            .map_or(0, |frame| frame.transforms.len()),
        "FBX animation sampled"
    );

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

fn sample_animation(
    scene: &ufbx::Scene,
    animation: &FbxAnimation,
    skeleton: &Skeleton,
    fps: f32,
    annotations: Vec<AnimationAnnotation>,
) -> Result<Animation, Error> {
    let stack = animation.stack;
    let anim = &stack.anim;

    #[cfg(feature = "tracing")]
    tracing::debug!(
        stack_name = %stack.element.name,
        stack_time_begin = stack.time_begin,
        stack_time_end = stack.time_end,
        anim_time_begin = anim.time_begin,
        anim_time_end = anim.time_end,
        layer_count = stack.layers.len(),
        custom = anim.custom,
        "FBX animation range"
    );

    let (time_begin, time_end) = animation_time_range(stack);

    let duration = (time_end - time_begin) as f32;

    if !duration.is_finite() || duration < 0.0 {
        return Err(Error::InvalidDuration { duration });
    }

    #[cfg(feature = "tracing")]
    tracing::debug!(
        stack_name = %stack.element.name,
        time_begin,
        time_end,
        duration,
        fps,
        "sampling FBX animation"
    );

    let num_frames = (duration * fps).ceil() as u32 + 1;

    if num_frames == 0 {
        return Err(Error::InvalidFrameCount {
            count: num_frames as u64,
        });
    }

    let mapping = build_bone_mapping(scene, skeleton)?;
    #[cfg(feature = "tracing")]
    tracing::debug!(
        skeleton_bones = skeleton.bones.len(),
        mapped_bones = mapping.len(),
        "FBX bone mapping"
    );
    let mut frames = Vec::with_capacity(num_frames as usize);

    for frame_index in 0..num_frames {
        let time = animation.stack.time_begin + frame_index as f64 / fps as f64;
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

fn animation_time_range(stack: &ufbx::AnimStack) -> (f64, f64) {
    let stack_begin = stack.time_begin;
    let stack_end = stack.time_end;

    if stack_end > stack_begin {
        return (stack_begin, stack_end);
    }

    let anim_begin = stack.anim.time_begin;
    let anim_end = stack.anim.time_end;

    if anim_end > anim_begin {
        return (anim_begin, anim_end);
    }

    (stack_begin, stack_end)
}

fn sample_frame(
    anim: &ufbx::Anim,
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
