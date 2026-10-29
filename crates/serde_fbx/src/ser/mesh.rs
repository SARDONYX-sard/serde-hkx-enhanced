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

use super::node::{Node, Properties70, Property, object_name};
use super::{SceneBuilder, model_properties, quat_to_euler};
use crate::math::Mat4;

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
    const fn property(self) -> &'static str {
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

impl SceneBuilder {
    /// Adds meshes skinned to the skeleton `models` (`bone name -> model id`).
    ///
    /// `rest_world`: world matrix of every skeleton model at rest, used for
    /// bind pose entries of bones without a cluster.
    ///
    /// Returns the names of cluster bones missing from the skeleton.
    pub(super) fn add_meshes(
        &mut self,
        meshes: &[Mesh],
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
            let r = quat_to_euler(q);
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
                        .value("Indexes", Property::I32Array(cluster.indices.clone()))
                        .value("Weights", Property::F64Array(cluster.weights.clone()))
                        .value("Transform", Property::F64Array(transform.to_fbx_array()))
                        .value("TransformLink", Property::F64Array(cluster.bind.to_fbx_array())),
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

    /// `Material <- Texture <- Video`, connected to `model`.
    ///
    /// ```text
    /// Material: <id>, "Material::<mesh>", "" {
    ///     Version: 102
    ///     ShadingModel: "phong"                ; "lambert" or "phong"
    ///     Properties70: {
    ///         P: "DiffuseColor", "Color", "", "A", 1, 1, 1    ; multiplied with the diffuse texture
    ///         P: "DiffuseFactor", "Number", "", "A", 1
    ///     }
    /// }
    /// Texture: <id>, "Texture::highlandcow.dds", "" {
    ///     Type: "TextureVideoClip"   Version: 202
    ///     TextureName: "Texture::highlandcow.dds"
    ///     Properties70: { P: "UVSet", "KString", "", "", "UVMap" }  ; LayerElementUV name to use
    ///     Media: "Video::highlandcow.dds"      ; name of the Video object
    ///     FileName / RelativeFilename: "textures/actors/cow/highlandcow.dds"
    ///     ModelUVTranslation: 0, 0   ModelUVScaling: 1, 1   ; UV offset / tiling
    ///     Texture_Alpha_Source: "None"   Cropping: 0, 0, 0, 0
    /// }
    /// Video: <id>, "Video::highlandcow.dds", "Clip" {
    ///     Type: "Clip"
    ///     Properties70: { P: "Path", "KString", "XRefUrl", "", "<path>" }
    ///     Filename / RelativeFilename: "<path>"
    ///     Content: <raw file bytes>            ; optional embedded image
    /// }
    /// C: "OO", <material>, <mesh model>
    /// C: "OP", <texture>, <material>, "DiffuseColor"   ; or "NormalMap"
    /// C: "OO", <video>, <texture>
    /// ```
    ///
    /// `RelativeFilename` is relative to the FBX file and is what Blender
    /// tries first; `FileName` is the fallback (usually absolute).
    fn add_material(&mut self, model: i64, mesh: &Mesh, videos: &mut HashMap<String, i64>) {
        let material = self.new_id();
        self.objects.push(
            Node::new("Material")
                .prop(material)
                .prop(object_name(&mesh.name, "Material"))
                .prop("")
                .value("Version", 102)
                .value("ShadingModel", "phong")
                .value("MultiLayer", 0)
                .child(
                    Properties70::new()
                        .string("ShadingModel", "phong")
                        .entry("DiffuseColor", "Color", "", "A", [1.0, 1.0, 1.0].map(Property::F64))
                        .entry("DiffuseFactor", "Number", "", "A", [Property::F64(1.0)])
                        .build(),
                ),
        );
        self.materials += 1;
        self.connect(material, model);

        for texture in &mesh.textures {
            let name = texture
                .path
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&texture.path)
                .to_owned();

            let id = self.new_id();
            self.objects.push(
                Node::new("Texture")
                    .prop(id)
                    .prop(object_name(&name, "Texture"))
                    .prop("")
                    .value("Type", "TextureVideoClip")
                    .value("Version", 202)
                    .value("TextureName", object_name(&name, "Texture"))
                    .child(
                        Properties70::new()
                            .string("UVSet", "UVMap")
                            .entry("UseMaterial", "bool", "", "", [Property::I32(1)])
                            .build(),
                    )
                    .value("Media", object_name(&name, "Video"))
                    .value("FileName", texture.path.as_str())
                    .value("RelativeFilename", texture.path.as_str())
                    .child(Node::new("ModelUVTranslation").prop(0.0).prop(0.0))
                    .child(Node::new("ModelUVScaling").prop(1.0).prop(1.0))
                    .value("Texture_Alpha_Source", "None")
                    .child(Node::new("Cropping").prop(0i32).prop(0i32).prop(0i32).prop(0i32)),
            );
            self.textures += 1;
            self.connect_prop(id, material, texture.slot.property());

            let video = match videos.get(&texture.path) {
                Some(&video) => video,
                None => {
                    let video = self.new_id();
                    let mut node = Node::new("Video")
                        .prop(video)
                        .prop(object_name(&name, "Video"))
                        .prop("Clip")
                        .value("Type", "Clip")
                        .child(
                            Properties70::new()
                                .entry("Path", "KString", "XRefUrl", "", [Property::from(texture.path.as_str())])
                                .build(),
                        )
                        .value("UseMipMap", 0)
                        .value("Filename", texture.path.as_str())
                        .value("RelativeFilename", texture.path.as_str());
                    if let Some(content) = &texture.content {
                        node.push(Node::new("Content").prop(Property::Raw(content.clone())));
                    }
                    self.objects.push(node);
                    self.videos += 1;
                    videos.insert(texture.path.clone(), video);
                    video
                }
            };
            self.connect(video, id);
        }
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
                    .value("Matrix", Property::F64Array(matrix.to_fbx_array())),
            );
        }
        self.objects.push(pose);
        self.poses += 1;
    }
}

