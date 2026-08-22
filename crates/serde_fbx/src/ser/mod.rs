//! Export skeleton animation data and optional NIF scene data through
//! `ufbx_write`.
//!
//! The skeleton and animation are always exported. When a [`NifScene`] is
//! provided, its nodes, meshes, materials, textures, and skinning data are
//! added to the same FBX scene.
//!
//! FFI API reference:
//! - https://raw.githubusercontent.com/ufbx/ufbx-write/refs/heads/main/ufbx_write.h
//! - https://raw.githubusercontent.com/ufbx/ufbx-write/refs/heads/main/ufbx_write.c
//!
//! # Errors
//!
//! The public [`to_fbx`] function returns [`Error`] when animation data is
//! invalid, a NIF reference is invalid, or `ufbx_write` fails to construct,
//! prepare, or serialize the FBX scene.

pub mod nif_compat;
mod sys_error;

use std::collections::HashMap;

use havok_types::{QsTransform, Quaternion, Vector4};
use nif_compat::{Matrix3, Mesh, Node, Scene, Skin, Texture};
use serde_spline::hkx::{Animation, Skeleton};
use ufbx_write::sys;

use crate::{error::Error, ser::nif_compat::Matrix4};

/// Configuration for FBX export.
#[derive(Debug)]
pub struct Config {
    /// Sampling rate of the output animation, in frames per second.
    pub fps: f32,

    /// Target FBX format.
    pub format: Format,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            fps: 30.0,
            format: Format::FbxBin,
        }
    }
}

/// Target FBX format.
#[derive(Debug, Clone, Copy)]
pub enum Format {
    /// Binary FBX.
    FbxBin,

    /// ASCII FBX.
    FbxAscii,
}

/// Exports a skeleton and sampled animation as an FBX file.
///
/// The skeleton and animation are always exported. The NIF scene is optional.
/// When `nif` is `Some`, NIF nodes, meshes, materials, textures, and skinning
/// data are added to the same FBX scene.
///
/// Skeleton nodes are created first. NIF nodes with the same name reuse the
/// corresponding skeleton node so animation and mesh skinning operate on the
/// same FBX node.
///
/// # Errors
///
/// Returns [`Error`] if:
///
/// - The animation track count is invalid.
/// - The declared frame count does not match the sampled frames.
/// - A sampled frame does not contain one transform for every skeleton bone.
/// - A NIF node has an invalid parent index.
/// - A NIF mesh has malformed vertex data.
/// - A NIF mesh references an invalid node, material, or skin.
/// - A NIF skin has inconsistent weight/index data.
/// - A NIF skin references a missing bone.
/// - `ufbx_write` fails to create, prepare, or serialize the scene.
pub fn to_fbx(
    animation: &Animation,
    skeleton: &Skeleton,
    nif: Option<&Scene>,
    config: Config,
) -> Result<Vec<u8>, Error> {
    let Config { fps, format } = config;

    validate_animation(skeleton, animation)?;

    let scene = unsafe { sys::ufbxw_create_scene(&scene_options()) };

    if scene.is_null() {
        return Err(Error::ExportFbx {
            message: "ufbxw_create_scene() returned NULL".to_owned(),
        });
    }

    let result = export_scene(scene, skeleton, animation, nif, fps, format);

    unsafe {
        sys::ufbxw_free_scene(scene);
    }

    result
}

/// Builds the FBX scene and saves it.
///
/// # Errors
///
/// Returns [`Error`] if skeleton creation, NIF export, animation creation,
/// scene preparation, or serialization fails.
fn export_scene(
    scene: *mut sys::ufbxw_scene,
    skeleton: &Skeleton,
    animation: &Animation,
    nif: Option<&Scene>,
    fps: f32,
    format: Format,
) -> Result<Vec<u8>, Error> {
    set_scene_config_for_blender(scene, fps);

    let mut context = create_skeleton(scene, skeleton)?;

    if let Some(nif) = nif {
        create_nif_nodes(scene, nif, &mut context)?;
        create_nif_materials(scene, nif, &mut context)?;
        create_nif_meshes(scene, nif, &mut context)?;
        create_nif_skins(scene, nif, &context)?;
    }

    create_animation(scene, &context.skeleton_nodes, animation)?;

    unsafe {
        sys::ufbxw_prepare_scene(scene, &sys::ufbxw_default_prepare_opts);
    }

    save_memory(scene, format)
}

/// Mapping between source nodes and FBX nodes.
///
/// Skeleton nodes are always created first. NIF nodes that have the same name
/// reuse those nodes. NIF-only nodes are added to `nif_nodes`.
struct ExportContext {
    /// FBX nodes indexed by skeleton bone index.
    skeleton_nodes: Vec<sys::ufbxw_node>,

    /// FBX nodes indexed by NIF node index.
    nif_nodes: Vec<sys::ufbxw_node>,

    /// All FBX nodes indexed by name.
    nodes_by_name: HashMap<String, sys::ufbxw_node>,

    /// FBX meshes indexed by NIF mesh index.
    meshes: Vec<sys::ufbxw_mesh>,

    /// FBX materials indexed by NIF material index.
    materials: Vec<sys::ufbxw_material>,
}

