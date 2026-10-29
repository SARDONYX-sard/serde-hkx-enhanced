//! Export skeleton animation data as FBX.
//!
//! The scene is written by this crate without any C library:
//!
//! - every Havok bone becomes a `Model` of type `LimbNode` with a `Skeleton`
//!   node attribute, so Blender imports the hierarchy as an armature;
//! - every bone gets `Lcl Translation`/`Lcl Rotation`/`Lcl Scaling` curves
//!   keyed at each sampled Havok frame;
//! - the scene frame rate is written as `TimeMode` (falling back to
//!   `CustomFrameRate`) so that importers place each key on an integer frame.
//!
//! Importers interpolate between keys themselves, and the FBX rotation curve
//! is Euler. If keys fall between frames (e.g. 30fps keys in a 24fps scene),
//! the displayed pose is an Euler blend, not the Havok pose.
//!
//! # FBX primer
//!
//! An FBX file is a tree of nodes `Name: prop, prop, ... { children }`
//! (see the `node` module for the binary encoding). A scene is described by four parts:
//!
//! - `GlobalSettings`: axes, unit and frame rate of the whole file;
//! - `Objects`: every object (bone, mesh, animation curve, ...). Each object
//!   node is `Type: <i64 id>, "<name>\0\x01<class>", "<subclass>" { ... }`;
//! - `Connections`: how objects are linked, by id. There is no nesting in
//!   `Objects`; the hierarchy and every reference live here:
//!   - `C: "OO", child, parent`: object to object (child bone -> parent
//!     bone, mesh geometry -> mesh model, layer -> stack, ...). Parent `0` is
//!     the scene root.
//!   - `C: "OP", child, parent, "property"`: object to a property of
//!     another object (animation curve node -> `"Lcl Rotation"` of a bone,
//!     texture -> `"DiffuseColor"` of a material, ...).
//! - `Definitions`: object counts per type (and optional property templates
//!   holding default values).
//!
//! Object properties live in `Properties70`, one `P` per property:
//!
//! ```text
//! P: "<name>", "<type>", "<label>", "<flags>", <value>...
//! P: "Lcl Translation", "Lcl Translation", "", "A", 1.0, 2.0, 3.0
//! ```
//!
//! `flags` contains `A` when the property is animatable.
//!
//! # What this module writes (ASCII form, 1 bone, ids shortened)
//!
//! ```text
//! FBXHeaderExtension: { FBXHeaderVersion: 1003  FBXVersion: 7400 ... }
//! GlobalSettings: {
//!     Properties70: {
//!         P: "UpAxis", "int", "Integer", "", 2          ; Z up
//!         P: "FrontAxis", "int", "Integer", "", 1       ; Y ...
//!         P: "FrontAxisSign", "int", "Integer", "", -1  ; ... negative: -Y front
//!         P: "UnitScaleFactor", "double", "Number", "", 10   ; 1 unit = 10 cm
//!         P: "TimeMode", "enum", "", "", 6              ; 6 = 30 fps
//!         P: "CustomFrameRate", "double", "Number", "", 30
//!     }
//! }
//! Objects: {
//!     NodeAttribute: 2, "NodeAttribute::Pelvis", "LimbNode" { TypeFlags: "Skeleton" }
//!     Model: 1, "Model::Pelvis", "LimbNode" {
//!         Properties70: {
//!             P: "Lcl Translation", "Lcl Translation", "", "A", 0, -49.4, 90.8   ; parent space
//!             P: "Lcl Rotation", "Lcl Rotation", "", "A", -90, 2.1, 90           ; XYZ Euler, degrees
//!             P: "Lcl Scaling", "Lcl Scaling", "", "A", 1, 1, 1
//!         }
//!     }
//!     AnimationStack: 10, "AnimStack::getup", "" { LocalStart/LocalStop in KTime }
//!     AnimationLayer: 11, "AnimLayer::BaseLayer", ""
//!     AnimationCurveNode: 12, "AnimCurveNode::R", "" { P: "d|X" ... "d|Y" ... "d|Z" (defaults) }
//!     AnimationCurve: 13, "AnimCurve::", "" { KeyTime: *46 {...}  KeyValueFloat: *46 {...} }
//! }
//! Connections: {
//!     C: "OO", 1, 0                    ; bone Pelvis -> scene root
//!     C: "OO", 2, 1                    ; node attribute -> bone
//!     C: "OO", 11, 10                  ; layer -> stack
//!     C: "OO", 12, 11                  ; curve node -> layer
//!     C: "OP", 12, 1, "Lcl Rotation"   ; curve node animates the bone's rotation
//!     C: "OP", 13, 12, "d|X"           ; curve = X channel of the curve node
//! }
//! Takes: { Current: "getup"  Take: "getup" { LocalTime: 0, <stop> } }
//! ```
//!
//! Meshes, skinning, materials and textures are added by the `mesh` module.
mod anim;
mod ascii;
mod binary;
mod geometry;
mod material;
mod mesh;
mod node;
mod scene;
mod settings;

