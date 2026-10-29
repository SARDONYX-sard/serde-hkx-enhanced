//! Scene assembly: turns the decoded block graph into nodes and shapes.

use std::borrow::Cow;
use std::collections::HashSet;

use super::blocks::Block;
use super::{Node, Scene, Shape, Skin};
use crate::error::{Error, InvalidDataSnafu};
use crate::transform::Transform;
/// Walks the block graph from the roots and flattens it into a [Scene].
pub struct SceneBuilder<'b, 'a> {
    pub blocks: &'b [Block<'a>],
    pub strings: &'b [Cow<'a, str>],
    pub scene: Scene<'a>,
    /// Blocks already visited: guards against cycles and shared children.
    pub visited: HashSet<usize>,
}

impl<'a> SceneBuilder<'_, 'a> {
    /// Cloning is cheap: table strings borrow the file unless they were not UTF-8.
    fn string(&self, index: Option<usize>) -> Cow<'a, str> {
        index
            .and_then(|i| self.strings.get(i))
            .cloned()
            .unwrap_or_default()
    }

    fn push_node(
        &mut self,
        name: Cow<'a, str>,
        transform: Transform,
        parent: Option<usize>,
    ) -> usize {
        self.scene.nodes.push(Node {
            name,
            parent,
            transform,
        });
        self.scene.nodes.len() - 1
    }

    pub fn visit(&mut self, block_index: usize, parent: Option<usize>) -> Result<(), Error> {
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
                    triangles = partitions
                        .iter()
                        .flat_map(|p| p.true_triangles.iter().copied())
                        .collect();
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

    fn textures(&self, shader: Option<usize>) -> Vec<Cow<'a, str>> {
        let Some(Block::LightingShader { texture_set }) = shader.and_then(|s| self.blocks.get(s))
        else {
            return Vec::new();
        };
        let Some(Block::TextureSet { textures }) = texture_set.and_then(|t| self.blocks.get(t))
        else {
            return Vec::new();
        };
        textures.clone()
    }

    fn skin_partition(&self, skin: Option<usize>) -> Option<&Block<'a>> {
        let Some(Block::SkinInstance { partition, .. }) = skin.and_then(|s| self.blocks.get(s))
        else {
            return None;
        };
        partition.and_then(|p| self.blocks.get(p))
    }

    /// Bones, bind transforms and weights from `NiSkinData` (or the partition).
    fn skin(&self, skin: Option<usize>, num_vertices: usize) -> Result<Option<Skin<'a>>, Error> {
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
            _ => Cow::Borrowed(""),
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
        } else if let Some(Block::SkinPartition { partitions, .. }) =
            partition.and_then(|p| self.blocks.get(p))
        {
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
                        let (Some(&weight), Some(&bone)) =
                            (p.weights.get(i), p.bone_indices.get(i))
                        else {
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
