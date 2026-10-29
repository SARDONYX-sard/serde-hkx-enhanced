//! Scene objects shared by every export: bone models, connections, definitions.

use std::collections::HashMap;

use havok_types::QsTransform;
use rig_math::{Mat4, compatible_euler, quat_to_euler_xyz};
use serde_spline::hkx::Skeleton;

use super::node::{Node, Properties70, Property, object_name};
use crate::error::Error;

/// Objects and connections collected while building the scene.
pub(super) struct SceneBuilder<'a> {
    pub(super) objects: Node<'a>,
    pub(super) connections: Node<'a>,
    pub(super) next_id: i64,
    pub(super) models: usize,
    pub(super) node_attributes: usize,
    pub(super) geometries: usize,
    pub(super) deformers: usize,
    pub(super) poses: usize,
    pub(super) materials: usize,
    pub(super) textures: usize,
    pub(super) videos: usize,
    pub(super) curve_nodes: usize,
    pub(super) curves: usize,
}

impl Default for SceneBuilder<'_> {
    fn default() -> Self {
        Self {
            objects: Node::new("Objects"),
            connections: Node::new("Connections"),
            next_id: 1_000_000,
            models: 0,
            node_attributes: 0,
            geometries: 0,
            deformers: 0,
            poses: 0,
            materials: 0,
            textures: 0,
            videos: 0,
            curve_nodes: 0,
            curves: 0,
        }
    }
}

impl<'a> SceneBuilder<'a> {
    /// Object ids are arbitrary unique `i64`s; `0` is reserved for the scene root.
    pub(super) const fn new_id(&mut self) -> i64 {
        self.next_id += 1;
        self.next_id
    }

    /// `C: "OO", child, parent`
    pub(super) fn connect(&mut self, child: i64, parent: i64) {
        self.connections
            .push(Node::new("C").prop("OO").prop(child).prop(parent));
    }

    /// `C: "OP", child, parent, property`
    pub(super) fn connect_prop(&mut self, child: i64, parent: i64, property: &'a str) {
        self.connections.push(
            Node::new("C")
                .prop("OP")
                .prop(child)
                .prop(parent)
                .prop(property),
        );
    }

    /// Creates one `LimbNode` model per bone and returns their ids.
    ///
    /// A bone is two objects:
    ///
    /// ```text
    /// NodeAttribute: <attr id>, "NodeAttribute::<bone>", "LimbNode" {
    ///     Properties70: { P: "Size", "double", "Number", "", 1 }   ; display size only
    ///     TypeFlags: "Skeleton"
    /// }
    /// Model: <id>, "Model::<bone>", "LimbNode" {                  ; "LimbNode" = bone
    ///     Version: 232                                           ; Model node format version
    ///     Properties70: { Lcl Translation / Lcl Rotation / Lcl Scaling = reference pose }
    ///     Shading: T  Culling: "CullingOff"                       ; viewport flags, unused by Blender
    /// }
    /// C: "OO", <attr id>, <id>          ; attribute belongs to the model
    /// C: "OO", <id>, <parent id or 0>   ; bone hierarchy
    /// ```
    ///
    /// The `Lcl` values are the Havok reference pose, i.e. the transform
    /// relative to the parent bone.
    pub(super) fn add_skeleton(&mut self, skeleton: &Skeleton) -> Result<Vec<i64>, Error> {
        let mut models = Vec::with_capacity(skeleton.bones.len());

        for bone in &skeleton.bones {
            let model = self.new_id();
            let attribute = self.new_id();
            let [t, r, s] = transform_to_trs(&bone.reference_pose, None);

            self.objects.push(
                Node::new("NodeAttribute")
                    .prop(attribute)
                    .prop(object_name(&bone.name, "NodeAttribute"))
                    .prop("LimbNode")
                    .child(Properties70::new().double("Size", 1.0).build())
                    .value("TypeFlags", "Skeleton"),
            );

            self.objects.push(
                Node::new("Model")
                    .prop(model)
                    .prop(object_name(&bone.name, "Model"))
                    .prop("LimbNode")
                    .value("Version", 232)
                    .child(model_properties(t, r, s).build())
                    .value("MultiLayer", 0) // legacy (FBX 6) take flags, always 0
                    .value("MultiTake", 0)
                    .value("Shading", Property::Bool(true))
                    .value("Culling", "CullingOff"),
            );

            self.connect(attribute, model);
            models.push(model);
        }
        self.models += models.len();
        self.node_attributes += models.len();

        for (index, bone) in skeleton.bones.iter().enumerate() {
            let parent = match usize::try_from(bone.parent_index) {
                Ok(parent) => *models.get(parent).ok_or_else(|| Error::ExportFbx {
                    message: format!(
                        "bone {index} has invalid parent index {}",
                        bone.parent_index
                    ),
                })?,
                Err(_) => 0, // scene root
            };
            self.connect(models[index], parent);
        }

        Ok(models)
    }

