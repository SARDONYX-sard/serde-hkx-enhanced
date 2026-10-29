//! Skinned meshes in the exported FBX scene.
//!
//! Objects of one skinned, textured mesh and their connections
//! (`A <- B` = `C: "OO", B, A`, i.e. B is connected to A):
//!
//! ```text
//! scene root (0)
//! └─ Model "Model::<mesh>", "Mesh"            placement of the mesh (Lcl T/R/S = world transform)
//!    ├─ Geometry "Geometry::<mesh>", "Mesh"   vertices, triangles, normals, UVs
//!    │  └─ Deformer "Deformer::<mesh>", "Skin"            one per skinned geometry
//!    │     └─ Deformer "SubDeformer::<bone>", "Cluster"   one per influencing bone:
//!    │        │                                           vertex indices + weights
//!    │        └─ Model "<bone>", "LimbNode"               (C: "OO", bone, cluster)
//!    └─ Material "Material::<mesh>"
//!       └─ Texture "Texture::<file>"  (C: "OP", texture, material, "DiffuseColor" | "NormalMap")
//!          └─ Video "Video::<file>", "Clip"   the image file (path, optional embedded bytes)
//! Pose "Pose::BindPose", "BindPose"           world matrix of the mesh and every bone at bind time
//! ```
//!
//! A cluster stores, as Blender's exporter does:
//! - `TransformLink`: bone world matrix at bind time;
//! - `Transform`: `inverse(TransformLink) * mesh world matrix`.
//!
//! Matrices are written as 16 `f64` in column-major order (translation in
//! elements 12..15), see [`Mat4::to_fbx_array`].

use std::collections::HashMap;

use super::geometry::geometry_node;
use super::node::{Node, Property, object_name};
use rig_math::{Mat4, quat_to_euler_xyz};

use super::scene::{SceneBuilder, model_properties};

/// A triangle mesh to place in the FBX scene.
#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub name: String,
    /// World transform of the mesh node, also used as its bind transform.
    pub transform: Mat4,
    pub vertices: Vec<[f32; 3]>,
    /// Per-vertex normals; empty when unknown.
    pub normals: Vec<[f32; 3]>,
    /// Per-vertex UVs in FBX convention (`v` up); empty when unknown.
    pub uvs: Vec<[f32; 2]>,
    pub triangles: Vec<[u32; 3]>,
    /// Skin clusters; empty for a rigid mesh.
    pub clusters: Vec<Cluster>,
    /// Textures of the mesh's material; no material is written when empty.
    pub textures: Vec<Texture>,
}

/// Material channel a texture is connected to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureSlot {
    Diffuse,
    Normal,
}

impl TextureSlot {
    /// `Material` property the texture connects to.
    pub(super) const fn property(self) -> &'static str {
        match self {
            Self::Diffuse => "DiffuseColor",
            Self::Normal => "NormalMap",
        }
    }
}

/// An image used by a mesh's material.
#[derive(Debug, Clone)]
pub struct Texture {
    pub slot: TextureSlot,
    /// Path written to the FBX (e.g. `textures/actors/cow/highlandcow.dds`).
    pub path: String,
    /// Image file bytes embedded in the FBX (`Video.Content`), so the FBX
    /// works without the texture files next to it.
    pub content: Option<Vec<u8>>,
}

/// Vertex weights for one bone.
#[derive(Debug, Clone, Default)]
pub struct Cluster {
    /// Bone name (matched case-insensitively against the skeleton).
    pub bone: String,
    /// Bone world transform at bind time.
    pub bind: Mat4,
    pub indices: Vec<i32>,
    pub weights: Vec<f64>,
}