/// Creates all skeleton nodes and their hierarchy.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if a node cannot be created or configured, or
/// if a skeleton parent index is invalid.
fn create_skeleton(
    scene: *mut sys::ufbxw_scene,
    skeleton: &Skeleton,
) -> Result<ExportContext, Error> {
    let mut skeleton_nodes = Vec::with_capacity(skeleton.bones.len());

    let mut nodes_by_name = HashMap::with_capacity(skeleton.bones.len());

    for bone in &skeleton.bones {
        let node = unsafe { sys::ufbxw_create_node(scene) };

        if node.id == 0 {
            return Err(Error::ExportFbx {
                message: format!("failed to create FBX node for bone {:?}", bone.name),
            });
        }

        set_node_name(scene, node, &bone.name)?;

        set_node_transform(scene, node, &bone.reference_pose)?;

        if nodes_by_name.insert(bone.name.clone(), node).is_some() {
            return Err(Error::ExportFbx {
                message: format!("duplicate skeleton bone name {:?}", bone.name),
            });
        }

        skeleton_nodes.push(node);
    }

    set_parent_nodes(scene, skeleton, &skeleton_nodes)?;

    Ok(ExportContext {
        skeleton_nodes,
        nif_nodes: Vec::new(),
        nodes_by_name,
        meshes: Vec::new(),
        materials: Vec::new(),
    })
}

/// Creates NIF nodes and reuses skeleton nodes with matching names.
///
/// NIF nodes are indexed independently from skeleton nodes because
/// [`NifMesh::node`] refers to a NIF node index.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if a NIF node cannot be created or if its
/// parent index is invalid.
fn create_nif_nodes(
    scene: *mut sys::ufbxw_scene,
    nif: &Scene,
    context: &mut ExportContext,
) -> Result<(), Error> {
    context.nif_nodes.reserve(nif.nodes.len());

    for nif_node in &nif.nodes {
        let node = if let Some(node) = context.nodes_by_name.get(&nif_node.name) {
            *node
        } else {
            let node = unsafe { sys::ufbxw_create_node(scene) };

            if node.id == 0 {
                return Err(Error::ExportFbx {
                    message: format!("failed to create FBX node for NIF node {:?}", nif_node.name),
                });
            }

            set_node_name(scene, node, &nif_node.name)?;

            set_nif_node_transform(scene, node, nif_node)?;

            context.nodes_by_name.insert(nif_node.name.clone(), node);

            node
        };

        context.nif_nodes.push(node);
    }

    for (index, nif_node) in nif.nodes.iter().enumerate() {
        if nif_node.parent < 0 {
            continue;
        }

        let parent_index = usize::try_from(nif_node.parent).map_err(|_| Error::ExportFbx {
            message: format!(
                "NIF node {:?} has invalid parent index {}",
                nif_node.name, nif_node.parent
            ),
        })?;

        let parent = context
            .nif_nodes
            .get(parent_index)
            .copied()
            .ok_or_else(|| Error::ExportFbx {
                message: format!(
                    "NIF node {:?} has invalid parent index {}",
                    nif_node.name, nif_node.parent
                ),
            })?;

        let node = context
            .nif_nodes
            .get(index)
            .copied()
            .ok_or_else(|| Error::ExportFbx {
                message: format!("missing FBX node for NIF node index {}", index),
            })?;

        unsafe {
            sys::ufbxw_node_set_parent(scene, node, parent);
        }
    }

    Ok(())
}

/// Creates FBX materials.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if a material or texture cannot be created, or
/// if a material references an invalid texture.
fn create_nif_materials(
    scene: *mut sys::ufbxw_scene,
    nif: &Scene,
    context: &mut ExportContext,
) -> Result<(), Error> {
    context.materials.reserve(nif.materials.len());

    for material in &nif.materials {
        let fbx_material = unsafe {
            sys::ufbxw_create_material(scene, sys::ufbxw_material_type_UFBXW_MATERIAL_FBX_PHONG)
        };

        if fbx_material.id == 0 {
            return Err(Error::ExportFbx {
                message: format!("failed to create FBX material {:?}", material.name),
            });
        }

        set_element_name(scene, fbx_material.id, &material.name)?;

        if material.texture >= 0 {
            let texture_index =
                usize::try_from(material.texture).map_err(|_| Error::ExportFbx {
                    message: format!(
                        "material {:?} has invalid texture index {}",
                        material.name, material.texture
                    ),
                })?;

            let texture = nif
                .textures
                .get(texture_index)
                .ok_or_else(|| Error::ExportFbx {
                    message: format!(
                        "material {:?} references invalid texture index {}",
                        material.name, texture_index
                    ),
                })?;

            connect_material_texture(scene, fbx_material, texture)?;
        }

        context.materials.push(fbx_material);
    }

    Ok(())
}

/// Creates the textures referenced by a material and connects them.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if a referenced texture cannot be created.
fn connect_material_texture(
    scene: *mut sys::ufbxw_scene,
    material: sys::ufbxw_material,
    texture: &Texture,
) -> Result<(), Error> {
    create_material_texture(scene, material, "DiffuseColor", &texture.diffuse)?;

    create_material_texture(scene, material, "NormalMap", &texture.normal)?;

    create_material_texture(scene, material, "EmissiveColor", &texture.glow)?;

    create_material_texture(scene, material, "SpecularColor", &texture.specular)?;

    Ok(())
}

