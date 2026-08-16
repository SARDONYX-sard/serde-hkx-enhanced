// SPDX-FileCopyrightText: Copyright (c) 2011 figment
// SPDX-License-Identifier: BSD-3-Clause
//
// https://github.com/jgernandt/hkxcmd/tree/51260f796d1d19255936b2e35f33fa752391083a/Addins/ConvertKF.cpp

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
using Vector3Key = Niflib::Key<Niflib::Vector3>;
using QuaternionKey = Niflib::Key<Niflib::Quaternion>;
using FloatKey = Niflib::Key<float>;

struct TransformKeys {
  std::vector<Vector3Key> translation;
  std::vector<QuaternionKey> rotation;
  std::vector<FloatKey> scale;

  Niflib::KeyType translation_type;
  Niflib::KeyType rotation_type;
  Niflib::KeyType scale_type;
};

struct TransformTrack {
  std::size_t bone_index;
  TransformKeys keys;
};

bool approximately_equal(float lhs, float rhs) {
  constexpr float TOLERANCE = 1.0e-5f;
  return std::fabs(lhs - rhs) < TOLERANCE;
}

Niflib::Quaternion normalize_quaternion(Niflib::Quaternion value) {
  // NOTE: Component-wise lerp between two unit quaternions does NOT
  // preserve unit length. The spline compressor (e.g. THREECOMP40)
  // assumes a unit-length input and reconstructs the dropped component
  // via sqrt(1 - a^2 - b^2 - c^2), so feeding it a non-unit quaternion
  // silently distorts the rotation angle on the next compression pass.
  const float length_sq = value.w * value.w + value.x * value.x +
                          value.y * value.y + value.z * value.z;

  if (length_sq > 1.0e-12f) {
    const float inv_length = 1.0f / std::sqrt(length_sq);
    value.w *= inv_length;
    value.x *= inv_length;
    value.y *= inv_length;
    value.z *= inv_length;
  }

  return value;
}

std::vector<Niflib::Ref<Niflib::NiObject>>
read_kf(rust::Slice<const std::uint8_t> input) {
  const char *data = reinterpret_cast<const char *>(input.data());

  std::istringstream stream(std::string(data, input.size()),
                            std::ios::in | std::ios::binary);

  std::list<Niflib::Ref<Niflib::NiObject>> missing_link_stack;
  Niflib::NifInfo info = new_nif_info();

  return Niflib::ReadNifList(stream, missing_link_stack, &info);
} // namespace

Niflib::NiControllerSequenceRef
find_sequence(const std::vector<Niflib::Ref<Niflib::NiObject>> &objects) {
  Niflib::NiControllerSequenceRef sequence;

  for (const auto &object : objects) {
    if (!object) {
      continue;
    }

    if (!object->IsDerivedType(Niflib::NiControllerSequence::TYPE)) {
      continue;
    }

    if (sequence) {
      throw std::runtime_error(
          "KF contains multiple NiControllerSequence objects");
    }

    sequence = Niflib::DynamicCast<Niflib::NiControllerSequence>(object);
  }

  if (!sequence) {
    throw std::runtime_error("KF contains no NiControllerSequence");
  }

  return sequence;
}

std::map<std::string, std::size_t> make_bone_map(const Skeleton &skeleton) {
  std::map<std::string, std::size_t> result;

  for (std::size_t i = 0; i < skeleton.bones.size(); ++i) {
    result.emplace(skeleton.bones[i].name, i);
  }

  return result;
}

