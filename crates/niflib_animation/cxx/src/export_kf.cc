// SPDX-FileCopyrightText: Copyright (c) 2011 figment
// SPDX-License-Identifier: BSD-3-Clause
//
// https://github.com/jgernandt/hkxcmd/tree/51260f796d1d19255936b2e35f33fa752391083a/Addins/ExportKF.cpp
#include "obj/NiObjectNET.h" // IWYU pragma: keep

#include <niflib.h>
#include <obj/NiControllerSequence.h> // NOTE: Need NiObjectNET.h
#include <obj/NiInterpolator.h>
#include <obj/NiTextKeyExtraData.h>
#include <obj/NiTransformData.h>
#include <obj/NiTransformInterpolator.h>

#include "niflib_animation.h"

namespace niflib_animation {

namespace {

Niflib::Vector3 to_vector3(const Vec4 &value) {
  return Niflib::Vector3(value.x, value.y, value.z);
}

Niflib::Quaternion to_quaternion(const Quaternion &value) {
  // FFI: x/y/z/w
  // Niflib: w/x/y/z
  return Niflib::Quaternion(value.w, value.x, value.y, value.z);
}

Niflib::IndexString to_index_string(const rust::String &value) {
  return {std::string{std::string_view{
      value.data(), // NOTE: non terminated null
      value.size(),
  }}};
}

Niflib::NiTransformDataRef make_transform_data(const Skeleton &skeleton,
                                               const Animation &animation,
                                               std::size_t bone_index) {
  const std::size_t frame_count = animation.frames.size();

  if (frame_count == 0) {
    throw std::runtime_error("animation contains no frames");
  }

  if (bone_index >= skeleton.bones.size()) {
    throw std::runtime_error("bone index exceeds skeleton bone count");
  }

  const Bone &bone = skeleton.bones[bone_index];

  std::vector<Niflib::Key<Niflib::Vector3>> translate_keys;
  std::vector<Niflib::Key<Niflib::Quaternion>> rotate_keys;
  std::vector<Niflib::Key<float>> scale_keys;

  translate_keys.reserve(frame_count);
  rotate_keys.reserve(frame_count);
  scale_keys.reserve(frame_count);

  const float frame_time =
      frame_count > 1 ? animation.duration / static_cast<float>(frame_count - 1)
                      : 0.0f;

  for (std::size_t frame_index = 0; frame_index < frame_count; ++frame_index) {
    const AnimationFrame &frame = animation.frames[frame_index];

    if (frame.transforms.size() != skeleton.bones.size()) {
      throw std::runtime_error("animation frame transform count does not match "
                               "skeleton bone count");
    }

    const Transform &transform = frame.transforms[bone_index];

    Niflib::Key<Niflib::Vector3> translation;
    translation.time = static_cast<float>(frame_index) * frame_time;
    translation.data = to_vector3(transform.translation);

    translate_keys.push_back(translation);

    Niflib::Key<Niflib::Quaternion> rotation;
    rotation.time = static_cast<float>(frame_index) * frame_time;
    rotation.data = to_quaternion(transform.rotation);

    rotate_keys.push_back(rotation);

    Niflib::Key<float> scale;
    scale.time = static_cast<float>(frame_index) * frame_time;

    /*
     * Niflib's NiTransformData stores scale as a scalar.
     *
     * The Rust FFI type intentionally uses Vec4 because it mirrors
     * the Havok transform representation. KF cannot preserve
     * independent XYZ scale components here.
     */
    scale.data = transform.scale.x;
    scale_keys.push_back(scale);
  }

  Niflib::NiTransformDataRef data = new Niflib::NiTransformData();

  data->SetTranslateType(Niflib::LINEAR_KEY);
  data->SetRotateType(Niflib::QUADRATIC_KEY);
  data->SetScaleType(Niflib::LINEAR_KEY);

  data->SetTranslateKeys(translate_keys);
  data->SetQuatRotateKeys(rotate_keys);
  data->SetScaleKeys(scale_keys);

  return data;
}

Niflib::NiControllerSequenceRef make_animation(const Skeleton &skeleton,
                                               const Animation &animation) {
  if (animation.frames.empty()) {
    throw std::runtime_error("animation contains no frames");
  }

  if (animation.num_frames != animation.frames.size()) {
    throw std::runtime_error(
        "animation num_frames does not match frames.size()");
  }

  if (animation.num_tracks > skeleton.bones.size()) {
    throw std::runtime_error(
        "animation track count exceeds skeleton bone count");
  }

  for (const AnimationFrame &frame : animation.frames) {
    if (frame.transforms.size() != skeleton.bones.size()) {
      throw std::runtime_error("animation frame transform count does not match "
                               "skeleton bone count");
    }
  }

  Niflib::NiControllerSequenceRef sequence = new Niflib::NiControllerSequence();

  sequence->SetName("animation");
  sequence->SetStartTime(0.0f);
  sequence->SetStopTime(animation.duration);
  sequence->SetFrequency(1.0f);
  sequence->SetCycleType(Niflib::CYCLE_CLAMP);

  std::vector<Niflib::ControllerLink> links;
  links.reserve(skeleton.bones.size());

  for (std::size_t bone_index = 0; bone_index < skeleton.bones.size();
       ++bone_index) {
    const Bone &bone = skeleton.bones[bone_index];

    Niflib::NiTransformInterpolatorRef interpolator =
        new Niflib::NiTransformInterpolator();

    interpolator->SetTranslation(to_vector3(bone.reference_pose.translation));
    interpolator->SetRotation(to_quaternion(bone.reference_pose.rotation));
    interpolator->SetScale(bone.reference_pose.scale.x);

    interpolator->SetData(make_transform_data(skeleton, animation, bone_index));

    Niflib::ControllerLink link;
    link.nodeName = to_index_string(bone.name);
    link.interpolator = interpolator;
    link.priority = 0;

    links.push_back(link);
  }

  sequence->SetControlledBlocks(links);

  if (!animation.annotations.empty()) {
    Niflib::NiTextKeyExtraDataRef text_keys = new Niflib::NiTextKeyExtraData();

    std::vector<Niflib::Key<std::string>> keys;
    keys.reserve(animation.annotations.size());

    for (const AnimationAnnotation &annotation : animation.annotations) {
      Niflib::Key<std::string> key;
      key.time = annotation.time;
      key.data = to_index_string(annotation.text);

      keys.push_back(key);
    }

    text_keys->SetKeys(keys);
    sequence->SetTextKey(text_keys);
  }

  return sequence;
}

std::vector<std::uint8_t> write_kf(Niflib::NiControllerSequence *sequence) {
  if (sequence == nullptr) {
    throw std::runtime_error("cannot write a null NiControllerSequence");
  }

  std::ostringstream stream(std::ios::out | std::ios::binary);
  Niflib::WriteNifTree(stream, sequence, new_nif_info());
  const std::string bytes = stream.str();

  return {bytes.begin(), bytes.end()};
}

} // namespace

rust::Vec<std::uint8_t> export_kf(const Kf &input) {
  if (input.skeleton.bones.empty()) {
    throw std::runtime_error("cannot convert KF with an empty skeleton");
  }

  // The current niflib KF writer accepts one root object.
  auto sequence = make_animation(input.skeleton, input.animation);
  const std::vector<std::uint8_t> bytes = write_kf(sequence);

  // to rust vec
  rust::Vec<std::uint8_t> result;
  result.reserve(bytes.size());
  for (std::uint8_t byte : bytes) {
    result.push_back(byte);
  }

  return result;
}

} // namespace niflib_animation