/// Creates and connects one file texture.
///
/// Empty texture names are ignored.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if `ufbx_write` fails to create the texture.
fn create_material_texture(
    scene: *mut sys::ufbxw_scene,
    material: sys::ufbxw_material,
    property: &str,
    filename: &str,
) -> Result<(), Error> {
    if filename.is_empty() {
        return Ok(());
    }

    let texture =
        unsafe { sys::ufbxw_create_texture(scene, sys::ufbxw_texture_type_UFBXW_TEXTURE_FILE) };

    if texture.id == 0 {
        return Err(Error::ExportFbx {
            message: format!("failed to create FBX texture {:?}", filename),
        });
    }

    unsafe {
        sys::ufbxw_set_name_len(scene, texture.id, filename.as_ptr().cast(), filename.len());

        sys::ufbxw_texture_set_filename_len(
            scene,
            texture,
            filename.as_ptr().cast(),
            filename.len(),
        );

        sys::ufbxw_material_set_texture_len(
            scene,
            material,
            property.as_ptr().cast(),
            property.len(),
            texture,
        );
    }

    Ok(())
}

/// Creates NIF meshes.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if mesh data is malformed or if a mesh
/// references an invalid node or material.
fn create_nif_meshes(
    scene: *mut sys::ufbxw_scene,
    nif: &Scene,
    context: &mut ExportContext,
) -> Result<(), Error> {
    context.meshes.reserve(nif.meshes.len());

    for mesh in &nif.meshes {
        validate_mesh(mesh)?;

        let fbx_mesh = unsafe { sys::ufbxw_create_mesh(scene) };

        if fbx_mesh.id == 0 {
            return Err(Error::ExportFbx {
                message: format!("failed to create FBX mesh {:?}", mesh.name),
            });
        }

        set_element_name(scene, fbx_mesh.id, &mesh.name)?;

        let vertices = vec3_buffer(scene, &mesh.positions, &mesh.name, "positions")?;

        unsafe {
            sys::ufbxw_mesh_set_vertices(scene, fbx_mesh, vertices);
        }

        let indices = int_buffer(scene, &mesh.indices, &mesh.name, "indices")?;

        unsafe {
            sys::ufbxw_mesh_set_triangles(scene, fbx_mesh, indices);
        }

        if !mesh.normals.is_empty() {
            let normals = vec3_buffer(scene, &mesh.normals, &mesh.name, "normals")?;

            unsafe {
                sys::ufbxw_mesh_set_normals(
                    scene,
                    fbx_mesh,
                    normals,
                    sys::ufbxw_attribute_mapping_UFBXW_ATTRIBUTE_MAPPING_VERTEX,
                );
            }
        }

        if !mesh.uvs.is_empty() {
            let uvs = vec2_buffer(scene, &mesh.uvs, &mesh.name, "uvs")?;

            unsafe {
                sys::ufbxw_mesh_set_uvs(
                    scene,
                    fbx_mesh,
                    0,
                    uvs,
                    sys::ufbxw_attribute_mapping_UFBXW_ATTRIBUTE_MAPPING_VERTEX,
                );
            }
        }

        if !mesh.tangents.is_empty() {
            let tangents = vec3_buffer(scene, &mesh.tangents, &mesh.name, "tangents")?;

            unsafe {
                sys::ufbxw_mesh_set_tangents(
                    scene,
                    fbx_mesh,
                    0,
                    tangents,
                    sys::ufbxw_attribute_mapping_UFBXW_ATTRIBUTE_MAPPING_VERTEX,
                );
            }
        }

        let node_index = usize::try_from(mesh.node).map_err(|_| Error::ExportFbx {
            message: format!("mesh {:?} has invalid node index {}", mesh.name, mesh.node),
        })?;

        let node = context
            .nif_nodes
            .get(node_index)
            .copied()
            .ok_or_else(|| Error::ExportFbx {
                message: format!(
                    "mesh {:?} references invalid node index {}",
                    mesh.name, mesh.node
                ),
            })?;

        unsafe {
            sys::ufbxw_mesh_add_instance(scene, fbx_mesh, node);
        }

        if mesh.material >= 0 {
            let material_index = usize::try_from(mesh.material).map_err(|_| Error::ExportFbx {
                message: format!(
                    "mesh {:?} has invalid material index {}",
                    mesh.name, mesh.material
                ),
            })?;

            let material = context
                .materials
                .get(material_index)
                .copied()
                .ok_or_else(|| Error::ExportFbx {
                    message: format!(
                        "mesh {:?} references invalid material index {}",
                        mesh.name, mesh.material
                    ),
                })?;

            unsafe {
                sys::ufbxw_node_set_material(scene, node, 0, material);
            }
        }

        context.meshes.push(fbx_mesh);
    }

    Ok(())
}