/// `Geometry` of type `Mesh`: the polygon data.
///
/// ```text
/// Geometry: <id>, "Geometry::<mesh>", "Mesh" {
///     GeometryVersion: 124
///     Vertices: *9 { a: x0,y0,z0, x1,y1,z1, x2,y2,z2 }   ; f64, mesh-local space
///     PolygonVertexIndex: *3 { a: 0, 1, -3 }           ; see below
///     LayerElementMaterial: 0 { ... }   ; material per polygon
///     LayerElementNormal: 0 { ... }     ; normals
///     LayerElementUV: 0 { ... }         ; texture coordinates
///     Layer: 0 {                        ; which elements form layer 0
///         Version: 100
///         LayerElement: { Type: "LayerElementNormal"  TypedIndex: 0 }
///         ...
///     }
/// }
/// ```
///
/// `PolygonVertexIndex` lists the vertex indices of every polygon in order;
/// the last index of a polygon is stored as `-(index + 1)` (bitwise NOT) to
/// mark its end. Triangle `(0, 1, 2)` is `0, 1, -3`.
///
/// Every `LayerElement*` has the same shape:
///
/// - `MappingInformationType`: what each value belongs to
///   - `"ByVertice"` (sic): one per vertex (`Vertices` order)
///   - `"ByPolygonVertex"`: one per entry of `PolygonVertexIndex` (corner)
///   - `"ByPolygon"`: one per polygon
///   - `"AllSame"`: one value for the whole mesh
/// - `ReferenceInformationType`:
///   - `"Direct"`: the values are listed in mapping order
///   - `"IndexToDirect"`: values are listed once, and an `...Index` array
///     maps each mapping slot to a value
fn geometry_node(id: i64, mesh: &Mesh) -> Node {
    let vertices = mesh
        .vertices
        .iter()
        .flat_map(|v| v.map(f64::from))
        .collect();
    // The last index of each polygon is stored as `-(index + 1)`.
    let polygon_indices: Vec<i32> = mesh
        .triangles
        .iter()
        .flat_map(|&[a, b, c]| [a as i32, b as i32, -(c as i32) - 1])
        .collect();

    let mut geometry = Node::new("Geometry")
        .prop(id)
        .prop(object_name(&mesh.name, "Geometry"))
        .prop("Mesh")
        .child(Properties70::new().build())
        .value("GeometryVersion", 124)
        .value("Vertices", Property::F64Array(vertices))
        .value("PolygonVertexIndex", Property::I32Array(polygon_indices));

    let mut layer = Node::new("Layer").prop(0i32).value("Version", 100);

    // Every polygon uses material 0 (the first material connected to the model).
    if !mesh.textures.is_empty() {
        geometry.push(
            Node::new("LayerElementMaterial")
                .prop(0i32)
                .value("Version", 101)
                .value("Name", "")
                .value("MappingInformationType", "AllSame")
                .value("ReferenceInformationType", "IndexToDirect")
                .value("Materials", Property::I32Array(vec![0])),
        );
        layer.push(layer_element("LayerElementMaterial"));
    }

    // One normal (x, y, z) per vertex, in `Vertices` order.
    if mesh.normals.len() == mesh.vertices.len() && !mesh.normals.is_empty() {
        geometry.push(
            Node::new("LayerElementNormal")
                .prop(0i32)
                .value("Version", 101)
                .value("Name", "")
                .value("MappingInformationType", "ByVertice")
                .value("ReferenceInformationType", "Direct")
                .value(
                    "Normals",
                    Property::F64Array(mesh.normals.iter().flat_map(|n| n.map(f64::from)).collect()),
                ),
        );
        layer.push(layer_element("LayerElementNormal"));
    }

    // UVs (u, v), v pointing up (0 = bottom of the image). One UV per vertex in
    // `UV`, and `UVIndex` (one per polygon corner) points into it; the same
    // indices as `PolygonVertexIndex` without the end markers.
    // `Name` is the UV map name referenced by `Texture.UVSet`.
    if mesh.uvs.len() == mesh.vertices.len() && !mesh.uvs.is_empty() {
        let uv_index = mesh
            .triangles
            .iter()
            .flat_map(|t| t.map(|i| i as i32))
            .collect();
        geometry.push(
            Node::new("LayerElementUV")
                .prop(0i32)
                .value("Version", 101)
                .value("Name", "UVMap")
                .value("MappingInformationType", "ByPolygonVertex")
                .value("ReferenceInformationType", "IndexToDirect")
                .value(
                    "UV",
                    Property::F64Array(mesh.uvs.iter().flat_map(|uv| uv.map(f64::from)).collect()),
                )
                .value("UVIndex", Property::I32Array(uv_index)),
        );
        layer.push(layer_element("LayerElementUV"));
    }

    geometry.child(layer)
}

fn layer_element(element: &str) -> Node {
    Node::new("LayerElement")
        .value("Type", element)
        .value("TypedIndex", 0)
}
