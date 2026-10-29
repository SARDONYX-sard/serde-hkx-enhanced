//! Block decoding: which NIF block types are read, and the shared prefixes
//! (`NiObjectNET`, `NiAVObject`) every scene object starts with.

use std::borrow::Cow;

use super::geometry::{Vertex, VertexLayout, read_bs_tri_shape, read_geometry_data};
use super::skin::{Partition, read_skin_data, read_skin_partition};
use crate::error::Error;
use crate::header::Header;
use crate::reader::Reader;
use crate::transform::Transform;

#[derive(Debug)]
pub enum Block<'a> {
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
        textures: Vec<Cow<'a, str>>,
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
pub fn read_block<'a>(
    r: &mut Reader<'a>,
    block_type: &str,
    header: &Header<'_>,
) -> Result<Block<'a>, Error> {
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
        "BSTriShape" | "BSMeshLODTriShape" | "BSSubIndexTriShape" => {
            read_bs_tri_shape(r, header, false)?
        }
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
        "NiTriShapeData" | "NiTriStripsData" => {
            read_geometry_data(r, block_type == "NiTriStripsData")?
        }
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
                textures: (0..count)
                    .map(|_| r.sized_string())
                    .collect::<Result<_, _>>()?,
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
pub fn read_object_net(r: &mut Reader<'_>) -> Result<Option<usize>, Error> {
    let name = r.index()?;
    r.index_list()?; // extra data
    r.index()?; // controller
    Ok(name)
}

/// `f32 x3` translation, `f32 x9` rotation (row-major 3x3), `f32` scale.
pub fn read_transform_trs(r: &mut Reader<'_>) -> Result<Transform, Error> {
    Ok(Transform {
        translation: r.vec3()?,
        rotation: r.matrix33()?,
        scale: r.f32()?,
    })
}

/// `NiSkinData` stores rotation first.
pub fn read_transform_rts(r: &mut Reader<'_>) -> Result<Transform, Error> {
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
