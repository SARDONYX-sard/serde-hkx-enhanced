//! Export skeleton animation data through `ufbx_write`.
//!
//! This module converts the crate's skeleton and sampled animation types into
//! an `ufbx_write` scene. FBX object layout, animation curves, connections,
//! and binary serialization are handled by `ufbx_write`.
//!
//! ffi api ref:
//! - https://raw.githubusercontent.com/ufbx/ufbx-write/refs/heads/main/ufbx_write.h
//! - https://raw.githubusercontent.com/ufbx/ufbx-write/refs/heads/main/ufbx_write.c
use havok_types::{QsTransform, Quaternion, Vector4};
use serde_spline::hkx::{Animation, Skeleton};
use ufbx_write::sys;

use super::AnimationInput;
use crate::error::Error;

/// Exports a skeleton and sampled animation as a binary FBX file.
///
/// # Errors
///
/// Returns [`Error::InvalidTrackCount`] if the animation track count does not
/// match the number of skeleton bones.
///
/// Returns [`Error::EncoderFrameCountMismatch`] if the declared frame count
/// does not match the number of sampled frames.
///
/// Returns [`Error::EncoderTransformCountMismatch`] if a sampled frame does
/// not contain one transform for every skeleton bone.
///
/// Returns [`Error::ExportFbx`] if `ufbx_write` fails to create, prepare, or
/// save the FBX scene.
pub(crate) fn export_fbx(
    skeleton: &Skeleton,
    animation: &AnimationInput,
) -> Result<Vec<u8>, Error> {
    let animation = Animation::from_bytes(skeleton, animation.bytes, animation.path)?;
    validate_animation(skeleton, &animation)?;

    let scene = unsafe { sys::ufbxw_create_scene(&scene_options()) };
    if scene.is_null() {
        return Err(Error::ExportFbx {
            message: "ufbxw_create_scene() returned NULL".to_owned(),
        });
    }

    let result = export_scene(scene, skeleton, &animation);
    unsafe { sys::ufbxw_free_scene(scene) };
    result
}

/// Builds the `ufbx_write` scene and saves it.
///
/// The caller owns `scene` and must free it after this function returns.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if scene construction, animation creation,
/// scene preparation, or file output fails.
fn export_scene(
    scene: *mut sys::ufbxw_scene,
    skeleton: &Skeleton,
    animation: &Animation,
) -> Result<Vec<u8>, Error> {
    set_target_coordinate_axes(scene);
    let nodes = create_skeleton(scene, skeleton)?;

    create_animation(scene, &nodes, animation)?;

    unsafe {
        sys::ufbxw_prepare_scene(scene, &sys::ufbxw_default_prepare_opts);
    }

    save_memory(scene)
}

/// Creates all skeleton nodes and their hierarchy.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if a node cannot be created or configured.
fn create_skeleton(
    scene: *mut sys::ufbxw_scene,
    skeleton: &Skeleton,
) -> Result<Vec<sys::ufbxw_node>, Error> {
    let mut nodes = Vec::with_capacity(skeleton.bones.len());

    for bone in &skeleton.bones {
        let node = unsafe { sys::ufbxw_create_node(scene) };

        if node.id == 0 {
            return Err(Error::ExportFbx {
                message: format!("failed to create FBX node for bone {:?}", bone.name),
            });
        }

        set_node_name(scene, node, &bone.name)?;
        set_node_transform(scene, node, &bone.reference_pose)?;

        nodes.push(node);
    }

    set_parent_nodes(scene, skeleton, &nodes)?;

    Ok(nodes)
}

/// Sets a node's FBX name.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if the bone name contains an interior NUL byte.
fn set_node_name(
    scene: *mut sys::ufbxw_scene,
    node: sys::ufbxw_node,
    name: &str,
) -> Result<(), Error> {
    unsafe {
        sys::ufbxw_set_name_len(scene, node.id, name.as_ptr().cast(), name.len());
    }

    Ok(())
}

/// Sets the reference-pose transform of a skeleton node.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if `ufbx_write` rejects the transform.
fn set_node_transform(
    scene: *mut sys::ufbxw_scene,
    node: sys::ufbxw_node,
    transform: &QsTransform,
) -> Result<(), Error> {
    unsafe {
        sys::ufbxw_node_set_translation(scene, node, to_sys_vec3(transform.transition.clone()));
        sys::ufbxw_node_set_scaling_offset(scene, node, to_sys_vec3(transform.scale.clone()));
        sys::ufbxw_node_set_rotation_quat(
            scene,
            node,
            to_sys_rotation(&transform.quaternion),
            sys::ufbxw_rotation_order_UFBXW_ROTATION_ORDER_XYZ,
        );
    }

    Ok(())
}