/// Creates skin deformers and skin clusters for NIF meshes.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if a skin references an invalid mesh, bone,
/// weight array, or bind matrix.
fn create_nif_skins(
    scene: *mut sys::ufbxw_scene,
    nif: &Scene,
    context: &ExportContext,
) -> Result<(), Error> {
    for (mesh_index, mesh) in nif.meshes.iter().enumerate() {
        if mesh.skin < 0 {
            continue;
        }

        let skin_index = usize::try_from(mesh.skin).map_err(|_| Error::ExportFbx {
            message: format!("mesh {:?} has invalid skin index {}", mesh.name, mesh.skin),
        })?;

        let skin = nif.skins.get(skin_index).ok_or_else(|| Error::ExportFbx {
            message: format!(
                "mesh {:?} references invalid skin index {}",
                mesh.name, skin_index
            ),
        })?;

        let fbx_mesh = context
            .meshes
            .get(mesh_index)
            .copied()
            .ok_or_else(|| Error::ExportFbx {
                message: format!("missing FBX mesh for NIF mesh {:?}", mesh.name),
            })?;

        create_skin(scene, skin, mesh, fbx_mesh, context)?;
    }

    Ok(())
}

/// Creates one NIF skin deformer.
///
/// NIF skin data contains four influences per vertex in the current
/// `load_nif` representation.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if the skin data is malformed or a skin bone
/// cannot be resolved to an FBX node.
fn create_skin(
    scene: *mut sys::ufbxw_scene,
    skin: &Skin,
    mesh: &Mesh,
    fbx_mesh: sys::ufbxw_mesh,
    context: &ExportContext,
) -> Result<(), Error> {
    if skin.bone_indices.len() != skin.bone_weights.len() {
        return Err(Error::ExportFbx {
            message: format!(
                "skin for mesh {:?} has {} indices but {} weights",
                mesh.name,
                skin.bone_indices.len(),
                skin.bone_weights.len()
            ),
        });
    }

    let vertex_count = mesh.positions.len() / 3;

    if skin.bone_indices.len() != vertex_count * 4 {
        return Err(Error::ExportFbx {
            message: format!(
                "skin for mesh {:?} contains {} influences, expected {}",
                mesh.name,
                skin.bone_indices.len(),
                vertex_count * 4
            ),
        });
    }

    let deformer = unsafe { sys::ufbxw_create_skin_deformer(scene, fbx_mesh) };

    if deformer.id == 0 {
        return Err(Error::ExportFbx {
            message: format!("failed to create skin deformer for mesh {:?}", mesh.name),
        });
    }

    unsafe {
        sys::ufbxw_skin_deformer_set_skinning_type(
            scene,
            deformer,
            sys::ufbxw_skinning_type_UFBXW_SKINNING_TYPE_LINEAR,
        );
    }

    let bind_pose = unsafe { sys::ufbxw_create_bind_pose(scene) };

    if bind_pose.id == 0 {
        return Err(Error::ExportFbx {
            message: format!("failed to create bind pose for mesh {:?}", mesh.name),
        });
    }

    for (skin_bone_index, bone_name) in skin.bones.iter().enumerate() {
        let node = context
            .nodes_by_name
            .get(bone_name)
            .copied()
            .ok_or_else(|| Error::ExportFbx {
                message: format!(
                    "skin for mesh {:?} references missing bone {:?}",
                    mesh.name, bone_name
                ),
            })?;

        let cluster = unsafe { sys::ufbxw_create_skin_cluster(scene, deformer, node) };

        if cluster.id == 0 {
            return Err(Error::ExportFbx {
                message: format!("failed to create skin cluster for bone {:?}", bone_name),
            });
        }

        let mut vertex_indices = Vec::new();
        let mut weights = Vec::new();

        for vertex_index in 0..vertex_count {
            for influence in 0..4 {
                let offset = vertex_index * 4 + influence;

                if usize::from(skin.bone_indices[offset]) != skin_bone_index {
                    continue;
                }

                let weight = skin.bone_weights[offset];

                if weight == 0.0 {
                    continue;
                }

                vertex_indices.push(i32::try_from(vertex_index).map_err(|_| Error::ExportFbx {
                    message: format!("vertex index {} exceeds FBX i32 range", vertex_index),
                })?);

                weights.push(weight as f64);
            }
        }

        let index_buffer = unsafe {
            sys::ufbxw_copy_int_array(scene, vertex_indices.as_ptr(), vertex_indices.len())
        };

        let weight_buffer =
            unsafe { sys::ufbxw_copy_real_array(scene, weights.as_ptr(), weights.len()) };

        unsafe {
            sys::ufbxw_skin_cluster_set_weights(scene, cluster, index_buffer, weight_buffer);
        }

        if let Some(bind_matrix) = skin.bind_matrices.get(skin_bone_index) {
            let matrix = nif_matrix_to_ufbx(bind_matrix);

            unsafe {
                sys::ufbxw_skin_cluster_set_link_transform(scene, cluster, matrix);

                sys::ufbxw_bind_pose_add_node(scene, bind_pose, node, matrix);
            }
        }
    }

    unsafe {
        sys::ufbxw_skin_deformer_set_bind_pose(scene, deformer, bind_pose);
    }

    Ok(())
}

