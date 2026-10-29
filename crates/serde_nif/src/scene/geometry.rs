//! Shape geometry: SE packed vertices (`BSTriShape`) and LE `NiTriShapeData`.

use super::blocks::{Block, read_object_net, read_transform_trs};
use crate::error::Error;
use crate::header::Header;
use crate::reader::Reader;
#[derive(Debug, Clone, Copy, Default)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub normal: [f32; 3],
    pub weights: [f32; 4],
    pub bones: [u8; 4],
}

#[derive(Debug, Default)]
pub struct VertexLayout {
    pub has_uv: bool,
    pub has_normal: bool,
    pub has_tangent: bool,
    pub has_color: bool,
    pub skinned: bool,
    pub has_eye_data: bool,
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
pub const fn vertex_layout(desc: u64) -> VertexLayout {
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
        vertex.normal = [
            byte_to_unit(r.u8()?),
            byte_to_unit(r.u8()?),
            byte_to_unit(r.u8()?),
        ];
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
pub fn read_vertices(
    r: &mut Reader<'_>,
    count: usize,
    vertex_size: usize,
    desc: u64,
    layout: &VertexLayout,
    full_precision: bool,
) -> Result<Vec<Vertex>, Error> {
    r.check_len(count, vertex_size)?;
    (0..count)
        .map(|_| {
            read_vertex(
                &mut Reader::new(r.take(vertex_size)?),
                desc,
                layout,
                full_precision,
            )
        })
        .collect()
}

/// `u16 x3` vertex indices per triangle.
pub fn read_triangles(r: &mut Reader<'_>, count: usize) -> Result<Vec<[u16; 3]>, Error> {
    r.check_len(count, 6)?;
    (0..count)
        .map(|_| Ok([r.u16()?, r.u16()?, r.u16()?]))
        .collect()
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
pub fn read_bs_tri_shape<'a>(
    r: &mut Reader<'_>,
    header: &Header<'_>,
    dynamic: bool,
) -> Result<Block<'a>, Error> {
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
pub fn read_geometry_data<'a>(r: &mut Reader<'_>, strips: bool) -> Result<Block<'a>, Error> {
    r.i32()?; // group id
    let num_vertices = r.u16()? as usize;
    r.skip(2)?; // keep / compress flags
    let has_vertices = r.bool()?;
    r.check_len(num_vertices, 12)?;
    let vertices = if has_vertices {
        (0..num_vertices)
            .map(|_| r.vec3())
            .collect::<Result<_, _>>()?
    } else {
        Vec::new()
    };

    let data_flags = r.u16()?;
    r.u32()?; // material CRC (bs version > 34)
    let has_normals = r.bool()?;
    let mut normals = Vec::new();
    if has_normals {
        normals = (0..num_vertices)
            .map(|_| r.vec3())
            .collect::<Result<_, _>>()?;
        if data_flags & 0xf000 != 0 {
            r.skip(num_vertices.saturating_mul(24))?; // tangents + bitangents
        }
    }
    r.skip(16)?; // bounding sphere
    if r.bool()? {
        r.skip(num_vertices.saturating_mul(16))?; // vertex colors
    }
    let uvs = if data_flags & 1 != 0 {
        (0..num_vertices)
            .map(|_| r.vec2())
            .collect::<Result<_, _>>()?
    } else {
        Vec::new()
    };
    r.u16()?; // consistency flags
    r.index()?; // additional data

    let num_triangles = r.u16()? as usize;
    let triangles = if strips {
        let num_strips = r.u16()? as usize;
        let lengths = (0..num_strips)
            .map(|_| r.u16())
            .collect::<Result<Vec<_>, _>>()?;
        let mut triangles = Vec::with_capacity(num_triangles);
        if r.bool()? {
            for len in lengths {
                let points = (0..len).map(|_| r.u16()).collect::<Result<Vec<_>, _>>()?;
                for (i, w) in points.windows(3).enumerate() {
                    if w[0] == w[1] || w[1] == w[2] || w[0] == w[2] {
                        continue; // degenerate strip joint
                    }
                    triangles.push(if i % 2 == 0 {
                        [w[0], w[1], w[2]]
                    } else {
                        [w[0], w[2], w[1]]
                    });
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
