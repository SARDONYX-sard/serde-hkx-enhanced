//! Skinned mesh scenes: the node hierarchy, shapes and their skinning.
//!
//! Strings (node names, texture paths) borrow the file bytes, so a [`Scene`]
//! cannot outlive the buffer it was read from.

mod blocks;
mod build;
mod geometry;
mod skin;

use std::borrow::Cow;
use std::collections::HashSet;

use self::blocks::read_block;
use self::build::SceneBuilder;
use crate::error::{Error, UnsupportedVersionSnafu};
use crate::header::{Header, read_roots};
use crate::reader::Reader;
use crate::transform::Transform;

/// A scene graph node (`NiNode` and subclasses, and shapes themselves).
#[derive(Debug, Clone)]
pub struct Node<'a> {
    pub name: Cow<'a, str>,
    /// Index into [`Scene::nodes`].
    pub parent: Option<usize>,
    /// Local transform relative to the parent.
    pub transform: Transform,
}

/// Skinning of a shape.
///
/// Vertices are stored in "skin space" (usually the shape's own space). At
/// runtime a vertex is moved by each influencing bone:
///
/// ```text
/// v' = Σ weight_b * bone_world_b * skin_to_bone_b * v
/// ```
///
/// At bind time `bone_world_b * skin_to_bone_b` is the skin-to-world matrix,
/// so the bone's bind pose in skin space is `inverse(skin_to_bone_b)`.
#[derive(Debug, Clone, Default)]
pub struct Skin<'a> {
    /// Bone node names, indexed by the bone indices in `weights`.
    pub bones: Vec<Cow<'a, str>>,
    /// `NiSkinData` overall transform (global -> skin space). Usually the
    /// inverse of the shape's world transform (identity for most meshes).
    pub global_to_skin: Transform,
    /// Per bone: skin space -> bone space at bind time.
    pub skin_to_bone: Vec<Transform>,
    /// Per vertex: `(bone index, weight)` with non-zero weights.
    pub weights: Vec<Vec<(u16, f32)>>,
}

#[derive(Debug, Clone, Default)]
pub struct Shape<'a> {
    pub name: Cow<'a, str>,
    /// Index of this shape's own entry in [`Scene::nodes`].
    pub node: usize,
    pub vertices: Vec<[f32; 3]>,
    /// Empty if the shape has no normals.
    pub normals: Vec<[f32; 3]>,
    /// Empty if the shape has no UVs.
    pub uvs: Vec<[f32; 2]>,
    pub triangles: Vec<[u16; 3]>,
    pub skin: Option<Skin<'a>>,
    /// `BSShaderTextureSet` slots; empty strings for unused slots. Skyrim:
    ///
    /// | slot | texture | example |
    /// |---|---|---|
    /// | 0 | diffuse (color, alpha = transparency) | `bear.dds` |
    /// | 1 | normal map (alpha = specular) | `bear_n.dds`, `malebody_1_msn.dds` |
    /// | 2 | glow / skin tint / subsurface | `bear_sk.dds` |
    /// | 3 | height (parallax) | |
    /// | 4 | environment cube map | `cubemaps\shinybright_e.dds` |
    /// | 5 | environment mask | `bear_m.dds` |
    /// | 6 | multilayer inner layer | |
    /// | 7 | back light / specular (SE) | |
    pub textures: Vec<Cow<'a, str>>,
}

#[derive(Debug, Clone, Default)]
pub struct Scene<'a> {
    pub nodes: Vec<Node<'a>>,
    pub shapes: Vec<Shape<'a>>,
}

impl Scene<'_> {
    /// Transform from `node` space to the scene root, composed parent-first.
    pub fn world_transform(&self, node: usize) -> Transform {
        let mut chain = Vec::new();
        let mut current = Some(node);
        while let Some(node) = current.and_then(|index| self.nodes.get(index)) {
            chain.push(node.transform);
            current = node.parent;
        }
        chain
            .iter()
            .rev()
            .fold(Transform::default(), |parent, local| parent.compose(local))
    }
}

/// Reads a NIF mesh file.
///
/// # Errors
/// If the file is not Skyrim LE/SE (20.2.0.7, user version 12) or a decoded
/// block is malformed.
pub fn from_bytes(bytes: &[u8]) -> Result<Scene<'_>, Error> {
    let mut reader = Reader::new(bytes);
    let header = Header::read(&mut reader)?;
    // Block layouts below are Skyrim's; other games share the version number.
    if header.user_version != 12 {
        return UnsupportedVersionSnafu {
            version: crate::header::VERSION,
            user_version: header.user_version,
            bs_version: header.bs_version,
            expected: "a Skyrim LE/SE mesh (user version 12)",
        }
        .fail();
    }

    let blocks = header.read_blocks(&mut reader, |r, block_type| {
        read_block(r, block_type, &header)
    })?;
    let mut builder = SceneBuilder {
        blocks: &blocks,
        strings: &header.strings,
        scene: Scene::default(),
        visited: HashSet::new(),
    };
    for root in read_roots(&mut reader) {
        builder.visit(root, None)?;
    }
    Ok(builder.scene)
}
