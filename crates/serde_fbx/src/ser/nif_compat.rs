//! Rust-owned NIF animation scene data.
//!
//! This crate contains no C++ or NIFLib dependencies.
//! It is the interchange model between NIF loading and FBX export.

/// A complete NIF scene.
#[derive(Debug, Clone)]
pub struct Scene {
    /// Scene nodes.
    pub nodes: Vec<Node>,

    /// Scene meshes.
    pub meshes: Vec<Mesh>,

    /// Scene skins.
    pub skins: Vec<Skin>,

    /// Scene materials.
    pub materials: Vec<Material>,

    /// Scene textures.
    pub textures: Vec<Texture>,
}

/// A NIF scene node.
#[derive(Debug, Clone)]
pub struct Node {
    /// Node name.
    pub name: String,

    /// Parent node index.
    ///
    /// A negative value means that the node has no parent.
    pub parent: i32,

    /// Local translation.
    pub translation: Vector3,

    /// Local rotation.
    pub rotation: Matrix3,

    /// Uniform local scale.
    pub scale: f32,
}

/// A mesh.
#[derive(Debug, Clone)]
pub struct Mesh {
    /// Mesh name.
    pub name: String,

    /// NIF node index containing this mesh.
    pub node: i32,

    /// Vertex positions.
    ///
    /// Values are stored as XYZ triplets.
    pub positions: Vec<f32>,

    /// Vertex normals.
    ///
    /// Values are stored as XYZ triplets.
    pub normals: Vec<f32>,

    /// Texture coordinates.
    ///
    /// Values are stored as UV pairs.
    pub uvs: Vec<f32>,

    /// Vertex tangents.
    ///
    /// Values are stored as XYZ triplets.
    pub tangents: Vec<f32>,

    /// Triangle indices.
    pub indices: Vec<u32>,

    /// Material index.
    ///
    /// A negative value means that no material is assigned.
    pub material: i32,

    /// Skin index.
    ///
    /// A negative value means that the mesh is not skinned.
    pub skin: i32,
}

/// Skinning information.
#[derive(Debug, Clone)]
pub struct Skin {
    /// Bone names.
    pub bones: Vec<String>,

    /// Four bone indices per vertex.
    pub bone_indices: Vec<u16>,

    /// Four bone weights per vertex.
    pub bone_weights: Vec<f32>,

    /// Bind matrices corresponding to [`Skin::bones`].
    pub bind_matrices: Vec<Matrix4>,
}

/// Material information.
#[derive(Debug, Clone)]
pub struct Material {
    /// Material name.
    pub name: String,

    /// Texture index.
    ///
    /// A negative value means that no texture is assigned.
    pub texture: i32,
}

/// Texture information.
#[derive(Debug, Clone)]
pub struct Texture {
    /// Diffuse texture path.
    pub diffuse: String,

    /// Normal texture path.
    pub normal: String,

    /// Glow texture path.
    pub glow: String,

    /// Specular texture path.
    pub specular: String,
}

/// Three-dimensional vector.
#[derive(Debug, Clone, Copy, Default)]
pub struct Vector3 {
    /// X component.
    pub x: f32,

    /// Y component.
    pub y: f32,

    /// Z component.
    pub z: f32,
}

/// Three-dimensional rotation matrix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Matrix3 {
    /// X basis vector.
    pub x: Vector3,

    /// Y basis vector.
    pub y: Vector3,

    /// Z basis vector.
    pub z: Vector3,
}

/// Four-dimensional transformation matrix.
#[derive(Debug, Clone, Copy)]
pub struct Matrix4 {
    /// Matrix elements in row-major order.
    pub m: [f32; 16],
}

impl Default for Matrix4 {
    fn default() -> Self {
        let mut m = [0.0; 16];
        m[0] = 1.0;
        m[5] = 1.0;
        m[10] = 1.0;
        m[15] = 1.0;

        Self { m }
    }
}
