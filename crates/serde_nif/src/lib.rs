// SPDX-License-Identifier: GPL-3.0-only
//
// Reference (block layouts and skinning, nifly is the library behind PyNifly):
// - https://github.com/ousnius/nifly/blob/6cb15fb5da71527b33cea8ff7913be8790e1c633/src/Geometry.cpp
// - https://github.com/ousnius/nifly/blob/6cb15fb5da71527b33cea8ff7913be8790e1c633/src/Skin.cpp
// - https://github.com/ousnius/nifly/blob/6cb15fb5da71527b33cea8ff7913be8790e1c633/src/Objects.cpp
// - https://github.com/ousnius/nifly/blob/6cb15fb5da71527b33cea8ff7913be8790e1c633/src/Nodes.cpp
// - https://github.com/ousnius/nifly/blob/6cb15fb5da71527b33cea8ff7913be8790e1c633/src/NifFile.cpp
// - https://github.com/ousnius/nifly/blob/6cb15fb5da71527b33cea8ff7913be8790e1c633/include/VertexData.hpp
// - https://github.com/BadDogSkyrim/PyNifly/blob/ac3c1453a5efa44f580033205ecaf5be9a005eed/io_scene_nifly/blender_defs.py

//! Read Skyrim LE/SE NIF scenes: node hierarchy, triangle shapes and skinning.
//!
//! Only the blocks needed to rebuild a skinned mesh are decoded; every other
//! block is skipped using the block sizes stored in the header (20.2.0.5+).
//!
//! Block layouts follow nifly, the library behind PyNifly (see the references above).
//!
//! # NIF primer
//!
//! A NIF (NetImmerse/Gamebryo) file is a header followed by a flat list of
//! "blocks" (objects). Blocks refer to each other by **index** into that list.
//!
//! ```text
//! "Gamebryo File Format, Version 20.2.0.7\n"   header line
//! u32 version          0x14020007 (= 20.2.0.7, Skyrim LE and SE)
//! u8  endian           1 = little endian
//! u32 user version     12 (Skyrim)
//! u32 block count
//! u32 BS version       83 = Skyrim LE, 100 = Skyrim SE (Fallout 4: 130, ...)
//! short strings        author / process script / export script (u8 length incl. NUL)
//! u16 type count, then each type name   e.g. ["NiNode", "BSTriShape", "NiSkinInstance", ...]
//! u16 type index per block              block i is of type names[index[i]]
//! u32 size per block                    lets a reader skip blocks it does not know
//! u32 string count, u32 max length, then each string   e.g. ["Pelvis", "LFemur", ...]
//! u32 group count, groups (unused)
//! blocks...                             block 0, block 1, ...
//! u32 root count, root block indices    footer: entry points of the scene graph
//! ```
//!
//! Inside blocks:
//!
//! - a **ref** is an `i32` block index (`-1` = none);
//! - a **string** is an `i32` index into the header string table (`-1` = none);
//! - a transform is translation (3 `f32`), rotation (3x3 `f32`, row-major),
//!   uniform scale (`f32`).
//!
//! A skinned Skyrim mesh (blocks and the refs between them):
//!
//! ```text
//! BSFadeNode "HighlandCow.nif"               root node (NiNode subclass)
//! ├─ children → NiNode "Pelvis"              bones are plain nodes; their transforms
//! │             └─ NiNode "LFemur" ...       are the bind pose of the skeleton
//! └─ children → BSTriShape "highlandcow"     the mesh (SE; LE: NiTriShape + NiTriShapeData)
//!                ├─ skin   → NiSkinInstance          bones: refs to the bone NiNodes
//!                │           ├─ data      → NiSkinData       skin-to-bone transform per bone (+ weights on LE)
//!                │           └─ partition → NiSkinPartition  SE: vertex buffer with weights, triangles
//!                ├─ shader → BSLightingShaderProperty
//!                │           └─ texture set → BSShaderTextureSet  ["textures\\...\\a.dds", "..._n.dds", ...]
//!                └─ alpha  → NiAlphaProperty (not read)
//! ```
//!
//! Class inheritance decides the field order of a block:
//! `NiObjectNET` (name, extra data, controller) → `NiAVObject` (flags,
//! transform, collision) → `NiNode` (children) / `BSTriShape` (shape data).
//!
//! # Skinning
//!
//! - LE `NiTriShape`: vertices in `NiTriShapeData`, weights in `NiSkinData`.
//! - SE `BSTriShape`: when skinned, vertices, weights and triangles live in
//!   `NiSkinPartition` (global vertex buffer + `trueTriangles`). Vertex bone
//!   indices refer to `NiSkinInstance.bones`.
//!
//! A bone's bind pose in shape (skin) space is `inverse(skin_to_bone)`.
mod error;
mod reader;

use std::collections::HashSet;

pub use error::Error;
use error::{InvalidDataSnafu, InvalidHeaderSnafu, UnsupportedVersionSnafu};
use reader::Reader;

/// NIF `NiTransform`: `v' = rotation * (scale * v) + translation`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub translation: [f32; 3],
    /// Row-major.
    pub rotation: [[f32; 3]; 3],
    pub scale: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            scale: 1.0,
        }
    }
}

/// A scene graph node (`NiNode` and subclasses, and shapes themselves).
#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
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
pub struct Skin {
    /// Bone node names, indexed by the bone indices in `weights`.
    pub bones: Vec<String>,
    /// `NiSkinData` overall transform (global -> skin space). Usually the
    /// inverse of the shape's world transform (identity for most meshes).
    pub global_to_skin: Transform,
    /// Per bone: skin space -> bone space at bind time.
    pub skin_to_bone: Vec<Transform>,
    /// Per vertex: `(bone index, weight)` with non-zero weights.
    pub weights: Vec<Vec<(u16, f32)>>,
}