    /// `Definitions`: number of objects per type.
    ///
    /// ```text
    /// Definitions: {
    ///     Version: 100
    ///     Count: <total>
    ///     ObjectType: "Model" { Count: 44 }
    ///     ...
    /// }
    /// ```
    ///
    /// An `ObjectType` may also hold a `PropertyTemplate` with default property
    /// values; none is written here, so every object lists its properties.
    pub(super) fn definitions(&self) -> Node<'a> {
        let types = [
            ("GlobalSettings", 1),
            ("Model", self.models),
            ("NodeAttribute", self.node_attributes),
            ("Geometry", self.geometries),
            ("Deformer", self.deformers),
            ("Pose", self.poses),
            ("Material", self.materials),
            ("Texture", self.textures),
            ("Video", self.videos),
            ("AnimationStack", 1),
            ("AnimationLayer", 1),
            ("AnimationCurveNode", self.curve_nodes),
            ("AnimationCurve", self.curves),
        ];
        let total: usize = types.iter().map(|(_, count)| count).sum();

        let mut definitions = Node::new("Definitions")
            .value("Version", 100)
            .value("Count", total as i32);
        for (name, count) in types.into_iter().filter(|(_, count)| *count > 0) {
            definitions.push(
                Node::new("ObjectType")
                    .prop(name)
                    .value("Count", count as i32),
            );
        }
        definitions
    }
}

/// World matrix of every skeleton model in its reference pose.
pub(super) fn rest_world_matrices(skeleton: &Skeleton, models: &[i64]) -> HashMap<i64, Mat4> {
    let mut world: Vec<Mat4> = Vec::with_capacity(skeleton.bones.len());
    for bone in &skeleton.bones {
        let local = Mat4::from_qs_transform(&bone.reference_pose);
        let parent = usize::try_from(bone.parent_index)
            .ok()
            .and_then(|p| world.get(p).copied());
        world.push(parent.map_or(local, |parent| parent * local));
    }
    models.iter().copied().zip(world).collect()
}

/// Converts a Havok transform to FBX translation, XYZ Euler degrees, and scale.
///
/// `prev_euler` keeps consecutive keys on the closest equivalent Euler angles.
pub(super) fn transform_to_trs(
    transform: &QsTransform,
    prev_euler: Option<[f64; 3]>,
) -> [[f64; 3]; 3] {
    let t = &transform.transition;
    let s = &transform.scale;

    let q = &transform.quaternion;
    let euler = quat_to_euler_xyz([q.x, q.y, q.z, q.scaler].map(f64::from));
    let euler = prev_euler.map_or(euler, |prev| compatible_euler(euler, prev));

    [
        [f64::from(t.x), f64::from(t.y), f64::from(t.z)],
        euler,
        [f64::from(s.x), f64::from(s.y), f64::from(s.z)],
    ]
}

/// `Properties70` of a `Model` with the given local transform.
///
/// - `RotationOrder` (enum): order of `Lcl Rotation` Euler angles.
///   0 = XYZ (X applied first: `R = Rz * Ry * Rx`), 1 = XZY, 2 = YZX, 3 = YXZ,
///   4 = ZXY, 5 = ZYX, 6 = spheric XYZ.
/// - `InheritType` (enum): how the parent's scale applies to children.
///   0 = RrSs, 1 = RSrs (parent scale affects child translation, the usual
///   behavior, written by Blender), 2 = Rrs (no parent scale).
/// - `DefaultAttributeIndex`: which connected `NodeAttribute` is the main
///   one (0 = first).
/// - `Lcl Translation` / `Lcl Rotation` / `Lcl Scaling`: local transform
///   relative to the parent (translation in file units, rotation in degrees).
///
/// The full local transform an importer computes is
/// `T * Roff * Rp * Rpre * R * Rpost^-1 * Rp^-1 * Soff * Sp * S * Sp^-1`;
/// the pivot/offset/pre/post properties are left at their identity defaults,
/// so it reduces to `T * R * S`, which is exactly Havok's `QsTransform`.
pub(super) fn model_properties(t: [f64; 3], r: [f64; 3], s: [f64; 3]) -> Properties70<'static> {
    Properties70::new()
        .enumeration("RotationOrder", 0) // eEulerXYZ
        .enumeration("InheritType", 1) // eInheritRSrs
        .int("DefaultAttributeIndex", 0)
        .lcl("Lcl Translation", t)
        .lcl("Lcl Rotation", r)
        .lcl("Lcl Scaling", s)
}
