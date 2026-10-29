//! NIF shapes as FBX meshes.
//!
//! Follows PyNifly's import: a shape is placed at its node's world transform,
//! and each bone's bind pose is `shape world * inverse(skin_to_bone)`.

use serde_nif::{Scene, Shape, Skin, Transform};

use crate::Mat4;
use crate::ser::{Cluster, Mesh, Texture, TextureSlot};

/// Converts every shape of a NIF scene into a [`Mesh`].
///
/// Unskinned shapes get a single cluster binding them rigidly to their parent
/// node, so they follow that bone when it exists in the skeleton.
///
/// `load_texture` receives a normalized texture path
/// (`textures/actors/cow/highlandcow.dds`) and returns the file bytes to
/// embed, or `None` to only reference the path.
pub fn meshes_from_nif(scene: &Scene, load_texture: impl Fn(&str) -> Option<Vec<u8>>) -> Vec<Mesh> {
    scene
        .shapes
        .iter()
        .map(|shape| {
            let world = to_mat4(&scene.world_transform(shape.node));
            let clusters = shape.skin.as_ref().map_or_else(
                || rigid_cluster(scene, shape),
                |skin| skin_clusters(skin, world),
            );

            Mesh {
                name: shape.name.clone(),
                transform: world,
                vertices: shape.vertices.clone(),
                normals: shape.normals.clone(),
                // DirectX UV origin (top-left) -> FBX (bottom-left).
                uvs: shape.uvs.iter().map(|&[u, v]| [u, 1.0 - v]).collect(),
                triangles: shape.triangles.iter().map(|t| t.map(u32::from)).collect(),
                clusters,
                textures: [TextureSlot::Diffuse, TextureSlot::Normal]
                    .into_iter()
                    .zip(&shape.textures)
                    .filter(|(_, path)| !path.is_empty())
                    .map(|(slot, path)| {
                        let path = normalize_texture_path(path);
                        Texture {
                            slot,
                            content: load_texture(&path),
                            path,
                        }
                    })
                    .collect(),
            }
        })
        .collect()
}

/// One cluster per weighted bone; bind = `shape world * inverse(skin_to_bone)`.
fn skin_clusters(skin: &Skin, world: Mat4) -> Vec<Cluster> {
    skin.bones
        .iter()
        .enumerate()
        .map(|(bone, name)| {
            let skin_to_bone = skin.skin_to_bone.get(bone).map(to_mat4).unwrap_or_default();
            let (indices, weights) = skin
                .weights
                .iter()
                .enumerate()
                .flat_map(|(vertex, influences)| {
                    influences
                        .iter()
                        .filter(move |(b, _)| usize::from(*b) == bone)
                        .map(move |(_, w)| (vertex as i32, f64::from(*w)))
                })
                .unzip();
            Cluster {
                bone: name.clone(),
                bind: world * skin_to_bone.inverse(),
                indices,
                weights,
            }
        })
        .filter(|cluster| !cluster.indices.is_empty())
        .collect()
}

/// Binds every vertex of an unskinned shape to its parent node.
fn rigid_cluster(scene: &Scene, shape: &Shape) -> Vec<Cluster> {
    let parent = scene.nodes.get(shape.node).and_then(|node| node.parent);
    let Some((parent, node)) = parent.and_then(|p| Some((p, scene.nodes.get(p)?))) else {
        return Vec::new();
    };
    vec![Cluster {
        bone: node.name.clone(),
        bind: to_mat4(&scene.world_transform(parent)),
        indices: (0..shape.vertices.len() as i32).collect(),
        weights: vec![1.0; shape.vertices.len()],
    }]
}

/// `Textures\Actors\Cow\a.dds` or `actors\cow\a.dds` -> `textures/actors/cow/a.dds`.
pub fn normalize_texture_path(path: &str) -> String {
    let path = path.replace('\\', "/").trim_start_matches('/').to_lowercase();
    if path.starts_with("textures/") {
        path
    } else {
        format!("textures/{path}")
    }
}

fn to_mat4(t: &Transform) -> Mat4 {
    let r = t.rotation.map(|row| row.map(f64::from));
    Mat4::translation(t.translation.map(f64::from))
        * Mat4::from_rotation(r)
        * Mat4::scale([f64::from(t.scale); 3])
}