#[derive(Debug, Clone, Default)]
pub struct Shape {
    pub name: String,
    /// Index of this shape's own entry in [`Scene::nodes`].
    pub node: usize,
    pub vertices: Vec<[f32; 3]>,
    /// Empty if the shape has no normals.
    pub normals: Vec<[f32; 3]>,
    /// Empty if the shape has no UVs.
    pub uvs: Vec<[f32; 2]>,
    pub triangles: Vec<[u16; 3]>,
    pub skin: Option<Skin>,
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
    pub textures: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Scene {
    pub nodes: Vec<Node>,
    pub shapes: Vec<Shape>,
}

impl Scene {
    /// Transform from `node` space to the scene root, as `(rotation, translation, scale)`
    /// composed parent-first.
    pub fn world_transform(&self, node: usize) -> Transform {
        let mut chain = Vec::new();
        let mut current = Some(node);
        while let Some(index) = current {
            let Some(node) = self.nodes.get(index) else {
                break;
            };
            chain.push(node.transform);
            current = node.parent;
        }
        chain.iter().rev().fold(Transform::default(), |parent, local| parent.compose(local))
    }
}

impl Transform {
    /// Inverse transform (rotation is assumed orthonormal).
    pub fn inverse(&self) -> Self {
        let rt: [[f32; 3]; 3] = core::array::from_fn(|r| core::array::from_fn(|c| self.rotation[c][r]));
        let inv_scale = if self.scale.abs() > f32::EPSILON { 1.0 / self.scale } else { 0.0 };
        let t = mul33v(&rt, self.translation);
        Self {
            translation: t.map(|v| -v * inv_scale),
            rotation: rt,
            scale: inv_scale,
        }
    }

    /// `self * child` (apply `child` first).
    pub fn compose(&self, child: &Self) -> Self {
        let r = mul33(&self.rotation, &child.rotation);
        let t = mul33v(&self.rotation, child.translation);
        Self {
            translation: core::array::from_fn(|i| t[i].mul_add(self.scale, self.translation[i])),
            rotation: r,
            scale: self.scale * child.scale,
        }
    }
}

fn mul33(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    core::array::from_fn(|r| core::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}

fn mul33v(a: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|r| (0..3).map(|k| a[r][k] * v[k]).sum())
}

/// Reads a NIF file.
///
/// # Errors
/// If the file is not Skyrim LE/SE (20.2.0.7, user version 12) or a decoded
/// block is malformed.
pub fn from_bytes(bytes: &[u8]) -> Result<Scene, Error> {
    let mut reader = Reader::new(bytes);
    let header = Header::read(&mut reader)?;

    let mut blocks = Vec::with_capacity(header.block_types.len());
    for (index, (block_type, &size)) in header.block_types.iter().zip(&header.block_sizes).enumerate() {
        let start = reader.pos;
        let data = reader.take(size as usize)?;
        let block = read_block(&mut Reader::new(data), block_type, &header).map_err(|source| Error::Block {
            index,
            block_type: block_type.clone(),
            source: Box::new(source),
        })?;
        debug_assert_eq!(reader.pos, start + size as usize);
        blocks.push(block);
    }

    // Footer: root references.
    let roots = reader
        .index_list()
        .ok()
        .map(|roots| roots.into_iter().flatten().collect::<Vec<_>>())
        .filter(|roots| !roots.is_empty())
        .unwrap_or_else(|| vec![0]);

    let mut builder = SceneBuilder {
        blocks: &blocks,
        strings: &header.strings,
        scene: Scene::default(),
        visited: HashSet::new(),
    };
    for root in roots {
        builder.visit(root, None)?;
    }
    Ok(builder.scene)
}

// ---------------------------------------------------------------------------
// Header

/// The parts of the file header needed to read blocks (layout in the crate docs).
struct Header {
    /// 12 for Skyrim.
    user_version: u32,
    /// Bethesda stream version: 83 = Skyrim LE, 100 = Skyrim SE. Several block
    /// layouts depend on it (e.g. SE packs vertices into `BSTriShape`).
    bs_version: u32,
    /// Type name of every block, e.g. `["BSFadeNode", "NiNode", "BSTriShape", ...]`.
    block_types: Vec<String>,
    /// Byte size of every block.
    block_sizes: Vec<u32>,
    /// String table; blocks store `i32` indices into it (names of nodes, ...).
    strings: Vec<String>,
}

impl Header {
    fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
        // "Gamebryo File Format, Version 20.2.0.7\n"
        let mut line = Vec::new();
        loop {
            match reader.u8() {
                Ok(b'\n') => break,
                Ok(byte) if line.len() < 128 => line.push(byte),
                _ => return InvalidHeaderSnafu.fail(),
            }
        }
        if !(line.starts_with(b"Gamebryo File Format") || line.starts_with(b"NetImmerse File Format")) {
            return InvalidHeaderSnafu.fail();
        }

        let version = reader.u32()?;
        let _endian = reader.u8()?;
        let user_version = reader.u32()?;
        let num_blocks = reader.u32()? as usize;
        let bs_version = reader.u32()?;

        if version != 0x1402_0007 || user_version != 12 {
            return UnsupportedVersionSnafu {
                version,
                user_version,
                bs_version,
            }
            .fail();
        }

        let _author = reader.short_string()?;
        if bs_version > 130 {
            reader.u32()?;
        }
        if bs_version < 131 {
            let _process_script = reader.short_string()?;
        }
        let _export_script = reader.short_string()?;
        if bs_version >= 103 {
            let _max_filepath = reader.short_string()?;
        }