use serde_spline::hkx::{Animation, Skeleton};

use self::anim::{animation_frame_rate, key_times, snap_frame_rate};
pub use self::mesh::{Cluster, Mesh, Texture, TextureSlot};
use self::node::{Node, Property};
use self::scene::{SceneBuilder, rest_world_matrices};
use self::settings::{CREATOR, FILE_ID, global_settings, header_extension, takes};
use crate::error::Error;

/// Configuration for exporting a Havok animation as FBX.
#[derive(Debug)]
pub struct Config<'a> {
    /// Scene frame rate.
    ///
    /// When `None`, the frame rate of the Havok animation
    /// (`(num_frames - 1) / duration`) is used, which places every key on an
    /// integer frame.
    pub fps: Option<f32>,

    /// Target fbx format.
    pub format: Format,

    /// Name of the animation stack (Blender: action name suffix).
    pub take_name: String,

    /// Meshes skinned to the skeleton (e.g. from a NIF, see [`crate::nif`]).
    ///
    /// Borrowed: the same meshes (with embedded textures) are typically
    /// written into every animation of an actor.
    pub meshes: &'a [Mesh],
}

impl Default for Config<'_> {
    fn default() -> Self {
        Self {
            fps: None,
            format: Format::FbxBin,
            take_name: "Take 001".to_owned(),
            meshes: &[],
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Format {
    FbxBin,
    FbxAscii,
}

/// Exports a skeleton and sampled animation as an FBX file.
///
/// # Errors
///
/// - If the animation track count does not match the number of skeleton bones.
/// - If the declared frame count does not match the number of sampled frames.
/// - If a sampled frame does not contain one transform for every skeleton bone.
/// - If a bone has an invalid parent index.
pub fn to_fbx(
    animation: &Animation,
    skeleton: &Skeleton,
    config: Config<'_>,
) -> Result<Vec<u8>, Error> {
    let Config {
        fps,
        format,
        take_name,
        meshes,
    } = config;

    validate_animation(skeleton, animation)?;

    let anim_fps = animation_frame_rate(animation);
    let scene_fps = fps.map_or(anim_fps, |fps| snap_frame_rate(f64::from(fps)));
    // Every curve keys the same frames; built once and borrowed by all of them.
    let times = key_times(animation.frames.len(), anim_fps);
    let stop = times.last().copied().unwrap_or(0);

    let mut scene = SceneBuilder::default();
    let models = scene.add_skeleton(skeleton)?;

    if !meshes.is_empty() {
        let by_name = skeleton
            .bones
            .iter()
            .zip(&models)
            .map(|(bone, &id)| (bone.name.to_lowercase(), id))
            .collect();
        let rest_world = rest_world_matrices(skeleton, &models);
        let _missing = scene.add_meshes(meshes, &by_name, &rest_world);
        #[cfg(feature = "tracing")]
        if !_missing.is_empty() {
            tracing::debug!(missing = ?_missing, "mesh bones missing from the skeleton; their weights are dropped");
        }
    }

    scene.add_animation(&models, animation, &times, &take_name);

    // Top-level nodes, in the order the FBX SDK writes them.
    let nodes = [
        header_extension(),
        // `FileId` (16 raw bytes) and `CreationTime` identify the file for the
        // FBX SDK; importers do not interpret them.
        Node::new("FileId").prop(Property::Raw(FILE_ID[..].into())),
        Node::new("CreationTime").prop("1970-01-01 10:00:00:000"),
        Node::new("Creator").prop(CREATOR),
        global_settings(scene_fps, stop),
        scene.definitions(),
        scene.objects,
        scene.connections,
        takes(&take_name, stop),
    ];

    Ok(match format {
        Format::FbxBin => binary::to_binary(&nodes),
        Format::FbxAscii => ascii::to_ascii(&nodes).into_bytes(),
    })
}

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