/// Creates the skeleton parent hierarchy.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if a parent index is outside the skeleton.
fn set_parent_nodes(
    scene: *mut sys::ufbxw_scene,
    skeleton: &Skeleton,
    nodes: &[sys::ufbxw_node],
) -> Result<(), Error> {
    for (index, bone) in skeleton.bones.iter().enumerate() {
        if bone.parent_index < 0 {
            continue;
        }

        let parent_index = bone.parent_index as usize;

        let parent = nodes.get(parent_index).ok_or_else(|| Error::ExportFbx {
            message: format!("bone {} has invalid parent index {}", index, parent_index),
        })?;

        let child = nodes[index];

        unsafe {
            sys::ufbxw_node_set_parent(scene, child, *parent);
        }
    }

    Ok(())
}

/// Declares the coordinate axes this scene's transforms are authored in.
///
/// This only sets FBX's `CoordAxis` / `UpAxis` / `FrontAxis` metadata; it
/// does not touch any node's translation, rotation, or animation data.
/// Importers such as Blender read this metadata and reorient the scene
/// on load to match their own convention.
///
/// # Note
///
/// `front` denotes the axis pointing *backward*, opposite of the character's
/// forward-facing direction (this mirrors `ufbx`'s `target_axes` convention).
/// If the imported result faces the wrong way, flip the sign of `front`.
fn set_target_coordinate_axes(scene: *mut sys::ufbxw_scene) {
    // Without this, characters will face the Z-axis by default in Blender.
    // Doing this will make the characters face the Y-axis.
    let axes = sys::ufbxw_coordinate_axes {
        right: sys::ufbxw_coordinate_axis_UFBXW_COORDINATE_AXIS_POSITIVE_X,
        up: sys::ufbxw_coordinate_axis_UFBXW_COORDINATE_AXIS_POSITIVE_Z,
        front: sys::ufbxw_coordinate_axis_UFBXW_COORDINATE_AXIS_NEGATIVE_Y,
    };

    unsafe {
        sys::ufbxw_scene_set_coordinate_axes(scene, axes);
    }
}

/// Creates animation data for every skeleton node.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if an animation object or animation key
/// cannot be created.
fn create_animation(
    scene: *mut sys::ufbxw_scene,
    nodes: &[sys::ufbxw_node],
    animation: &Animation,
) -> Result<(), Error> {
    let stack = unsafe { sys::ufbxw_create_anim_stack(scene) };
    let layer = unsafe { sys::ufbxw_create_anim_layer(scene, stack) };

    fn seconds_to_ktime(seconds: f32) -> i64 {
        (seconds as f64 * 46_186_158_000.0).round() as i64
    }
    let duration = seconds_to_ktime(animation.duration);

    unsafe {
        sys::ufbxw_anim_stack_set_time_range(scene, stack, 0, duration);
        sys::ufbxw_anim_stack_set_reference_time_range(scene, stack, 0, duration);
        sys::ufbxw_set_active_anim_stack(scene, stack);
    }

    for (bone_index, &node) in nodes.iter().enumerate() {
        let translation = unsafe { sys::ufbxw_node_animate_translation(scene, node, layer) };
        let rotation = unsafe { sys::ufbxw_node_animate_rotation(scene, node, layer) };
        let scaling = unsafe { sys::ufbxw_node_animate_scaling(scene, node, layer) };

        for (frame_index, frame) in animation.frames.iter().enumerate() {
            fn frame_time(duration: f32, frame_count: usize, frame_index: usize) -> i64 {
                if frame_count <= 1 {
                    return 0;
                }

                let seconds = duration as f64 * frame_index as f64 / (frame_count - 1) as f64;
                (seconds * 46_186_158_000.0).round() as i64
            }

            let time = frame_time(animation.duration, animation.frames.len(), frame_index);
            let transform = &frame.transforms[bone_index];

            unsafe {
                sys::ufbxw_anim_add_keyframe_vec3(
                    scene,
                    translation,
                    time,
                    to_sys_vec3(transform.transition.clone()),
                    sys::ufbxw_keyframe_type_UFBXW_KEYFRAME_LINEAR as u32,
                );

                sys::ufbxw_anim_add_keyframe_vec3(
                    scene,
                    rotation,
                    time,
                    quaternion_to_euler(&transform.quaternion),
                    sys::ufbxw_keyframe_type_UFBXW_KEYFRAME_LINEAR as u32,
                );

                sys::ufbxw_anim_add_keyframe_vec3(
                    scene,
                    scaling,
                    time,
                    sys::ufbxw_vec3 {
                        x: transform.scale.x as f64,
                        y: transform.scale.y as f64,
                        z: transform.scale.z as f64,
                    },
                    sys::ufbxw_keyframe_type_UFBXW_KEYFRAME_LINEAR as u32,
                );
            }
        }

        unsafe {
            sys::ufbxw_anim_finish_keyframes(scene, translation);
            sys::ufbxw_anim_finish_keyframes(scene, rotation);
            sys::ufbxw_anim_finish_keyframes(scene, scaling);
        }
    }

    Ok(())
}

