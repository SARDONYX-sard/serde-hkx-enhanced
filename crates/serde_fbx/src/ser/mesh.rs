//! Skinned meshes in the exported FBX scene.
//!
//! ```text
//! Model(Mesh) <- Geometry(Mesh) <- Deformer(Skin) <- SubDeformer(Cluster) <- Model(LimbNode)
//! ```
//!
//! A cluster stores, as Blender's exporter does:
//! - `TransformLink`: bone world matrix at bind time;
//! - `Transform`: `inverse(TransformLink) * mesh world matrix`.

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