std::vector<TransformTrack>
collect_tracks(const Niflib::NiControllerSequenceRef &sequence,
               const Skeleton &skeleton) {
  const auto bone_map = make_bone_map(skeleton);

  std::vector<TransformTrack> result;

  for (const auto &block : sequence->GetControlledBlocks()) {
    if (!block.interpolator) {
      continue;
    }

    if (!block.interpolator->IsSameType(
            Niflib::NiTransformInterpolator::TYPE)) {
      continue;
    }

    const auto bone = bone_map.find(block.nodeName);

    if (bone == bone_map.end()) {
      continue;
    }

    const auto interpolator =
        Niflib::StaticCast<Niflib::NiTransformInterpolator>(block.interpolator);

    const auto data = interpolator->GetData();

    if (!data) {
      continue;
    }

    TransformKeys keys{
        .translation = data->GetTranslateKeys(),
        .rotation = data->GetQuatRotateKeys(),
        .scale = data->GetScaleKeys(),

        .translation_type = data->GetTranslateType(),
        .rotation_type = data->GetRotateType(),
        .scale_type = data->GetScaleType(),
    };
    result.push_back({bone->second, std::move(keys)});
  }

  return result;
}

Niflib::Vector3 sample_vector(const std::vector<Vector3Key> &keys,
                              Niflib::KeyType type, float time,
                              std::size_t &hint) {
  if (keys.empty()) {
    return Niflib::Vector3();
  }

  while (hint < keys.size() && keys[hint].time <= time) {
    ++hint;
  }

  if (hint == 0) {
    return keys.front().data;
  }

  if (hint >= keys.size()) {
    return keys.back().data;
  }

  const auto &previous = keys[hint - 1];
  const auto &next = keys[hint];

  if (approximately_equal(previous.time, time) || type == Niflib::CONST_KEY) {
    return previous.data;
  }

  if (approximately_equal(next.time, time)) {
    return next.data;
  }

  const float interval = next.time - previous.time;

  if (approximately_equal(interval, 0.0f)) {
    return previous.data;
  }

  const float t = (time - previous.time) / interval;

  return previous.data + (next.data - previous.data) * t;
}

Niflib::Quaternion sample_quaternion(const std::vector<QuaternionKey> &keys,
                                     Niflib::KeyType type, float time,
                                     std::size_t &hint) {
  if (keys.empty()) {
    return Niflib::Quaternion();
  }

  while (hint < keys.size() && keys[hint].time <= time) {
    ++hint;
  }

  if (hint == 0) {
    return keys.front().data;
  }

  if (hint >= keys.size()) {
    return keys.back().data;
  }

  const auto &previous = keys[hint - 1];
  const auto &next = keys[hint];

  if (previous.time == time) {
    return previous.data;
  }

  if (next.time == time) {
    return next.data;
  }

  if (approximately_equal(previous.time, time) || type == Niflib::CONST_KEY) {
    return previous.data;
  }

  if (approximately_equal(next.time, time)) {
    return next.data;
  }

  const float interval = next.time - previous.time;

  if (approximately_equal(interval, 0.0f)) {
    return previous.data;
  }

  const float t = (time - previous.time) / interval;

  Niflib::Quaternion value{
      previous.data.w + (next.data.w - previous.data.w) * t,
      previous.data.x + (next.data.x - previous.data.x) * t,
      previous.data.y + (next.data.y - previous.data.y) * t,
      previous.data.z + (next.data.z - previous.data.z) * t,
  };

  return normalize_quaternion(value);
}

float sample_float(const std::vector<FloatKey> &keys, Niflib::KeyType type,
                   float time, std::size_t &hint) {
  if (keys.empty()) {
    return 1.0f;
  }

  while (hint < keys.size() && keys[hint].time <= time) {
    ++hint;
  }

  if (hint == 0) {
    return keys.front().data;
  }

  if (hint >= keys.size()) {
    return keys.back().data;
  }

  const auto &previous = keys[hint - 1];
  const auto &next = keys[hint];

  if (approximately_equal(previous.time, time) || type == Niflib::CONST_KEY) {
    return previous.data;
  }

  if (approximately_equal(next.time, time)) {
    return next.data;
  }

  if (previous.time == time) {
    return previous.data;
  }

  if (next.time == time) {
    return next.data;
  }

  const float interval = next.time - previous.time;
  if (approximately_equal(interval, 0.0f)) {
    return previous.data;
  }

  const float t = (time - previous.time) / interval;
  return previous.data + (next.data - previous.data) * t;
}