/// Validates a NIF mesh.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if vertex or index data is malformed.
fn validate_mesh(mesh: &Mesh) -> Result<(), Error> {
    if !mesh.positions.len().is_multiple_of(3) {
        return Err(Error::ExportFbx {
            message: format!(
                "mesh {:?} has {} position floats, expected a multiple of 3",
                mesh.name,
                mesh.positions.len()
            ),
        });
    }

    let vertex_count = mesh.positions.len() / 3;

    if !mesh.normals.is_empty() && mesh.normals.len() != vertex_count * 3 {
        return Err(Error::ExportFbx {
            message: format!(
                "mesh {:?} has {} normal floats, expected {}",
                mesh.name,
                mesh.normals.len(),
                vertex_count * 3
            ),
        });
    }

    if !mesh.uvs.is_empty() && mesh.uvs.len() != vertex_count * 2 {
        return Err(Error::ExportFbx {
            message: format!(
                "mesh {:?} has {} UV floats, expected {}",
                mesh.name,
                mesh.uvs.len(),
                vertex_count * 2
            ),
        });
    }

    if !mesh.tangents.is_empty() && mesh.tangents.len() != vertex_count * 3 {
        return Err(Error::ExportFbx {
            message: format!(
                "mesh {:?} has {} tangent floats, expected {}",
                mesh.name,
                mesh.tangents.len(),
                vertex_count * 3
            ),
        });
    }

    if !mesh.indices.len().is_multiple_of(3) {
        return Err(Error::ExportFbx {
            message: format!(
                "mesh {:?} has {} indices, expected a multiple of 3",
                mesh.name,
                mesh.indices.len()
            ),
        });
    }

    for &index in &mesh.indices {
        if usize::try_from(index)
            .ok()
            .is_none_or(|index| index >= vertex_count)
        {
            return Err(Error::ExportFbx {
                message: format!(
                    "mesh {:?} contains out-of-range vertex index {}",
                    mesh.name, index
                ),
            });
        }
    }

    Ok(())
}

/// Sets an FBX element name.
///
/// # Errors
///
/// This function does not fail because the underlying API accepts a pointer
/// and explicit byte length.
fn set_element_name(
    scene: *mut sys::ufbxw_scene,
    id: sys::ufbxw_id,
    name: &str,
) -> Result<(), Error> {
    unsafe {
        sys::ufbxw_set_name_len(scene, id, name.as_ptr().cast(), name.len());
    }

    Ok(())
}

/// Sets an FBX node name.
///
/// # Errors
///
/// Returns [`Error`] only for API consistency; `ufbx_write` does not return
/// an error from the underlying setter.
fn set_node_name(
    scene: *mut sys::ufbxw_scene,
    node: sys::ufbxw_node,
    name: &str,
) -> Result<(), Error> {
    set_element_name(scene, node.id, name)
}

/// Sets a skeleton reference-pose transform.
///
/// # Errors
///
/// This function does not fail because the underlying transform setters do
/// not return status values.
fn set_node_transform(
    scene: *mut sys::ufbxw_scene,
    node: sys::ufbxw_node,
    transform: &QsTransform,
) -> Result<(), Error> {
    unsafe {
        sys::ufbxw_node_set_translation(scene, node, to_sys_vec3(transform.transition.clone()));

        sys::ufbxw_node_set_scaling(scene, node, to_sys_vec3(transform.scale.clone()));

        sys::ufbxw_node_set_rotation_quat(
            scene,
            node,
            to_sys_rotation(&transform.quaternion),
            sys::ufbxw_rotation_order_UFBXW_ROTATION_ORDER_XYZ,
        );
    }

    Ok(())
}

/// Sets a NIF node transform.
///
/// # Errors
///
/// Returns [`Error`] if the NIF rotation matrix contains non-finite values.
fn set_nif_node_transform(
    scene: *mut sys::ufbxw_scene,
    node: sys::ufbxw_node,
    transform: &Node,
) -> Result<(), Error> {
    let rotation = nif_matrix_to_quaternion(&transform.rotation)?;

    unsafe {
        sys::ufbxw_node_set_translation(
            scene,
            node,
            sys::ufbxw_vec3 {
                x: transform.translation.x as f64,
                y: transform.translation.y as f64,
                z: transform.translation.z as f64,
            },
        );

        sys::ufbxw_node_set_scaling(
            scene,
            node,
            sys::ufbxw_vec3 {
                x: transform.scale as f64,
                y: transform.scale as f64,
                z: transform.scale as f64,
            },
        );

        sys::ufbxw_node_set_rotation_quat(
            scene,
            node,
            rotation,
            sys::ufbxw_rotation_order_UFBXW_ROTATION_ORDER_XYZ,
        );
    }

    Ok(())
}

/// Sets the skeleton hierarchy.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if a parent index is invalid.
fn set_parent_nodes(
    scene: *mut sys::ufbxw_scene,
    skeleton: &Skeleton,
    nodes: &[sys::ufbxw_node],
) -> Result<(), Error> {
    for (index, bone) in skeleton.bones.iter().enumerate() {
        if bone.parent_index < 0 {
            continue;
        }

        let parent_index = usize::try_from(bone.parent_index).map_err(|_| Error::ExportFbx {
            message: format!(
                "bone {} has invalid parent index {}",
                index, bone.parent_index
            ),
        })?;

        let parent = nodes
            .get(parent_index)
            .copied()
            .ok_or_else(|| Error::ExportFbx {
                message: format!("bone {} has invalid parent index {}", index, parent_index),
            })?;

        unsafe {
            sys::ufbxw_node_set_parent(scene, nodes[index], parent);
        }
    }

    Ok(())
}