        let num_types = reader.u16()? as usize;
        let types = (0..num_types)
            .map(|_| reader.sized_string())
            .collect::<Result<Vec<_>, _>>()?;

        reader.check_len(num_blocks, 6)?;
        let block_types = (0..num_blocks)
            .map(|_| {
                let index = (reader.u16()? & 0x7fff) as usize;
                types.get(index).cloned().ok_or_else(|| {
                    InvalidDataSnafu {
                        message: format!("block type index {index} out of range"),
                    }
                    .build()
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let block_sizes = (0..num_blocks).map(|_| reader.u32()).collect::<Result<Vec<_>, _>>()?;

        let num_strings = reader.u32()? as usize;
        let _max_string_len = reader.u32()?;
        reader.check_len(num_strings, 4)?;
        let strings = (0..num_strings)
            .map(|_| reader.sized_string())
            .collect::<Result<Vec<_>, _>>()?;

        let num_groups = reader.u32()? as usize;
        reader.skip(num_groups.saturating_mul(4))?;

        Ok(Self {
            user_version,
            bs_version,
            block_types,
            block_sizes,
            strings,
        })
    }
}

// ---------------------------------------------------------------------------
// Blocks

#[derive(Debug, Clone, Copy, Default)]
struct Vertex {
    position: [f32; 3],
    uv: [f32; 2],
    normal: [f32; 3],
    weights: [f32; 4],
    bones: [u8; 4],
}

#[derive(Debug, Default)]
struct VertexLayout {
    has_uv: bool,
    has_normal: bool,
    has_tangent: bool,
    has_color: bool,
    skinned: bool,
    has_eye_data: bool,
}

#[derive(Debug)]
struct Partition {
    bones: Vec<u16>,
    vertex_map: Vec<u16>,
    /// `[vertex][weights_per_vertex]`
    weights: Vec<f32>,
    bone_indices: Vec<u8>,
    weights_per_vertex: usize,
    /// SE only: triangles indexing the global vertex buffer.
    true_triangles: Vec<[u16; 3]>,
}

#[derive(Debug)]
enum Block {
    Node {
        name: Option<usize>,
        transform: Transform,
        children: Vec<Option<usize>>,
    },
    /// SE `BSTriShape`.
    TriShape {
        name: Option<usize>,
        transform: Transform,
        skin: Option<usize>,
        shader: Option<usize>,
        layout: VertexLayout,
        vertices: Vec<Vertex>,
        triangles: Vec<[u16; 3]>,
        /// `BSDynamicTriShape` positions, overriding the packed ones.
        dynamic_positions: Vec<[f32; 3]>,
    },
    /// LE `NiTriShape` / `NiTriStrips`.
    Geometry {
        name: Option<usize>,
        transform: Transform,
        data: Option<usize>,
        skin: Option<usize>,
        shader: Option<usize>,
    },
    GeometryData {
        vertices: Vec<[f32; 3]>,
        normals: Vec<[f32; 3]>,
        uvs: Vec<[f32; 2]>,
        triangles: Vec<[u16; 3]>,
    },
    SkinInstance {
        data: Option<usize>,
        partition: Option<usize>,
        bones: Vec<Option<usize>>,
    },
    SkinData {
        global_to_skin: Transform,
        /// `(skin_to_bone, vertex weights)`
        bones: Vec<(Transform, Vec<(u16, f32)>)>,
        has_vertex_weights: bool,
    },
    SkinPartition {
        layout: VertexLayout,
        vertices: Vec<Vertex>,
        partitions: Vec<Partition>,
    },
    LightingShader {
        texture_set: Option<usize>,
    },
    TextureSet {
        textures: Vec<String>,
    },
    Other,
}

/// Every type sharing the `NiNode` layout prefix.
const NODE_TYPES: &[&str] = &[
    "NiNode",
    "BSFadeNode",
    "BSLeafAnimNode",
    "BSTreeNode",
    "BSMultiBoundNode",
    "BSOrderedNode",
    "BSValueNode",
    "BSDamageStage",
    "BSBlastNode",
    "BSRangeNode",
    "BSDebrisNode",
    "NiBillboardNode",
    "NiSwitchNode",
    "NiLODNode",
    "NiBone",
];

/// Decodes the blocks this crate needs; any other type becomes [`Block::Other`].
fn read_block(r: &mut Reader<'_>, block_type: &str, header: &Header) -> Result<Block, Error> {
    Ok(match block_type {
        // NiNode:
        //   NiAVObject fields (see `read_av_object`)
        //   u32 child count, i32 refs   children (nodes and shapes)
        //   u32 effect count, i32 refs  lights etc. (Skyrim: always 0, not read)
        t if NODE_TYPES.contains(&t) => {
            let (name, transform) = read_av_object(r)?;
            let children = r.index_list()?;
            Block::Node {
                name,
                transform,
                children,
            }
        }
        "BSTriShape" | "BSMeshLODTriShape" | "BSSubIndexTriShape" => read_bs_tri_shape(r, header, false)?,
        "BSDynamicTriShape" => read_bs_tri_shape(r, header, true)?,
        // NiTriShape / NiTriStrips (LE mesh), NiGeometry fields after NiAVObject:
        //   i32 ref   data           NiTriShapeData / NiTriStripsData (vertices, triangles)
        //   i32 ref   skin instance  NiSkinInstance / BSDismemberSkinInstance, -1 if rigid
        //   u32 material count, then count x (i32 name string) + count x (i32 extra data)
        //   i32       active material
        //   u8        "default material needs update" flag
        //   i32 ref   shader property  BSLightingShaderProperty
        //   i32 ref   alpha property   NiAlphaProperty
        "NiTriShape" | "NiTriStrips" => {
            let (name, transform) = read_av_object(r)?;
            let data = r.index()?;
            let skin = r.index()?;
            let num_materials = r.u32()? as usize;
            r.skip(num_materials.saturating_mul(8))?; // names + extra data
            r.i32()?; // active material
            r.u8()?; // default material needs update
            let shader = r.index()?;
            let _alpha = r.index()?;
            Block::Geometry {
                name,
                transform,
                data,
                skin,
                shader,
            }
        }
        "NiTriShapeData" | "NiTriStripsData" => read_geometry_data(r, block_type == "NiTriStripsData")?,
        // NiSkinInstance (BSDismemberSkinInstance adds body part partitions after it):
        //   i32 ref   skin data       NiSkinData
        //   i32 ref   skin partition  NiSkinPartition
        //   i32 ref   skeleton root   NiNode the bones belong to
        //   u32 bone count, i32 refs  bone NiNodes; vertex bone index i = bones[i]
        "NiSkinInstance" | "BSDismemberSkinInstance" => Block::SkinInstance {
            data: r.index()?,
            partition: r.index()?,
            bones: {
                let _skeleton_root = r.index()?;
                r.index_list()?
            },
        },
        "NiSkinData" => read_skin_data(r)?,
        "NiSkinPartition" => read_skin_partition(r, header)?,
        // BSLightingShaderProperty (Skyrim material):
        //   u32       shader type (0 = default, 1 = environment map, 5 = skin tint, ...)
        //   NiObjectNET fields
        //   u32 x2    shader flags 1 / 2 (bit sets: skinned, specular, ...)
        //   f32 x2    UV offset   f32 x2  UV scale
        //   i32 ref   texture set  BSShaderTextureSet
        //   ... colors, glossiness, alpha etc. (not read)
        "BSLightingShaderProperty" => {
            r.u32()?; // shader type
            read_object_net(r)?;
            r.skip(4 + 4 + 8 + 8)?; // flags1, flags2, uv offset, uv scale
            Block::LightingShader {
                texture_set: r.index()?,
            }
        }
        // BSShaderTextureSet:
        //   u32 count (Skyrim: 9), then count x (u32 length + bytes) texture paths
        //   relative to Data, e.g. "textures\actors\cow\highlandcow.dds" (see `Shape::textures`)
        "BSShaderTextureSet" => {
            let count = r.u32()? as usize;
            r.check_len(count, 4)?;
            Block::TextureSet {
                textures: (0..count).map(|_| r.sized_string()).collect::<Result<_, _>>()?,
            }
        }
        _ => Block::Other,
    })
}

/// `NiObjectNET`: the base of every named block.
///
/// ```text
/// i32 string  name                 e.g. "Pelvis", "highlandcow"
/// u32 count, i32 refs  extra data  NiStringExtraData, BSXFlags, ...
/// i32 ref     controller           animation controller (-1 for static)
/// ```
fn read_object_net(r: &mut Reader<'_>) -> Result<Option<usize>, Error> {
    let name = r.index()?;
    r.index_list()?; // extra data
    r.index()?; // controller
    Ok(name)
}

/// `f32 x3` translation, `f32 x9` rotation (row-major 3x3), `f32` scale.
fn read_transform_trs(r: &mut Reader<'_>) -> Result<Transform, Error> {
    Ok(Transform {
        translation: r.vec3()?,
        rotation: r.matrix33()?,
        scale: r.f32()?,
    })
}

/// `NiSkinData` stores rotation first.
fn read_transform_rts(r: &mut Reader<'_>) -> Result<Transform, Error> {
    let rotation = r.matrix33()?;
    let translation = r.vec3()?;
    Ok(Transform {
        translation,
        rotation,
        scale: r.f32()?,
    })
}

/// `NiObjectNET` + `NiAVObject`: every object placed in the scene graph.
///
/// ```text
/// NiObjectNET fields
/// u32       flags          visibility, culling, ... (e.g. 14 = 0b1110)
/// transform                local, relative to the parent node (`read_transform_trs`)
/// i32 ref   collision      bhkCollisionObject (-1 if none)
/// ```
///
/// Older games also store a property list here; Skyrim (BS version > 34) does not.
fn read_av_object(r: &mut Reader<'_>) -> Result<(Option<usize>, Transform), Error> {
    let name = read_object_net(r)?;
    r.u32()?; // flags
    let transform = read_transform_trs(r)?;
    r.index()?; // collision
    Ok((name, transform))
}

/// Decodes `BSVertexDesc`, the 64-bit description of a packed SE vertex.
///
/// ```text
/// bits  0..4    vertex size / 4         (e.g. 7 -> 28 bytes, 11 -> 44 bytes)
/// bits  8..16   position section size / 4 (4 -> 16 bytes = f32 x3 + bitangent x)
/// bits 16..44   offsets of the other attributes (not needed when reading in order)
/// bits 44..56   attribute flags:
///   bit 0 VF_VERTEX  position          bit 5 VF_COLORS   vertex colors
///   bit 1 VF_UV      texture coords    bit 6 VF_SKINNED  bone weights + indices
///   bit 3 VF_NORMAL  normal            bit 8 VF_EYEDATA  eye scale
///   bit 4 VF_TANGENT tangent           bit 10 VF_FULLPREC f32 positions
/// ```
///
/// Example: `0x0007b0008765040b` -> flags `0x07b` (position, UV, normal,
/// tangent, colors, skinned), vertex size 44 bytes.
const fn vertex_layout(desc: u64) -> VertexLayout {
    let flags = (desc >> 44) as u16;
    VertexLayout {
        has_uv: flags & (1 << 1) != 0,
        has_normal: flags & (1 << 3) != 0,
        has_tangent: flags & (1 << 4) != 0,
        has_color: flags & (1 << 5) != 0,
        skinned: flags & (1 << 6) != 0,
        has_eye_data: flags & (1 << 8) != 0,
    }
}

/// Packed normal/tangent component: `0..=255` -> `-1.0..=1.0`.
fn byte_to_unit(b: u8) -> f32 {
    (f32::from(b) / 255.0).mul_add(2.0, -1.0)
}

/// One packed vertex (`BSVertexData`). Present parts, in this order:
///
/// ```text
/// position   f32 x3 + f32 bitangent.x   (or f16 x3 + f16 bitangent.x)
/// uv         f16 x2                     u, v (v = 0 at the top of the image)
/// normal     u8 x3 + u8 bitangent.y     packed, see `byte_to_unit`
/// tangent    u8 x3 + u8 bitangent.z
/// color      u8 x4                      RGBA
/// skinning   f16 x4 weights + u8 x4 bone indices (indices into NiSkinInstance bones)
/// eye data   f32
/// ```
///
/// `force_full_precision`: positions are `f32` regardless of the description
/// (SE `BSTriShape`). Otherwise the size of the position section decides:
/// 16 bytes = `f32 x3 + bitangent`, 8 bytes = `f16 x4`. SE partitions use
/// 16 bytes without setting `VF_FULLPREC`.
fn read_vertex(
    r: &mut Reader<'_>,
    desc: u64,
    layout: &VertexLayout,
    force_full_precision: bool,
) -> Result<Vertex, Error> {
    let main_size = ((desc & 0xff00) >> 8) as usize * 4;
    let has_position = (desc >> 44) & 1 != 0;
    let full_precision = force_full_precision || main_size == 16 || (desc >> 44) & 0x400 != 0;
    let mut vertex = Vertex::default();

    if has_position && main_size <= 16 {
        if full_precision {
            vertex.position = r.vec3()?;
            r.f32()?; // bitangent x
        } else {
            vertex.position = [r.f16()?, r.f16()?, r.f16()?];
            r.f16()?;
        }
    } else if main_size > 16 {
        vertex.position = r.vec3()?;
        r.skip(main_size - 16)?; // extra floats
        r.f32()?;
    }

    if layout.has_uv {
        vertex.uv = [r.f16()?, r.f16()?];
    }
    if layout.has_normal {
        vertex.normal = [byte_to_unit(r.u8()?), byte_to_unit(r.u8()?), byte_to_unit(r.u8()?)];
        r.u8()?; // bitangent y
        if layout.has_tangent {
            r.skip(4)?;
        }
    }
    if layout.has_color {
        r.skip(4)?;
    }
    if layout.skinned {
        vertex.weights = [r.f16()?, r.f16()?, r.f16()?, r.f16()?];
        vertex.bones = [r.u8()?, r.u8()?, r.u8()?, r.u8()?];
    }
    if layout.has_eye_data {
        r.f32()?;
    }
    Ok(vertex)
}

/// Reads `count` vertices of exactly `vertex_size` bytes each.
fn read_vertices(
    r: &mut Reader<'_>,
    count: usize,
    vertex_size: usize,
    desc: u64,
    layout: &VertexLayout,
    full_precision: bool,
) -> Result<Vec<Vertex>, Error> {
    r.check_len(count, vertex_size)?;
    (0..count)
        .map(|_| read_vertex(&mut Reader::new(r.take(vertex_size)?), desc, layout, full_precision))
        .collect()
}

/// `u16 x3` vertex indices per triangle.
fn read_triangles(r: &mut Reader<'_>, count: usize) -> Result<Vec<[u16; 3]>, Error> {
    r.check_len(count, 6)?;
    (0..count).map(|_| Ok([r.u16()?, r.u16()?, r.u16()?])).collect()
}

/// `BSTriShape` (SE mesh). Unlike `NiTriShape`, the geometry is inside the block.
///
/// ```text
/// NiObjectNET fields
/// u32       flags
/// transform
/// i32 ref   collision
/// f32 x4    bounding sphere (center, radius)
/// i32 ref   skin instance     NiSkinInstance, -1 if rigid
/// i32 ref   shader property   BSLightingShaderProperty
/// i32 ref   alpha property
/// u64       vertex desc       see `vertex_layout`
/// u16       triangle count    (u32 for BS version >= 130)
/// u16       vertex count
/// u32       data size         vertex size * vertex count + 6 * triangle count; 0 when skinned (SE)
/// vertices                    `read_vertex` x vertex count   (if data size > 0)
/// triangles                   u16 x3 x triangle count       (if data size > 0)
/// u32       particle data size, then half-float positions/normals + triangles if > 0
/// ```
///
/// `BSDynamicTriShape` (facegen heads) appends `u32 size` and `f32 x4` per
/// vertex: the real positions (the packed ones are zero).
fn read_bs_tri_shape(r: &mut Reader<'_>, header: &Header, dynamic: bool) -> Result<Block, Error> {
    let name = read_object_net(r)?;
    r.u32()?; // flags
    let transform = read_transform_trs(r)?;
    r.index()?; // collision
    r.skip(16)?; // bounding sphere
    if header.bs_version > 139 {
        r.skip(24)?;
    }
    let skin = r.index()?;
    let shader = r.index()?;
    let _alpha = r.index()?;
    let desc = r.u64()?;
    let layout = vertex_layout(desc);

    let num_triangles = if header.user_version >= 12 && header.bs_version < 130 {
        r.u16()? as usize
    } else {
        r.u32()? as usize
    };
    let num_vertices = r.u16()? as usize;
    let data_size = r.u32()?;

    let (vertices, triangles) = if data_size > 0 {
        let full_precision = header.bs_version == 100 || (desc >> 44) & 0x400 != 0;
        // `data_size` also counts the triangles; the vertex size is in the description.
        let vertex_size = (desc & 0xf) as usize * 4;
        let vertices = read_vertices(r, num_vertices, vertex_size, desc, &layout, full_precision)?;
        (vertices, read_triangles(r, num_triangles)?)
    } else {
        (Vec::new(), Vec::new())
    };

    // SE particle data (half-float positions/normals + triangles).
    if header.user_version == 12 && header.bs_version == 100 && r.u32()? > 0 {
        r.skip(num_vertices * 12 + num_triangles * 6)?;
    }

    // `BSDynamicTriShape` (facegen heads): positions are stored separately.
    let dynamic_positions = if dynamic {
        let _size = r.u32()?;
        r.check_len(num_vertices, 16)?;
        (0..num_vertices)
            .map(|_| {
                let position = r.vec3()?;
                r.f32()?; // bitangent x
                Ok(position)
            })
            .collect::<Result<_, Error>>()?
    } else {
        Vec::new()
    };

    Ok(Block::TriShape {
        name,
        transform,
        skin,
        shader,
        layout,
        vertices,
        triangles,
        dynamic_positions,
    })
}

/// `NiTriShapeData` / `NiTriStripsData` (LE geometry).
///
/// ```text
/// i32       group id
/// u16       vertex count
/// u8 x2     keep / compress flags
/// u8        has vertices,  then f32 x3 x vertex count
/// u16       data flags     bit 0: has UVs, bits 12..16: tangent space data present
/// u32       material CRC
/// u8        has normals,   then f32 x3 x vertex count
///                          (+ tangents and bitangents, f32 x3 x vertex count each, if flags bits 12..)
/// f32 x4    bounding sphere
/// u8        has colors,    then f32 x4 x vertex count
/// f32 x2 x vertex count    UVs, if data flags bit 0 (v = 0 at the top of the image)
/// u16       consistency flags
/// i32 ref   additional data
/// u16       triangle count
/// NiTriShapeData:  u32 point count (3 x triangles), u8 has triangles, u16 x3 x triangle count,
///                  u16 match group count + groups (shared vertices, not read)
/// NiTriStripsData: u16 strip count, u16 length per strip, u8 has points, u16 x length per strip
/// ```
///
/// A strip `a b c d e` is the triangles `abc`, `cbd` (flipped), `cde`, ...
fn read_geometry_data(r: &mut Reader<'_>, strips: bool) -> Result<Block, Error> {
    r.i32()?; // group id
    let num_vertices = r.u16()? as usize;
    r.skip(2)?; // keep / compress flags
    let has_vertices = r.bool()?;
    r.check_len(num_vertices, 12)?;
    let vertices = if has_vertices {
        (0..num_vertices).map(|_| r.vec3()).collect::<Result<_, _>>()?
    } else {
        Vec::new()
    };

    let data_flags = r.u16()?;
    r.u32()?; // material CRC (bs version > 34)
    let has_normals = r.bool()?;
    let mut normals = Vec::new();
    if has_normals {
        normals = (0..num_vertices).map(|_| r.vec3()).collect::<Result<_, _>>()?;
        if data_flags & 0xf000 != 0 {
            r.skip(num_vertices.saturating_mul(24))?; // tangents + bitangents
        }
    }
    r.skip(16)?; // bounding sphere
    if r.bool()? {
        r.skip(num_vertices.saturating_mul(16))?; // vertex colors
    }
    let uvs = if data_flags & 1 != 0 {
        (0..num_vertices).map(|_| r.vec2()).collect::<Result<_, _>>()?
    } else {
        Vec::new()
    };
    r.u16()?; // consistency flags
    r.index()?; // additional data

    let num_triangles = r.u16()? as usize;
    let triangles = if strips {
        let num_strips = r.u16()? as usize;
        let lengths = (0..num_strips).map(|_| r.u16()).collect::<Result<Vec<_>, _>>()?;
        let mut triangles = Vec::with_capacity(num_triangles);
        if r.bool()? {
            for len in lengths {
                let points = (0..len).map(|_| r.u16()).collect::<Result<Vec<_>, _>>()?;
                for (i, w) in points.windows(3).enumerate() {
                    if w[0] == w[1] || w[1] == w[2] || w[0] == w[2] {
                        continue; // degenerate strip joint
                    }
                    triangles.push(if i % 2 == 0 { [w[0], w[1], w[2]] } else { [w[0], w[2], w[1]] });
                }
            }
        }
        triangles
    } else {
        r.u32()?; // num triangle points
        if r.bool()? {
            read_triangles(r, num_triangles)?
        } else {
            Vec::new()
        }
    };

    Ok(Block::GeometryData {
        vertices,
        normals,
        uvs,
        triangles,
    })
}

/// `NiSkinData`: bind transforms (and LE weights).
///
/// Transforms here are stored rotation first (`read_transform_rts`).
///
/// ```text
/// transform  global -> skin                  (see `Skin::global_to_skin`)
/// u32        bone count                      same order as NiSkinInstance bones
/// u8         has vertex weights              1 on LE; SE meshes may keep them in the partition only
/// per bone:
///   transform  skin -> bone at bind time     (see `Skin::skin_to_bone`)
///   f32 x4     bounding sphere of the bone's vertices
///   u16        weighted vertex count
///   (u16 vertex index, f32 weight) x count   if has vertex weights
/// ```
fn read_skin_data(r: &mut Reader<'_>) -> Result<Block, Error> {
    let global_to_skin = read_transform_rts(r)?;
    let num_bones = r.u32()? as usize;
    let has_vertex_weights = r.u8()? != 0;
    r.check_len(num_bones, 70)?;

    let bones = (0..num_bones)
        .map(|_| {
            let skin_to_bone = read_transform_rts(r)?;
            r.skip(16)?; // bounding sphere
            let num_vertices = r.u16()? as usize;
            let weights = if has_vertex_weights {
                r.check_len(num_vertices, 6)?;
                (0..num_vertices)
                    .map(|_| Ok((r.u16()?, r.f32()?)))
                    .collect::<Result<_, Error>>()?
            } else {
                Vec::new()
            };
            Ok((skin_to_bone, weights))
        })
        .collect::<Result<_, Error>>()?;

    Ok(Block::SkinData {
        global_to_skin,
        bones,
        has_vertex_weights,
    })
}

/// `NiSkinPartition`: the mesh split into GPU-friendly pieces, each using a
/// limited set of bones.
///
/// ```text
/// u32 partition count
/// SE only (BS version 100):
///   u32 data size, u32 vertex size, u64 vertex desc
///   vertices: `read_vertex` x (data size / vertex size)   the whole mesh, global indices
/// per partition:
///   u16 vertex count, u16 triangle count, u16 bone count, u16 strip count,
///   u16 weights per vertex (usually 4)
///   u16 x bone count            bones used: partition-local bone i = skin instance bone bones[i]
///   u8 has vertex map,          u16 x vertex count: partition vertex -> mesh vertex
///   u8 has vertex weights,      f32 x weights per vertex x vertex count
///   u16 x strip count           strip lengths
///   u8 has faces,               strips (u16 x lengths) or triangles (u16 x3, partition-local)
///   u8 has bone indices,        u8 x weights per vertex x vertex count (partition-local bones)
///   u8 LOD level, u8 global vertex buffer flag
///   SE only: u64 vertex desc, u16 x3 x triangle count "true triangles" (global vertex indices)
/// ```
///
/// On SE the skinned `BSTriShape` has no data of its own: its vertices
/// (with weights) and triangles are the global vertex buffer and the
/// partitions' true triangles.
fn read_skin_partition(r: &mut Reader<'_>, header: &Header) -> Result<Block, Error> {
    let num_partitions = r.u32()? as usize;
    let is_se = header.bs_version == 100;

    let mut layout = VertexLayout::default();
    let mut vertices = Vec::new();
    if is_se {
        let data_size = r.u32()? as usize;
        let vertex_size = r.u32()? as usize;
        let desc = r.u64()?;
        layout = vertex_layout(desc);
        if data_size > 0 {
            if vertex_size == 0 {
                return InvalidDataSnafu {
                    message: "NiSkinPartition vertex size is zero".to_owned(),
                }
                .fail();
            }
            vertices = read_vertices(r, data_size / vertex_size, vertex_size, desc, &layout, false)?;
        }
    }

    let mut partitions = Vec::with_capacity(num_partitions.min(1024));
    for _ in 0..num_partitions {
        let num_vertices = r.u16()? as usize;
        let num_triangles = r.u16()? as usize;
        let num_bones = r.u16()? as usize;
        let num_strips = r.u16()? as usize;
        let weights_per_vertex = r.u16()? as usize;

        let bones = (0..num_bones).map(|_| r.u16()).collect::<Result<Vec<_>, _>>()?;
        let vertex_map = if r.bool()? {
            (0..num_vertices).map(|_| r.u16()).collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        let weights = if r.bool()? {
            let count = num_vertices * weights_per_vertex;
            r.check_len(count, 4)?;
            (0..count).map(|_| r.f32()).collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        let strip_lengths = (0..num_strips).map(|_| r.u16()).collect::<Result<Vec<_>, _>>()?;
        if r.bool()? {
            if num_strips == 0 {
                r.skip(num_triangles * 6)?;
            } else {
                let points: usize = strip_lengths.iter().map(|&l| l as usize).sum();
                r.skip(points * 2)?;
            }
        }
        let bone_indices = if r.bool()? {
            r.take(num_vertices * weights_per_vertex)?.to_vec()
        } else {
            Vec::new()
        };
        r.skip(2)?; // lod level, global vb

        let true_triangles = if is_se {
            r.u64()?; // vertex desc
            read_triangles(r, num_triangles)?
        } else {
            Vec::new()
        };

        partitions.push(Partition {
            bones,
            vertex_map,
            weights,
            bone_indices,
            weights_per_vertex,
            true_triangles,
        });
    }

    Ok(Block::SkinPartition {
        layout,
        vertices,
        partitions,
    })
}

// ---------------------------------------------------------------------------
// Scene assembly

struct SceneBuilder<'a> {
    blocks: &'a [Block],
    strings: &'a [String],
    scene: Scene,
    visited: HashSet<usize>,
}

impl SceneBuilder<'_> {
    fn string(&self, index: Option<usize>) -> String {
        index.and_then(|i| self.strings.get(i)).cloned().unwrap_or_default()
    }

    fn push_node(&mut self, name: String, transform: Transform, parent: Option<usize>) -> usize {
        self.scene.nodes.push(Node {
            name,
            parent,
            transform,
        });
        self.scene.nodes.len() - 1
    }

    fn visit(&mut self, block_index: usize, parent: Option<usize>) -> Result<(), Error> {
        if !self.visited.insert(block_index) {
            return Ok(()); // cycles / shared children
        }
        let Some(block) = self.blocks.get(block_index) else {
            return Ok(());
        };

        match block {
            Block::Node {
                name,
                transform,
                children,
            } => {
                let node = self.push_node(self.string(*name), *transform, parent);
                for child in children.iter().flatten() {
                    self.visit(*child, Some(node))?;
                }
            }
            Block::TriShape {
                name,
                transform,
                skin,
                shader,
                layout,
                vertices,
                triangles,
                dynamic_positions,
            } => {
                let name = self.string(*name);
                let node = self.push_node(name.clone(), *transform, parent);
                let mut shape = Shape {
                    name,
                    node,
                    textures: self.textures(*shader),
                    ..Default::default()
                };

                let mut vertices = vertices.as_slice();
                let mut triangles = triangles.clone();
                let mut layout = layout;

                // SE skinned shapes keep their geometry in the partition.
                let partition = self.skin_partition(*skin);
                if let Some(Block::SkinPartition {
                    layout: partition_layout,
                    vertices: partition_vertices,
                    partitions,
                }) = partition
                    && vertices.is_empty()
                {
                    vertices = partition_vertices;
                    layout = partition_layout;
                    triangles = partitions.iter().flat_map(|p| p.true_triangles.iter().copied()).collect();
                }

                shape.vertices = if dynamic_positions.len() == vertices.len() {
                    dynamic_positions.clone()
                } else {
                    vertices.iter().map(|v| v.position).collect()
                };
                if layout.has_normal {
                    shape.normals = vertices.iter().map(|v| v.normal).collect();
                }
                if layout.has_uv {
                    shape.uvs = vertices.iter().map(|v| v.uv).collect();
                }
                shape.triangles = triangles;

                if let Some(mut skin) = self.skin(*skin, vertices.len())? {
                    // Packed vertices carry the weights (as nifly reads them).
                    if layout.skinned {
                        skin.weights = vertices
                            .iter()
                            .map(|v| {
                                (0..4)
                                    .filter(|&i| v.weights[i] > 0.0)
                                    .map(|i| (u16::from(v.bones[i]), v.weights[i]))
                                    .collect()
                            })
                            .collect();
                    }
                    shape.skin = Some(skin);
                }

                self.scene.shapes.push(shape);
            }
            Block::Geometry {
                name,
                transform,
                data,
                skin,
                shader,
            } => {
                let name = self.string(*name);
                let node = self.push_node(name.clone(), *transform, parent);
                let Some(Block::GeometryData {
                    vertices,
                    normals,
                    uvs,
                    triangles,
                }) = data.and_then(|d| self.blocks.get(d))
                else {
                    return Ok(());
                };

                let skin = self.skin(*skin, vertices.len())?;
                self.scene.shapes.push(Shape {
                    name,
                    node,
                    vertices: vertices.clone(),
                    normals: normals.clone(),
                    uvs: uvs.clone(),
                    triangles: triangles.clone(),
                    skin,
                    textures: self.textures(*shader),
                });
            }
            _ => {}
        }
        Ok(())
    }

    fn textures(&self, shader: Option<usize>) -> Vec<String> {
        let Some(Block::LightingShader { texture_set }) = shader.and_then(|s| self.blocks.get(s)) else {
            return Vec::new();
        };
        let Some(Block::TextureSet { textures }) = texture_set.and_then(|t| self.blocks.get(t)) else {
            return Vec::new();
        };
        textures.clone()
    }

    fn skin_partition(&self, skin: Option<usize>) -> Option<&Block> {
        let Some(Block::SkinInstance { partition, .. }) = skin.and_then(|s| self.blocks.get(s)) else {
            return None;
        };
        partition.and_then(|p| self.blocks.get(p))
    }

    /// Bones, bind transforms and weights from `NiSkinData` (or the partition).
    fn skin(&self, skin: Option<usize>, num_vertices: usize) -> Result<Option<Skin>, Error> {
        let Some(Block::SkinInstance {
            data,
            partition,
            bones,
        }) = skin.and_then(|s| self.blocks.get(s))
        else {
            return Ok(None);
        };
        let Some(Block::SkinData {
            global_to_skin,
            bones: bone_data,
            has_vertex_weights,
        }) = data.and_then(|d| self.blocks.get(d))
        else {
            return InvalidDataSnafu {
                message: "NiSkinInstance without NiSkinData".to_owned(),
            }
            .fail();
        };

        let bone_name = |bone: &Option<usize>| match bone.and_then(|b| self.blocks.get(b)) {
            Some(Block::Node { name, .. }) => self.string(*name),
            _ => String::new(),
        };

        let mut weights = vec![Vec::new(); num_vertices];
        if *has_vertex_weights {
            for (bone, (_, bone_weights)) in bone_data.iter().enumerate() {
                for &(vertex, weight) in bone_weights {
                    if let Some(w) = weights.get_mut(vertex as usize)
                        && weight > 0.0
                    {
                        w.push((bone as u16, weight));
                    }
                }
            }
        } else if let Some(Block::SkinPartition { partitions, .. }) = partition.and_then(|p| self.blocks.get(p)) {
            for p in partitions {
                for (local, &vertex) in p.vertex_map.iter().enumerate() {
                    let Some(w) = weights.get_mut(vertex as usize) else {
                        continue;
                    };
                    if !w.is_empty() {
                        continue; // shared vertex already filled by another partition
                    }
                    for k in 0..p.weights_per_vertex {
                        let i = local * p.weights_per_vertex + k;
                        let (Some(&weight), Some(&bone)) = (p.weights.get(i), p.bone_indices.get(i)) else {
                            continue;
                        };
                        if weight > 0.0
                            && let Some(&global) = p.bones.get(bone as usize)
                        {
                            w.push((global, weight));
                        }
                    }
                }
            }
        }

        Ok(Some(Skin {
            bones: bones.iter().map(bone_name).collect(),
            global_to_skin: *global_to_skin,
            skin_to_bone: bone_data.iter().map(|(t, _)| *t).collect(),
            weights,
        }))
    }
}
