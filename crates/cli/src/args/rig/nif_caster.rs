use serde_fbx::ser::nif_compat::{
    Material, Matrix3, Matrix4, Mesh, Node, Scene, Skin, Texture, Vector3,
};

use niflib_animation::ffi;

/// Converts a NIFLib scene into the pure Rust NIF scene representation.
///
/// This function owns the boundary between the NIFLib FFI representation and
/// the application-level scene representation.
pub(super) fn cast(source: ffi::NifScene) -> Scene {
    Scene {
        nodes: source.nodes.into_iter().map(convert_node).collect(),
        meshes: source.meshes.into_iter().map(convert_mesh).collect(),
        skins: source.skins.into_iter().map(convert_skin).collect(),
        materials: source.materials.into_iter().map(convert_material).collect(),
        textures: source.textures.into_iter().map(convert_texture).collect(),
    }
}

fn convert_node(source: ffi::NifNode) -> Node {
    Node {
        name: source.name,
        parent: source.parent,
        translation: convert_vector3(&source.translation),
        rotation: convert_matrix3(&source.rotation),
        scale: source.scale,
    }
}

fn convert_mesh(source: ffi::NifMesh) -> Mesh {
    Mesh {
        name: source.name,
        node: source.node,
        positions: source.positions,
        normals: source.normals,
        uvs: source.uvs,
        tangents: source.tangents,
        indices: source.indices,
        material: source.material,
        skin: source.skin,
    }
}

fn convert_skin(source: ffi::NifSkin) -> Skin {
    Skin {
        bones: source.bones,
        bone_indices: source.bone_indices,
        bone_weights: source.bone_weights,
        bind_matrices: source.bind_matrices.iter().map(convert_matrix4).collect(),
    }
}

fn convert_material(source: ffi::NifMaterial) -> Material {
    Material {
        name: source.name,
        texture: source.texture,
    }
}

fn convert_texture(source: ffi::NifTexture) -> Texture {
    Texture {
        diffuse: source.diffuse,
        normal: source.normal,
        glow: source.glow,
        specular: source.specular,
    }
}

const fn convert_vector3(source: &ffi::Vector3) -> Vector3 {
    Vector3 {
        x: source.x,
        y: source.y,
        z: source.z,
    }
}

const fn convert_matrix3(source: &ffi::Matrix3) -> Matrix3 {
    Matrix3 {
        x: convert_vector3(&source.x),
        y: convert_vector3(&source.y),
        z: convert_vector3(&source.z),
    }
}

const fn convert_matrix4(source: &ffi::Matrix3) -> Matrix4 {
    Matrix4 {
        m: [
            source.x.x, source.x.y, source.x.z, 0.0, source.y.x, source.y.y, source.y.z, 0.0,
            source.z.x, source.z.y, source.z.z, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
    }
}