/// Configures the FBX scene for Blender.
///
/// # Errors
///
/// This function does not fail because the underlying API has no return value.
fn set_scene_config_for_blender(scene: *mut sys::ufbxw_scene, fps: f32) {
    const AXES: sys::ufbxw_coordinate_axes = sys::ufbxw_coordinate_axes {
        right: sys::ufbxw_coordinate_axis_UFBXW_COORDINATE_AXIS_POSITIVE_X,
        up: sys::ufbxw_coordinate_axis_UFBXW_COORDINATE_AXIS_POSITIVE_Z,
        front: sys::ufbxw_coordinate_axis_UFBXW_COORDINATE_AXIS_NEGATIVE_Y,
    };

    unsafe {
        sys::ufbxw_scene_set_coordinate_axes(scene, AXES);

        sys::ufbxw_scene_set_unit_scale_factor(scene, 10.0);

        sys::ufbxw_scene_set_custom_frame_rate(scene, fps as f64);
    }
}

/// Creates animation data for every skeleton node.
///
/// # Errors
///
/// Returns [`Error`] if an animation property cannot be created or if a
/// sampled frame does not contain the expected transform.
fn create_animation(
    scene: *mut sys::ufbxw_scene,
    nodes: &[sys::ufbxw_node],
    animation: &Animation,
) -> Result<(), Error> {
    #[cfg(feature = "tracing")]
    tracing::debug!(
        duration = animation.duration,
        num_frames = animation.num_frames,
        actual_frames = animation.frames.len(),
        num_tracks = animation.num_tracks,
        "creating FBX animation"
    );

    fn seconds_to_ktime(seconds: f32) -> i64 {
        (seconds as f64 * 46_186_158_000.0).round() as i64
    }

    fn frame_time(duration: f32, frame_count: usize, frame_index: usize) -> i64 {
        if frame_count <= 1 {
            return 0;
        }
        let seconds = duration as f64 * frame_index as f64 / (frame_count - 1) as f64;
        (seconds * 46_186_158_000.0).round() as i64
    }

    let stack = unsafe { sys::ufbxw_create_anim_stack(scene) };
    let layer = unsafe { sys::ufbxw_create_anim_layer(scene, stack) };

    if stack.id == 0 || layer.id == 0 {
        return Err(Error::ExportFbx {
            message: "failed to create FBX animation stack or layer".to_owned(),
        });
    }

    let duration = seconds_to_ktime(animation.duration);

    unsafe {
        sys::ufbxw_anim_stack_set_time_range(scene, stack, 0, duration);
        sys::ufbxw_anim_stack_set_reference_time_range(scene, stack, 0, duration);
        sys::ufbxw_set_active_anim_stack(scene, stack);
    }

    for (bone_index, &node) in nodes.iter().enumerate() {
        let translation = unsafe { sys::ufbxw_node_animate_translation(scene, node, layer) };
        let rotation = unsafe { sys::ufbxw_node_animate_rotation(scene, node, layer) };
        let scaling = unsafe { sys::ufbxw_node_animate_scaling(scene, node, layer) };

        if translation.id == 0 || rotation.id == 0 || scaling.id == 0 {
            return Err(Error::ExportFbx {
                message: format!(
                    "failed to create animation properties for bone {}",
                    bone_index
                ),
            });
        }

        for (frame_index, frame) in animation.frames.iter().enumerate() {
            let time = frame_time(animation.duration, animation.frames.len(), frame_index);

            let transform = frame
                .transforms
                .get(bone_index)
                .ok_or_else(|| Error::ExportFbx {
                    message: format!(
                        "animation frame {} does not contain transform {}",
                        frame_index, bone_index
                    ),
                })?;

            unsafe {
                sys::ufbxw_anim_add_keyframe_vec3(
                    scene,
                    translation,
                    time,
                    to_sys_vec3(transform.transition.clone()),
                    sys::ufbxw_keyframe_type_UFBXW_KEYFRAME_LINEAR as u32,
                );

                sys::ufbxw_anim_add_keyframe_vec3(
                    scene,
                    rotation,
                    time,
                    quaternion_to_euler(&transform.quaternion),
                    sys::ufbxw_keyframe_type_UFBXW_KEYFRAME_LINEAR as u32,
                );

                sys::ufbxw_anim_add_keyframe_vec3(
                    scene,
                    scaling,
                    time,
                    sys::ufbxw_vec3 {
                        x: transform.scale.x as f64,
                        y: transform.scale.y as f64,
                        z: transform.scale.z as f64,
                    },
                    sys::ufbxw_keyframe_type_UFBXW_KEYFRAME_LINEAR as u32,
                );
            }
        }

        unsafe {
            sys::ufbxw_anim_finish_keyframes(scene, translation);

            sys::ufbxw_anim_finish_keyframes(scene, rotation);

            sys::ufbxw_anim_finish_keyframes(scene, scaling);
        }
    }

    Ok(())
}

