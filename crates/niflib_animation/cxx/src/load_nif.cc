#include "niflib_animation.h"

#include <niflib.h>

#include <obj/NiAVObject.h>
#include <obj/NiGeometry.h>
#include <obj/NiNode.h>
#include <obj/NiTriBasedGeom.h>
#include <obj/NiTriShape.h>
#include <obj/NiTriStrips.h>
#include <obj/NiTriStripsData.h>

#include <obj/NiGeometryData.h>

#include <obj/NiSkinData.h>
#include <obj/NiSkinInstance.h>
#include <obj/NiSkinPartition.h>

#include <obj/BSEffectShaderProperty.h>
#include <obj/BSLightingShaderProperty.h>
#include <obj/BSShaderTextureSet.h>

#include <cstdint>
#include <stdexcept>
#include <string>
#include <unordered_map>
#include <utility>
#include <vector>

namespace niflib_animation {

namespace {

using namespace Niflib;

struct ExtractContext {
  NifScene scene;

  std::unordered_map<const NiAVObject *, int32_t> node_indices;
  std::unordered_map<const NiProperty *, int32_t> material_indices;
  std::unordered_map<const NiSkinInstance *, int32_t> skin_indices;
  std::unordered_map<const BSShaderTextureSet *, int32_t> texture_indices;
};

void append_vec3(rust::Vec<float> &output, const Niflib::Vector3 &value) {
  output.emplace_back(value.x);
  output.emplace_back(value.y);
  output.emplace_back(value.z);
}

void append_vec4(rust::Vec<float> &output, const Niflib::Vector4 &value) {
  output.emplace_back(value.x);
  output.emplace_back(value.y);
  output.emplace_back(value.z);
  output.emplace_back(value.w);
}

Matrix3 to_matrix3(Niflib::Matrix33 rotation) {
  return {
      .x = {rotation[0][0], rotation[0][1], rotation[0][2]},
      .y = {rotation[1][0], rotation[0][1], rotation[1][2]},
      .z = {rotation[2][0], rotation[2][1], rotation[2][2]},
  };
}

int32_t add_node(ExtractContext &context, const NiAVObjectRef &object,
                 int32_t parent) {
  auto existing = context.node_indices.find(object);

  if (existing != context.node_indices.end()) {
    return existing->second;
  }

  NifNode node;

  node.name = object->GetName();

  const auto transform = object->GetLocalTransform();
  const auto translation = transform.GetTranslation();

  node.translation = {
      .x = translation.x, .y = translation.y, .z = translation.z};

  node.rotation = to_matrix3(transform.GetRotation());

  node.scale = transform.GetScale();
  node.parent = parent;

  const auto index = static_cast<int32_t>(context.scene.nodes.size());

  context.scene.nodes.emplace_back(std::move(node));
  context.node_indices.emplace(object, index);

  return index;
}

int32_t find_or_add_texture(ExtractContext &context,
                            const BSShaderTextureSetRef &texture_set) {
  if (!texture_set) {
    return -1;
  }

  auto existing = context.texture_indices.find(texture_set);

  if (existing != context.texture_indices.end()) {
    return existing->second;
  }

  NifTexture texture;

  const auto textures = texture_set->GetTextures();

  if (textures.size() > 0) {
    texture.diffuse = textures[0];
  }

  if (textures.size() > 1) {
    texture.normal = textures[1];
  }

  if (textures.size() > 2) {
    texture.glow = textures[2];
  }

  if (textures.size() > 3) {
    texture.specular = textures[3];
  }

  const auto index = static_cast<int32_t>(context.scene.textures.size());

  context.scene.textures.emplace_back(std::move(texture));

  context.texture_indices.emplace(texture_set, index);

  return index;
}

int32_t find_or_add_material(ExtractContext &context,
                             const NiGeometryRef &geometry) {
  const auto property = geometry->GetBSProperty(0);

  if (!property) {
    return -1;
  }

  auto existing = context.material_indices.find(property);

  if (existing != context.material_indices.end()) {
    return existing->second;
  }

  NifMaterial material;
  material.name = geometry->GetName();
  material.texture = -1;

  if (property->IsDerivedType(BSLightingShaderProperty::TYPE)) {
    auto shader = DynamicCast<BSLightingShaderProperty>(property);

    if (shader) {
      auto texture_set = shader->GetTextureSet();

      material.texture = find_or_add_texture(context, texture_set);
    }
  }

  const auto index = static_cast<int32_t>(context.scene.materials.size());

  context.scene.materials.emplace_back(std::move(material));

  context.material_indices.emplace(property, index);

  return index;
}

void copy_geometry_data(const NiGeometryDataRef &data, NifMesh &mesh) {
  if (!data) {
    return;
  }

  const auto &vertices = data->GetVertices();

  mesh.positions.reserve(vertices.size() * 3);

  for (const auto &vertex : vertices) {
    append_vec3(mesh.positions, vertex);
  }

  const auto &normals = data->GetNormals();

  mesh.normals.reserve(normals.size() * 3);

  for (const auto &normal : normals) {
    append_vec3(mesh.normals, normal);
  }

  const auto uv_sets_count = data->GetUVSetCount();

  if (uv_sets_count != 0) {
    const auto &uvs = data->GetUVSet(0);

    mesh.uvs.reserve(uvs.size() * 2);

    for (const auto &uv : uvs) {
      mesh.uvs.emplace_back(uv.u);
      mesh.uvs.emplace_back(uv.v);
    }
  }
}

void copy_triangles(const NiTriBasedGeomRef &geometry, NifMesh &mesh) {
  if (!geometry) {
    return;
  }

  const auto data = geometry->GetData();

  if (!data) {
    return;
  }

  const auto tri_data = DynamicCast<NiTriBasedGeomData>(data);

  if (!tri_data) {
    return;
  }

  const auto triangles = tri_data->GetTriangles();

  mesh.indices.reserve(mesh.indices.size() + triangles.size() * 3);

  for (const auto &triangle : triangles) {
    mesh.indices.emplace_back(triangle.v1);
    mesh.indices.emplace_back(triangle.v2);
    mesh.indices.emplace_back(triangle.v3);
  }
}

int32_t copy_skin(ExtractContext &context, const NiTriBasedGeomRef &geometry) {
  const auto skin_instance = geometry->GetSkinInstance();

  if (!skin_instance) {
    return -1;
  }

  auto existing = context.skin_indices.find(skin_instance);

  if (existing != context.skin_indices.end()) {
    return existing->second;
  }

  NifSkin skin;

  const auto bones = skin_instance->GetBones();

  skin.bones.reserve(bones.size());

  for (const auto &bone : bones) {
    if (bone) {
      skin.bones.emplace_back(bone->GetName());
    } else {
      skin.bones.emplace_back("");
    }
  }

  const auto skin_data = skin_instance->GetSkinData();

  if (skin_data) {
    const auto bone_count = skin_data->GetBoneCount();

    skin.bind_matrices.reserve(bone_count * 16);

    for (auto i = 0; i < bone_count; i++) {
      const auto &bone = skin_data->GetBoneTransform(i);

      const auto rotation = bone.GetRotation();
      const auto translation = bone.GetTranslation();
      const auto scale = bone.GetScale();

      const float r00 = rotation[0][0] * scale;
      const float r01 = rotation[0][1] * scale;
      const float r02 = rotation[0][2] * scale;

      const float r10 = rotation[1][0] * scale;
      const float r11 = rotation[1][1] * scale;
      const float r12 = rotation[1][2] * scale;

      const float r20 = rotation[2][0] * scale;
      const float r21 = rotation[2][1] * scale;
      const float r22 = rotation[2][2] * scale;

      skin.bind_matrices.push_back({
          .x = {r00, r01, r02},
          .y = {r10, r11, r12},
          .z = {r20, r21, r22},
      });
    }
  }

  /*
   * The NiSkinPartition contains the data needed for rendering:
   *
   *   vertexMap
   *   boneIndices
   *   vertexWeights
   *   triangles
   *
   * A vertex in a partition does not necessarily correspond directly
   * to the original NiGeometryData vertex. vertexMap performs that
   * mapping.
   *
   * We flatten the partition data into four influences per original
   * vertex. This is the representation expected by the Rust scene.
   */
  auto skin_partition = skin_instance->GetSkinPartition();

  if (!skin_partition && skin_data) {
    skin_partition = skin_data->GetSkinPartition();
  }

  if (skin_partition) {
    size_t vertex_count = 0;

    const int partition_count = skin_partition->GetNumPartitions();

    for (int partition = 0; partition < partition_count; ++partition) {
      const auto vertex_map = skin_partition->GetVertexMap(partition);

      for (const auto vertex : vertex_map) {
        vertex_count = std::max(vertex_count, static_cast<size_t>(vertex) + 1);
      }
    }

    skin.bone_indices.truncate(vertex_count * 4);
    skin.bone_weights.truncate(vertex_count * 4);

    for (int partition = 0; partition < partition_count; ++partition) {
      const auto vertex_map = skin_partition->GetVertexMap(partition);

      const auto bone_map = skin_partition->GetBoneMap(partition);

      const int partition_vertex_count =
          skin_partition->GetNumVertices(partition);

      for (int vertex = 0; vertex < partition_vertex_count; ++vertex) {

        if (static_cast<size_t>(vertex) >= vertex_map.size()) {
          continue;
        }

        const auto absolute_vertex = static_cast<size_t>(vertex_map[vertex]);

        if (absolute_vertex >= vertex_count) {
          continue;
        }

        const auto local_bones =
            skin_partition->GetVertexBoneIndices(partition, vertex);

        const auto weights =
            skin_partition->GetVertexWeights(partition, vertex);

        struct Influence {
          uint16_t bone;
          float weight;
        };

        std::vector<Influence> influences;

        const auto count = std::min(local_bones.size(), weights.size());

        influences.reserve(count);

        for (size_t i = 0; i < count; ++i) {
          if (weights[i] <= 0.001f) {
            continue;
          }

          const auto local_bone = static_cast<size_t>(local_bones[i]);

          if (local_bone >= bone_map.size()) {
            continue;
          }

          const auto absolute_bone =
              static_cast<uint16_t>(bone_map[local_bone]);

          influences.push_back({absolute_bone, weights[i]});
        }

        std::sort(influences.begin(), influences.end(),
                  [](const Influence &lhs, const Influence &rhs) {
                    return lhs.weight > rhs.weight;
                  });

        if (influences.size() > 4) {
          influences.resize(4);
        }

        float weight_sum = 0.0f;

        for (const auto &influence : influences) {
          weight_sum += influence.weight;
        }

        for (size_t i = 0; i < influences.size(); ++i) {

          skin.bone_indices[absolute_vertex * 4 + i] = influences[i].bone;

          skin.bone_weights[absolute_vertex * 4 + i] =
              weight_sum > 0.0f ? influences[i].weight / weight_sum : 0.0f;
        }
      }
    }
  }

  const auto index = static_cast<int32_t>(context.scene.skins.size());

  context.scene.skins.emplace_back(std::move(skin));

  context.skin_indices.emplace(skin_instance, index);

  return index;
}

void extract_geometry(ExtractContext &context,
                      const NiTriBasedGeomRef &geometry, int32_t node) {
  const auto data = geometry->GetData();

  if (!data) {
    return;
  }

  NifMesh mesh;

  mesh.name = geometry->GetName();
  mesh.node = node;
  mesh.material =
      find_or_add_material(context, DynamicCast<NiGeometry>(geometry));

  mesh.skin = copy_skin(context, geometry);

  copy_geometry_data(data, mesh);
  copy_triangles(geometry, mesh);

  context.scene.meshes.emplace_back(std::move(mesh));
}

void visit_object(ExtractContext &context, const NiAVObjectRef &object,
                  int32_t parent) {
  if (!object) {
    return;
  }

  const auto node = add_node(context, object, parent);

  if (object->IsDerivedType(NiTriBasedGeom::TYPE)) {
    auto geometry = DynamicCast<NiTriBasedGeom>(object);

    if (geometry) {
      extract_geometry(context, geometry, node);
    }
  }

  if (object->IsDerivedType(NiNode::TYPE)) {
    auto ni_node = DynamicCast<NiNode>(object);

    if (!ni_node) {
      return;
    }

    for (const auto &child : ni_node->GetChildren()) {

      visit_object(context, child, node);
    }
  }
}

} // namespace

NifScene load_nif(rust::Str path) {
  const std::string filename(path);

  /*
   * Niflib owns the NIF object graph.
   * We immediately flatten the graph into NifScene so no Niflib
   * object crosses the CXX boundary.
   */
  auto info = new_nif_info();
  auto root = ReadNifTree(filename, &info);

  if (!root) {
    throw std::runtime_error("NIF has no root object: " + filename);
  }

  const auto object = DynamicCast<NiAVObject>(root);
  if (!object) {
    throw std::runtime_error("NIF root is not a NiAVObject");
  }

  ExtractContext context;
  visit_object(context, object, -1);

  return std::move(context.scene);
}

} // namespace niflib_animation
