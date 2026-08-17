#pragma once

#include "niflib_animation/src/ffi.rs.h"

#include "nif_basic_types.h"

namespace niflib_animation {

inline Niflib::NifInfo new_nif_info() {
  return Niflib::NifInfo{Niflib::VER_20_2_0_7, 11, 83};
}

rust::Vec<std::uint8_t> export_kf(const Skeleton &skeleton,
                                  const Animation &animation);
Animation convert_kf(rust::Slice<const std::uint8_t> input,
                     const Skeleton &skeleton, float fps);


NifScene load_nif(rust::Str path);
} // namespace niflib_animation