/// Converts a flat vec3 array into an FBX buffer.
///
/// # Errors
///
/// Returns [`Error`] when the number of source values is not divisible by 3.
fn vec3_buffer(
    scene: *mut sys::ufbxw_scene,
    data: &[f32],
    mesh_name: &str,
    field: &str,
) -> Result<sys::ufbxw_vec3_buffer, Error> {
    if !data.len().is_multiple_of(3) {
        return Err(Error::ExportFbx {
            message: format!(
                "mesh {:?} has malformed {} data: {} values",
                mesh_name,
                field,
                data.len()
            ),
        });
    }

    let values: Vec<_> = data
        .as_chunks::<3>()
        .0
        .iter()
        .map(|v| sys::ufbxw_vec3 {
            x: v[0] as f64,
            y: v[1] as f64,
            z: v[2] as f64,
        })
        .collect();

    Ok(unsafe { sys::ufbxw_copy_vec3_array(scene, values.as_ptr(), values.len()) })
}

/// Converts a flat vec2 array into an FBX buffer.
///
/// # Errors
///
/// Returns [`Error`] when the number of source values is not divisible by 2.
fn vec2_buffer(
    scene: *mut sys::ufbxw_scene,
    data: &[f32],
    mesh_name: &str,
    field: &str,
) -> Result<sys::ufbxw_vec2_buffer, Error> {
    if !data.len().is_multiple_of(2) {
        return Err(Error::ExportFbx {
            message: format!(
                "mesh {:?} has malformed {} data: {} values",
                mesh_name,
                field,
                data.len()
            ),
        });
    }

    let values: Vec<_> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|v| sys::ufbxw_vec2 {
            x: v[0] as f64,
            y: v[1] as f64,
        })
        .collect();

    Ok(unsafe { sys::ufbxw_copy_vec2_array(scene, values.as_ptr(), values.len()) })
}

/// Converts u32 indices into an FBX int buffer.
///
/// # Errors
///
/// Returns [`Error`] if an index does not fit into `i32`.
fn int_buffer(
    scene: *mut sys::ufbxw_scene,
    data: &[u32],
    mesh_name: &str,
    field: &str,
) -> Result<sys::ufbxw_int_buffer, Error> {
    let mut values = Vec::with_capacity(data.len());

    for &value in data {
        let value = i32::try_from(value).map_err(|_| Error::ExportFbx {
            message: format!(
                "mesh {:?} has {} value {} outside i32 range",
                mesh_name, field, value
            ),
        })?;

        values.push(value);
    }

    Ok(unsafe { sys::ufbxw_copy_int_array(scene, values.as_ptr(), values.len()) })
}

/// Converts a NIF rotation matrix into an FBX quaternion.
///
/// # Errors
///
/// Returns [`Error`] when the matrix contains non-finite values or cannot be
/// converted into a valid quaternion.
fn nif_matrix_to_quaternion(matrix: &Matrix3) -> Result<sys::ufbxw_quat, Error> {
    let values = [
        matrix.x.x, matrix.x.y, matrix.x.z, matrix.y.x, matrix.y.y, matrix.y.z, matrix.z.x,
        matrix.z.y, matrix.z.z,
    ];

    if values.iter().any(|value| !value.is_finite()) {
        return Err(Error::ExportFbx {
            message: "NIF rotation matrix contains a non-finite value".to_owned(),
        });
    }

    let m00 = matrix.x.x as f64;
    let m01 = matrix.x.y as f64;
    let m02 = matrix.x.z as f64;

    let m10 = matrix.y.x as f64;
    let m11 = matrix.y.y as f64;
    let m12 = matrix.y.z as f64;

    let m20 = matrix.z.x as f64;
    let m21 = matrix.z.y as f64;
    let m22 = matrix.z.z as f64;

    let trace = m00 + m11 + m22;

    let (x, y, z, w);

    if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;

        w = 0.25 * s;
        x = (m21 - m12) / s;
        y = (m02 - m20) / s;
        z = (m10 - m01) / s;
    } else if m00 > m11 && m00 > m22 {
        let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;

        w = (m21 - m12) / s;
        x = 0.25 * s;
        y = (m01 + m10) / s;
        z = (m02 + m20) / s;
    } else if m11 > m22 {
        let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;

        w = (m02 - m20) / s;
        x = (m01 + m10) / s;
        y = 0.25 * s;
        z = (m12 + m21) / s;
    } else {
        let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;

        w = (m10 - m01) / s;
        x = (m02 + m20) / s;
        y = (m12 + m21) / s;
        z = 0.25 * s;
    }

    let length = w.mul_add(w, z.mul_add(z, y.mul_add(y, x * x))).sqrt();

    if !length.is_finite() || length <= f64::EPSILON {
        return Err(Error::ExportFbx {
            message: "failed to convert NIF rotation matrix to quaternion".to_owned(),
        });
    }

    Ok(sys::ufbxw_quat {
        x: x / length,
        y: y / length,
        z: z / length,
        w: w / length,
    })
}

