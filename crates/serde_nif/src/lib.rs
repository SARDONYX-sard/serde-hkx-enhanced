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

//! Read Skyrim LE/SE NIF scenes (node hierarchy, triangle shapes, skinning)
//! and read/write KF animation files ([kf]).
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
mod header;
pub mod kf;
mod reader;
mod scene;
mod transform;
mod writer;

pub use self::error::Error;
pub use self::scene::{Node, Scene, Shape, Skin, from_bytes};
pub use self::transform::Transform;
