//! Convert FBX animations into Havok HKX animations.
//!
//! The FBX file is parsed and evaluated by this crate (`element`, `binary`, `ascii`, `scene` modules);
//! no C library is involved.
mod ascii;
mod binary;
mod element;
mod scene;

use std::collections::HashMap;

use serde_hkx_features::Format;
use serde_spline::hkx::{Animation, AnimationAnnotation, AnimationFrame, Skeleton, ser::to_hkx};

use self::scene::{AnimStack, KTIME_PER_SECOND, Scene};
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
    let format = config.format;
    let fps = config.fps;
    let animation = fbx_to_animation(bytes, skeleton, config)?;
    Ok(to_hkx(skeleton, &animation, fps, format)?)
}

/// Samples one FBX animation into Havok track order.
///
/// # Errors
/// See [`from_fbx`].
pub fn fbx_to_animation(
    bytes: &[u8],
    skeleton: &Skeleton,
    config: Config,
) -> Result<Animation, Error> {
    let Config {
        annotations,
        fps,
        format: _,
        animation_stack,
    } = config;

    validate_fps(fps)?;
    let nodes = element::parse(bytes)?;
    let scene = Scene::new(&nodes)?;

    let stack = select_animation(&scene, animation_stack.as_deref())?;
    sample_animation(&scene, stack, skeleton, fps, annotations)
}

fn validate_fps(fps: f32) -> Result<(), Error> {
    if !fps.is_finite() || fps <= 0.0 {
        return Err(Error::InvalidFps { fps });
    }

    Ok(())
}

/// Maps every skeleton bone to its FBX model id, in track order.
fn build_bone_mapping(scene: &Scene, skeleton: &Skeleton) -> Result<Vec<i64>, Error> {
    let mut models_by_name: HashMap<&str, Vec<i64>> = HashMap::new();
    for (&id, model) in &scene.models {
        models_by_name
            .entry(model.name)
            .or_default()
            .push(id);
    }

    skeleton
        .bones
        .iter()
        .map(
            |bone| match models_by_name.get(bone.name.as_str()).map(Vec::as_slice) {
                Some([id]) => Ok(*id),
                Some(_) => Err(Error::DuplicateBone {
                    name: bone.name.clone(),
                }),
                None => Err(Error::BoneNotFound {
                    name: bone.name.clone(),
                }),
            },
        )
        .collect()
}

fn select_animation<'s, 'a>(
    scene: &'s Scene<'a>,
    requested_name: Option<&str>,
) -> Result<&'s AnimStack<'a>, Error> {
    #[cfg(feature = "tracing")]
    for (index, stack) in scene.stacks.iter().enumerate() {
        tracing::debug!(
            index,
            stack_name = %stack.name,
            local_start = ?stack.local_start,
            local_stop = ?stack.local_stop,
            "FBX animation stack"
        );
    }

    if scene.stacks.is_empty() {
        return Err(Error::NoAnimationStacks);
    }

    requested_name.map_or_else(
        || {
            scene
                .stacks
                .iter()
                .find(|stack| matches!((stack.local_start, stack.local_stop), (Some(a), Some(b)) if b > a))
                .or_else(|| scene.stacks.first())
                .ok_or(Error::NoAnimationStacks)
        },
        |name| {
            scene
                .stacks
                .iter()
                .find(|stack| stack.name == name)
                .ok_or_else(|| Error::AnimationStackNotFound {
                    name: name.to_owned(),
                })
        },
    )
}

fn sample_animation(
    scene: &Scene,
    stack: &AnimStack,
    skeleton: &Skeleton,
    fps: f32,
    annotations: Vec<AnimationAnnotation>,
) -> Result<Animation, Error> {
    let models = build_bone_mapping(scene, skeleton)?;

    // The curves are authoritative: stacks may have an empty or unset range
    // even when the curves contain valid keys.
    let (begin, end) = scene
        .curve_time_range(stack, &models)
        .or_else(|| stack.local_start.zip(stack.local_stop))
        .unwrap_or((0, 0));

    let duration = ((end - begin) as f64 / KTIME_PER_SECOND) as f32;
    if !duration.is_finite() || duration < 0.0 {
        return Err(Error::InvalidDuration { duration });
    }

    // Round away f32 noise so that e.g. 1.5s * 30fps is 45 intervals, not 46.
    let intervals = f64::from(duration)
        .mul_add(f64::from(fps), -1.0e-4)
        .ceil()
        .max(0.0) as u32;
    let num_frames = intervals + 1;

    #[cfg(feature = "tracing")]
    tracing::debug!(stack_name = %stack.name, begin, end, duration, fps, num_frames, "sampling FBX animation");

    // Bone-local values are used as-is. Unit and axis conversion live on the
    // ancestors (`UnitScaleFactor`, or e.g. Blender's `Armature` node with
    // scale 10 and a -90° X rotation), which do not affect local transforms.
    let frames = (0..num_frames)
        .map(|frame| {
            let time =
                begin + (f64::from(frame) / f64::from(fps) * KTIME_PER_SECOND).round() as i64;
            let transforms = models
                .iter()
                .map(|&model| {
                    scene
                        .local_transform(model, Some(stack), time.min(end))
                        .to_qs_transform()
                })
                .collect();
            AnimationFrame { transforms }
        })
        .collect();

    Ok(Animation {
        duration,
        num_frames,
        num_tracks: skeleton.bones.len() as u32,
        frames,
        annotations,
    })
}