/// Converts a NIF 3x3 matrix to an FBX 4x4 matrix.
///
/// The current NIF FFI representation exposes only the 3x3 matrix.
///
/// # Errors
///
/// This function does not return an error.
const fn nif_matrix_to_ufbx(matrix: &Matrix4) -> sys::ufbxw_matrix {
    let mut result = unsafe { std::mem::zeroed::<sys::ufbxw_matrix>() };

    unsafe {
        result.__bindgen_anon_1.m[0] = matrix.m[0] as f64;
        result.__bindgen_anon_1.m[1] = matrix.m[1] as f64;
        result.__bindgen_anon_1.m[2] = matrix.m[2] as f64;

        result.__bindgen_anon_1.m[4] = matrix.m[4] as f64;
        result.__bindgen_anon_1.m[5] = matrix.m[5] as f64;
        result.__bindgen_anon_1.m[6] = matrix.m[6] as f64;

        result.__bindgen_anon_1.m[8] = matrix.m[8] as f64;
        result.__bindgen_anon_1.m[9] = matrix.m[9] as f64;
        result.__bindgen_anon_1.m[10] = matrix.m[10] as f64;

        result.__bindgen_anon_1.m[15] = 1.0;
    };

    result
}

/// Converts a Havok vector into an FBX vector.
///
/// # Errors
///
/// This function does not return an error.
const fn to_sys_vec3(value: Vector4) -> sys::ufbxw_vec3 {
    sys::ufbxw_vec3 {
        x: value.x as f64,
        y: value.y as f64,
        z: value.z as f64,
    }
}

/// Converts a Havok quaternion into an FBX quaternion.
///
/// # Errors
///
/// This function does not return an error.
const fn to_sys_rotation(rotation: &Quaternion) -> sys::ufbxw_quat {
    sys::ufbxw_quat {
        x: rotation.x as f64,
        y: rotation.y as f64,
        z: rotation.z as f64,
        w: rotation.scaler as f64,
    }
}

/// Converts a quaternion to XYZ Euler angles in degrees.
fn quaternion_to_euler(rotation: &Quaternion) -> sys::ufbxw_vec3 {
    unsafe {
        sys::ufbxw_quat_to_euler(
            to_sys_rotation(rotation),
            sys::ufbxw_rotation_order_UFBXW_ROTATION_ORDER_XYZ,
        )
    }
}

/// Saves the prepared scene as FBX.
///
/// # Errors
///
/// Returns [`Error::ExportFbx`] if `ufbx_write` fails to serialize the scene
/// or returns an invalid output buffer.
fn save_memory(scene: *mut sys::ufbxw_scene, format: Format) -> Result<Vec<u8>, Error> {
    let mut opts = unsafe { std::mem::zeroed::<sys::ufbxw_save_opts>() };

    opts.format = match format {
        Format::FbxBin => sys::ufbxw_save_format_UFBXW_SAVE_FORMAT_BINARY,
        Format::FbxAscii => sys::ufbxw_save_format_UFBXW_SAVE_FORMAT_ASCII,
    };

    opts.version = 7500;

    let mut buffer = unsafe { std::mem::zeroed::<sys::ufbxw_write_buffer>() };

    let mut error = unsafe { std::mem::zeroed::<sys::ufbxw_error>() };

    if !unsafe { sys::ufbxw_save_memory(scene, &mut buffer, &opts, &mut error) } {
        return Err(Error::ExportFbx {
            message: sys_error::Error::from(error).to_string(),
        });
    }

    if buffer.data.is_null() && buffer.size != 0 {
        return Err(Error::ExportFbx {
            message: "ufbxw_save_memory() returned an invalid buffer".to_owned(),
        });
    }

    let data =
        unsafe { std::slice::from_raw_parts(buffer.data.cast::<u8>(), buffer.size).to_vec() };

    unsafe {
        sys::ufbxw_free_write_buffer(buffer);
    }

    Ok(data)
}

/// Creates default scene options.
///
/// # Errors
///
/// This function does not return an error.
const fn scene_options() -> sys::ufbxw_scene_opts {
    unsafe { std::mem::zeroed() }
}

/// Validates animation data.
///
/// # Errors
///
/// Returns [`Error`] if the animation track count, frame count, or transform
/// count is inconsistent with the skeleton.
fn validate_animation(skeleton: &Skeleton, animation: &Animation) -> Result<(), Error> {
    let bone_count = skeleton.bones.len();

    if animation.num_tracks as usize > bone_count {
        return Err(Error::InvalidTrackCount {
            actual: animation.num_tracks as usize,
            maximum: bone_count,
        });
    }

    if animation.num_frames as usize != animation.frames.len() {
        return Err(Error::EncoderFrameCountMismatch {
            expected: animation.num_frames as usize,
            actual: animation.frames.len(),
        });
    }

    for (frame_index, frame) in animation.frames.iter().enumerate() {
        if frame.transforms.len() != bone_count {
            return Err(Error::EncoderTransformCountMismatch {
                frame_index,
                expected: bone_count,
                actual: frame.transforms.len(),
            });
        }
    }

    Ok(())
}
