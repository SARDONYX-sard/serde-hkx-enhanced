//! Skinning blocks: bind transforms (`NiSkinData`) and the GPU partitions
//! that carry SE vertex buffers and weights (`NiSkinPartition`).

use super::blocks::{Block, read_transform_rts};
use super::geometry::{VertexLayout, read_triangles, read_vertices, vertex_layout};
use crate::error::{Error, InvalidDataSnafu};
use crate::header::Header;
use crate::reader::Reader;
#[derive(Debug)]
pub struct Partition {
    pub bones: Vec<u16>,
    pub vertex_map: Vec<u16>,
    /// `[vertex][weights_per_vertex]`
    pub weights: Vec<f32>,
    pub bone_indices: Vec<u8>,
    pub weights_per_vertex: usize,
    /// SE only: triangles indexing the global vertex buffer.
    pub true_triangles: Vec<[u16; 3]>,
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
pub fn read_skin_data<'a>(r: &mut Reader<'_>) -> Result<Block<'a>, Error> {
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
pub fn read_skin_partition<'a>(
    r: &mut Reader<'_>,
    header: &Header<'_>,
) -> Result<Block<'a>, Error> {
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
            vertices = read_vertices(
                r,
                data_size / vertex_size,
                vertex_size,
                desc,
                &layout,
                false,
            )?;
        }
    }

    let mut partitions = Vec::with_capacity(num_partitions.min(1024));
    for _ in 0..num_partitions {
        let num_vertices = r.u16()? as usize;
        let num_triangles = r.u16()? as usize;
        let num_bones = r.u16()? as usize;
        let num_strips = r.u16()? as usize;
        let weights_per_vertex = r.u16()? as usize;

        let bones = (0..num_bones)
            .map(|_| r.u16())
            .collect::<Result<Vec<_>, _>>()?;
        let vertex_map = if r.bool()? {
            (0..num_vertices)
                .map(|_| r.u16())
                .collect::<Result<Vec<_>, _>>()?
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
        let strip_lengths = (0..num_strips)
            .map(|_| r.u16())
            .collect::<Result<Vec<_>, _>>()?;
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