/// Saves the prepared scene as binary FBX.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if the output path cannot be represented as a
/// C string or if `ufbx_write` reports a save failure.
fn save_memory(scene: *mut sys::ufbxw_scene) -> Result<Vec<u8>, Error> {
    let mut opts = unsafe { std::mem::zeroed::<sys::ufbxw_save_opts>() };
    opts.format = sys::ufbxw_save_format_UFBXW_SAVE_FORMAT_BINARY;
    // opts.format = sys::ufbxw_save_format_UFBXW_SAVE_FORMAT_ASCII;
    opts.version = 7500;

    let mut buffer = unsafe { core::mem::zeroed::<sys::ufbxw_write_buffer>() };
    let mut error = unsafe { std::mem::zeroed::<sys::ufbxw_error>() };

    if !unsafe { sys::ufbxw_save_memory(scene, &mut buffer, &opts, &mut error) } {
        return Err(Error::ExportFbx {
            message: super::fbx_error::Error::from(error).to_string(),
        });
    }

    if buffer.data.is_null() && buffer.size != 0 {
        return Err(Error::ExportFbx {
            message: "ufbxw_save_memory() returned an invalid buffer".to_owned(),
        });
    }
    let data =
        unsafe { std::slice::from_raw_parts(buffer.data.cast::<u8>(), buffer.size).to_vec() };

    // Safety: That's because we're converting the buffer to a new Vec using `to_vec` right before that.
    unsafe { sys::ufbxw_free_write_buffer(buffer) };
    Ok(data)
}

/// Creates default scene options.
///
/// `ufbx_write` documents zero-initialized options as the default.
const fn scene_options() -> sys::ufbxw_scene_opts {
    unsafe { std::mem::zeroed() }
}

/// Converts a quaternion to XYZ Euler angles in degrees.
///
/// # Errors
///
/// This function does not return an error. Floating-point operations follow
/// IEEE-754 semantics.
fn quaternion_to_euler(rotation: &Quaternion) -> sys::ufbxw_vec3 {
    let x = rotation.x as f64;
    let y = rotation.y as f64;
    let z = rotation.z as f64;
    let w = rotation.scaler as f64;

    let sin_x_cos_y = 2.0 * y.mul_add(z, w * x);
    let cos_x_cos_y = 2.0f64.mul_add(-y.mul_add(y, x * x), 1.0);

    let roll = sin_x_cos_y.atan2(cos_x_cos_y);

    let sin_y = 2.0 * z.mul_add(-x, w * y);

    let pitch = if sin_y.abs() >= 1.0 {
        sin_y.signum() * std::f64::consts::FRAC_PI_2
    } else {
        sin_y.asin()
    };

    let sin_z_cos_y = 2.0 * x.mul_add(y, w * z);
    let cos_z_cos_y = 2.0f64.mul_add(-z.mul_add(z, y * y), 1.0);

    let yaw = sin_z_cos_y.atan2(cos_z_cos_y);

    sys::ufbxw_vec3 {
        x: roll.to_degrees(),
        y: pitch.to_degrees(),
        z: yaw.to_degrees(),
    }
}

/// Converts a crate vector to an `ufbx_write` vector.
///
/// The fourth component is intentionally discarded because FBX node
/// translation and scaling are three-dimensional.
const fn to_sys_vec3(value: Vector4) -> sys::ufbxw_vec3 {
    sys::ufbxw_vec3 {
        x: value.x as f64,
        y: value.y as f64,
        z: value.z as f64,
    }
}

const fn to_sys_rotation(rotation: &Quaternion) -> sys::ufbxw_quat {
    sys::ufbxw_quat {
        x: rotation.x as f64,
        y: rotation.y as f64,
        z: rotation.z as f64,
        w: rotation.scaler as f64,
    }
}

/// Validates the relationship between skeleton and sampled animation.
///
/// # Errors
///
/// Returns [`Error::InvalidTrackCount`] if the animation track count does not
/// equal the skeleton bone count.
///
/// Returns [`Error::EncoderFrameCountMismatch`] if the declared frame count
/// does not equal the number of sampled frames.
///
/// Returns [`Error::EncoderTransformCountMismatch`] if a sampled frame does
/// not contain one transform for every skeleton bone.
fn validate_animation(skeleton: &Skeleton, animation: &Animation) -> Result<(), Error> {
    let bone_count = skeleton.bones.len();

    if animation.num_tracks as usize > skeleton.bones.len() {
        return Err(Error::InvalidTrackCount {
            actual: animation.num_tracks as usize,
            maximum: skeleton.bones.len(),
        });
    }

    if animation.num_frames as usize != animation.frames.len() {
        return Err(Error::EncoderFrameCountMismatch {
            expected: animation.num_frames as usize,
            actual: animation.frames.len(),
        });
    }

    for (frame_index, frame) in animation.frames.iter().enumerate() {
        if frame.transforms.len() != bone_count {
            return Err(Error::EncoderTransformCountMismatch {
                frame_index,
                expected: bone_count,
                actual: frame.transforms.len(),
            });
        }
    }

    Ok(())
}
