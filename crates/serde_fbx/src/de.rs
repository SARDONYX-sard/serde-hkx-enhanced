use havok_types::{QsTransform, Quaternion, Vector4};
use serde_hkx_features::Format;
use serde_spline::hkx::{Animation, AnimationAnnotation, AnimationFrame, Skeleton, ser::to_hkx};
use std::collections::HashMap;

use crate::Error;

/// Configuration for converting an FBX animation into a Havok animation.
#[derive(Debug)]
pub struct Config {
    /// Animation annotations applied while sampling the FBX animation.
    pub annotations: Vec<AnimationAnnotation>,

    /// Sampling rate of the output animation, in frames per second.
    pub fps: f32,

    /// Target Havok HKX format.
    pub format: Format,

    /// Name of the FBX animation stack to convert.
    ///
    /// When `None`, an animation stack with a non-empty time range is
    /// selected first. If no such stack exists, the first animation stack
    /// is used.
    pub animation_stack: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            annotations: Default::default(),
            fps: 30.0,
            format: Format::Amd64,
            animation_stack: None,
        }
    }
}

/// Converts one FBX animation into Havok HKX animation buffers.
///
/// The skeleton is supplied separately because the target Havok skeleton
/// determines the track ordering of the resulting animation.
///
/// # Errors
///
/// Returns [`Error`] if the FPS is invalid, an FBX document cannot be
/// loaded, an animation stack cannot be found, an FBX bone cannot be
/// mapped to the target skeleton, the animation duration is invalid,
/// or HKX encoding fails.
pub fn from_fbx(bytes: &[u8], skeleton: &Skeleton, config: Config) -> Result<Vec<u8>, Error> {
    let Config {
        annotations,
        fps,
        format,
        animation_stack,
    } = config;

    validate_fps(fps)?;
    let doc = load_fbx(bytes)?;
    let scene = &doc.scene;

    let animation = select_animation(scene, animation_stack.as_deref())?;
    let animation = sample_animation(scene, &animation, skeleton, fps, annotations)?;

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
    #[cfg(feature = "tracing")]
    tracing::debug!(
        requested_name = ?requested_name,
        scene_anim_time_begin = scene.anim.time_begin,
        scene_anim_time_end = scene.anim.time_end,
        anim_stack_count = scene.anim_stacks.len(),
        "Selecting FBX animation"
    );

    #[cfg(feature = "tracing")]
    for (index, stack) in scene.anim_stacks.iter().enumerate() {
        tracing::debug!(
            index,
            stack_name = %stack.element.name,
            stack_time_begin = stack.time_begin,
            stack_time_end = stack.time_end,
            stack_anim_time_begin = stack.anim.time_begin,
            stack_anim_time_end = stack.anim.time_end,
            layer_count = stack.layers.len(),
            "FBX animation stack"
        );

        for (layer_index, layer) in stack.layers.iter().enumerate() {
            tracing::debug!(
                index,
                layer_index,
                layer_name = %layer.element.name,
                "FBX animation layer"
            );
        }
    }

    if scene.anim_stacks.is_empty() {
        return Err(Error::NoAnimationStacks);
    }

    let stack = match requested_name {
        Some(name) => scene
            .anim_stacks
            .iter()
            .find(|stack| stack.element.name == name)
            .ok_or_else(|| Error::AnimationStackNotFound {
                name: name.to_owned(),
            })?,
        None => scene
            .anim_stacks
            .iter()
            .find(|stack| stack.time_end > stack.time_begin)
            .or_else(|| scene.anim_stacks.first())
            .ok_or(Error::NoAnimationStacks)?,
    };

    #[cfg(feature = "tracing")]
    tracing::debug!(
        stack_name = %stack.element.name,
        stack_time_begin = stack.time_begin,
        stack_time_end = stack.time_end,
        stack_anim_time_begin = stack.anim.time_begin,
        stack_anim_time_end = stack.anim.time_end,
        scene_anim_time_begin = scene.anim.time_begin,
        scene_anim_time_end = scene.anim.time_end,
        "Selected FBX animation"
    );

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

    let mapping = build_bone_mapping(scene, skeleton)?;
    let nodes: Vec<&ufbx::Node> = mapping.iter().map(|mapping| mapping.node).collect();
    let (time_begin, time_end) = animation_time_range(stack, &nodes);

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

    #[cfg(feature = "tracing")]
    tracing::debug!(
        skeleton_bones = skeleton.bones.len(),
        mapped_bones = mapping.len(),
        "FBX bone mapping"
    );

    let mut frames = Vec::with_capacity(num_frames as usize);

    for frame_index in 0..num_frames {
        let time = (time_begin + frame_index as f64 / fps as f64).min(time_end);
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

/// Returns the actual animation range represented by the animation curves.
///
/// FBX animation stacks may have an empty or unset time range even when
/// animation curves contain valid keyframes. The curve ranges are therefore
/// used as the authoritative range for sampling.
///
/// Falls back to the stack/animation ranges when no animation curve provides
/// a valid range.
///
/// # Errors
///
/// This function does not return an error. An empty or invalid curve range
/// falls back to the ranges supplied by `ufbx`.
fn animation_time_range(stack: &ufbx::AnimStack, nodes: &[&ufbx::Node]) -> (f64, f64) {
    let mut time_begin = f64::INFINITY;
    let mut time_end = f64::NEG_INFINITY;

    for layer in &stack.layers {
        for node in nodes {
            for prop in layer.find_anim_props(&node.element) {
                for (curve_index, curve) in prop.anim_value.curves.iter().flatten().enumerate() {
                    time_begin = time_begin.min(curve.min_time);
                    time_end = time_end.max(curve.max_time);

                    tracing::debug!(
                        bone = %node.element.name,
                        prop = %prop.anim_value.element.name,
                        curve_index,
                        min_time = curve.min_time,
                        max_time = curve.max_time,
                        keyframes = curve.keyframes.len(),
                        first_time = curve.keyframes.first().map(|key| key.time),
                        last_time = curve.keyframes.last().map(|key| key.time),
                        "FBX animation curve"
                    );
                }
            }
        }
    }

    if time_begin.is_finite() && time_end.is_finite() && time_end >= time_begin {
        (time_begin, time_end)
    } else {
        (stack.time_begin, stack.time_end)
    }
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