impl<'a> SceneBuilder<'a> {
    /// Adds meshes skinned to the skeleton `models` (`bone name -> model id`).
    ///
    /// `rest_world`: world matrix of every skeleton model at rest, used for
    /// bind pose entries of bones without a cluster.
    ///
    /// Returns the names of cluster bones missing from the skeleton.
    pub(super) fn add_meshes(
        &mut self,
        meshes: &'a [Mesh],
        models: &HashMap<String, i64>,
        rest_world: &HashMap<i64, Mat4>,
    ) -> Vec<String> {
        let mut missing = Vec::new();
        let mut bind_pose: HashMap<i64, Mat4> = rest_world.clone();
        // Path -> `Video` id, so an image shared by several meshes is embedded once.
        let mut videos: HashMap<String, i64> = HashMap::new();

        for mesh in meshes {
            // Mesh model: like a bone's Model but with subclass "Mesh"; its
            // Lcl T/R/S place the geometry in the scene.
            let model = self.new_id();
            let geometry = self.new_id();

            let (t, q, s) = mesh.transform.decompose();
            let r = quat_to_euler_xyz(q);
            self.objects.push(
                Node::new("Model")
                    .prop(model)
                    .prop(object_name(&mesh.name, "Model"))
                    .prop("Mesh")
                    .value("Version", 232)
                    .child(model_properties(t, r, s).build())
                    .value("MultiLayer", 0)
                    .value("MultiTake", 0)
                    .value("Shading", Property::Bool(true))
                    .value("Culling", "CullingOff"),
            );
            self.objects.push(geometry_node(geometry, mesh));
            self.models += 1;
            self.geometries += 1;
            self.connect(model, 0);
            self.connect(geometry, model);
            bind_pose.insert(model, mesh.transform);
            if !mesh.textures.is_empty() {
                self.add_material(model, mesh, &mut videos);
            }

            if mesh.clusters.is_empty() {
                continue;
            }

            // Skin deformer: container of the clusters of one geometry.
            //   Deformer: <id>, "Deformer::<mesh>", "Skin" {
            //       Version: 101
            //       Link_DeformAcuracy: 50     ; FBX SDK default (sic, "Acuracy"), unused by importers
            //   }
            let skin = self.new_id();
            self.objects.push(
                Node::new("Deformer")
                    .prop(skin)
                    .prop(object_name(&mesh.name, "Deformer"))
                    .prop("Skin")
                    .value("Version", 101)
                    .value("Link_DeformAcuracy", 50.0),
            );
            self.deformers += 1;
            self.connect(skin, geometry);

            for cluster in &mesh.clusters {
                let Some(&bone) = models.get(&cluster.bone.to_lowercase()) else {
                    missing.push(cluster.bone.clone());
                    continue;
                };

                // Cluster: the vertices one bone moves.
                //   Deformer: <id>, "SubDeformer::<bone>", "Cluster" {
                //       Version: 100
                //       UserData: "", ""
                //       Indexes: *N { a: 0, 5, 6, ... }        ; vertex indices into Geometry.Vertices
                //       Weights: *N { a: 1, 0.5, 0.25, ... }   ; weight per index (0..=1)
                //       Transform: *16 { ... }                 ; inverse(TransformLink) * mesh world
                //       TransformLink: *16 { ... }             ; bone world matrix at bind time
                //   }
                // A vertex's weights over all clusters should sum to 1.
                let id = self.new_id();
                let transform = cluster.bind.inverse() * mesh.transform;
                self.objects.push(
                    Node::new("Deformer")
                        .prop(id)
                        .prop(object_name(&cluster.bone, "SubDeformer"))
                        .prop("Cluster")
                        .value("Version", 100)
                        .child(Node::new("UserData").prop("").prop(""))
                        .value("Indexes", cluster.indices.as_slice())
                        .value("Weights", cluster.weights.as_slice())
                        .value("Transform", transform.to_fbx_array())
                        .value("TransformLink", cluster.bind.to_fbx_array()),
                );
                self.deformers += 1;
                self.connect(id, skin);
                self.connect(bone, id);
                bind_pose.insert(bone, cluster.bind);
            }
        }

        if !meshes.is_empty() {
            self.add_bind_pose(&bind_pose);
        }
        missing
    }

    /// `Pose` of type `BindPose`: world matrices at the time the mesh was bound.
    ///
    /// ```text
    /// Pose: <id>, "Pose::BindPose", "BindPose" {
    ///     Type: "BindPose"   Version: 100
    ///     NbPoseNodes: 45                       ; number of PoseNode children
    ///     PoseNode: { Node: <model id>  Matrix: *16 { ... } }   ; world matrix, column-major
    ///     ...
    /// }
    /// ```
    ///
    /// Importers use it as the rest pose of bones (Blender builds the armature
    /// from it). Bones with a cluster use the cluster's `TransformLink`, the
    /// others the Havok reference pose.
    fn add_bind_pose(&mut self, matrices: &HashMap<i64, Mat4>) {
        let id = self.new_id();
        let mut entries: Vec<_> = matrices.iter().collect();
        entries.sort_by_key(|(id, _)| **id);

        let mut pose = Node::new("Pose")
            .prop(id)
            .prop(object_name("BindPose", "Pose"))
            .prop("BindPose")
            .value("Type", "BindPose")
            .value("Version", 100)
            .value("NbPoseNodes", entries.len() as i32);
        for (&node, matrix) in entries {
            pose.push(
                Node::new("PoseNode")
                    .value("Node", node)
                    .value("Matrix", matrix.to_fbx_array()),
            );
        }
        self.objects.push(pose);
        self.poses += 1;
    }
}