Transform to_transform(const Niflib::Vector3 &translation,
                       const Niflib::Quaternion &rotation, float scale) {
  Transform result;

  result.translation.x = translation.x;
  result.translation.y = translation.y;
  result.translation.z = translation.z;
  result.translation.w = 0.0f;

  result.rotation.x = rotation.x;
  result.rotation.y = rotation.y;
  result.rotation.z = rotation.z;
  result.rotation.w = rotation.w;

  result.scale.x = scale;
  result.scale.y = scale;
  result.scale.z = scale;
  result.scale.w = 0.0f;

  return result;
}

Transform sample_transform(const TransformTrack &track,
                           const Transform &reference, float time,
                           std::size_t &translation_hint,
                           std::size_t &rotation_hint,
                           std::size_t &scale_hint) {
  Transform result = reference;

  if (!track.keys.translation.empty()) {
    const auto value =
        sample_vector(track.keys.translation, track.keys.translation_type, time,
                      translation_hint);

    result.translation.x = value.x;
    result.translation.y = value.y;
    result.translation.z = value.z;
  }

  if (!track.keys.rotation.empty()) {
    const auto value = sample_quaternion(
        track.keys.rotation, track.keys.rotation_type, time, rotation_hint);

    result.rotation.x = value.x;
    result.rotation.y = value.y;
    result.rotation.z = value.z;
    result.rotation.w = value.w;
  }

  if (!track.keys.scale.empty()) {
    const float value =
        sample_float(track.keys.scale, track.keys.scale_type, time, scale_hint);

    result.scale.x = value;
    result.scale.y = value;
    result.scale.z = value;
  }

  return result;
}

} // namespace

Animation convert_kf(rust::Slice<const std::uint8_t> input,
                     const Skeleton &skeleton, float fps) {

  if (input.empty()) {
    throw std::runtime_error("KF input is empty");
  }

  if (skeleton.bones.empty()) {
    throw std::runtime_error("skeleton contains no bones");
  }
  if (!std::isfinite(fps) || fps <= 0.0f) {
    throw std::invalid_argument(
        "KF sampling FPS must be finite and greater than zero");
  }

  const float frame_increment = 1.0f / fps;
  const auto objects = read_kf(input);
  const auto sequence = find_sequence(objects);
  const auto tracks = collect_tracks(sequence, skeleton);

  const float start_time = sequence->GetStartTime();
  const float stop_time = sequence->GetStopTime();

  if (stop_time < start_time) {
    throw std::runtime_error(
        "KF controller sequence has an invalid time range");
  }

  const float duration = stop_time - start_time;

  const std::uint32_t num_frames =
      static_cast<std::uint32_t>(std::round(duration / frame_increment)) + 1;

  Animation animation;

  animation.duration = duration;
  animation.num_frames = num_frames;
  animation.num_tracks = static_cast<std::uint32_t>(skeleton.bones.size());

  animation.frames.reserve(num_frames);

  /*
   * Initialize every frame from the skeleton reference pose.
   *
   * This is important because KF does not necessarily contain a track for
   * every skeleton bone.
   */
  for (std::uint32_t frame_index = 0; frame_index < num_frames; ++frame_index) {

    AnimationFrame frame;

    frame.transforms.reserve(skeleton.bones.size());

    for (const auto &bone : skeleton.bones) {
      frame.transforms.push_back(bone.reference_pose);
    }

    animation.frames.push_back(std::move(frame));
  }

  // Replace reference-pose transforms with sampled KF data.
  for (const auto &track : tracks) {
    std::size_t translation_hint = 0;
    std::size_t rotation_hint = 0;
    std::size_t scale_hint = 0;

    const Transform &reference =
        skeleton.bones[track.bone_index].reference_pose;

    for (std::uint32_t frame_index = 0; frame_index < num_frames;
         ++frame_index) {

      const float time = static_cast<float>(frame_index) * frame_increment;

      animation.frames[frame_index].transforms[track.bone_index] =
          sample_transform(track, reference, time, translation_hint,
                           rotation_hint, scale_hint);
    }
  }

  return animation;
}

} // namespace niflib_animation
