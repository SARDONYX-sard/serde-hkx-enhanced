# serde-hkx enhanced library

<div align="center">
  <a href="https://github.com/SARDONYX-sard/serde-hkx-enhanced/releases/latest">
    <img src="./crates/cli/assets/icon.svg" alt="serde hkx logo" width="150" height="150" />
  </a>

  <!-- Release & Build Badges -->
  <p>
    <a href="https://github.com/SARDONYX-sard/serde-hkx-enhanced/releases">
      <img src="https://img.shields.io/github/downloads/SARDONYX-sard/serde-hkx-enhanced/total?style=flat-square" alt="Total Downloads">
    </a>
    <a href="https://github.com/SARDONYX-sard/serde-hkx-enhanced/actions/workflows/release-cli.yaml">
      <img src="https://github.com/SARDONYX-sard/serde-hkx-enhanced/actions/workflows/release-cli.yaml/badge.svg?style=flat-square" alt="Release (CLI)">
    </a>
    <a href="https://github.com/SARDONYX-sard/serde-hkx-enhanced/actions/workflows/build-cli.yaml">
      <img src="https://github.com/SARDONYX-sard/serde-hkx-enhanced/actions/workflows/build-cli.yaml/badge.svg?style=flat-square" alt="Build (CLI)">
    </a>
  </p>
</div>

## Features

- kf
  - [x] Skeleton + animation.hkx => kf
  - [x] kf => Skeleton + animation.hkx (It hasn't been released yet, and there's something slightly off about the spline serialization.)

- fbx
  - [x] skeleton + animation.hkx => fbx
  - [ ] fbx => skeleton + animation.hkx

## build CLI

The following FFI are used here:

- The [`niflib`](https://github.com/niftools/niflib) FFI implemented by cxx for kf
- The [`ufbx-write`](https://github.com/ufbx/ufbx-write) FFI implemented by cbindgen

```shell
git clone --recurse-submodules https://github.com/SARDONYX-sard/serde-hkx-enhanced.git;
cd ./serde-hkx-enhanced;
cargo build
```

## License

Since the B-Spline analysis is based on GPL-3.0 code written in C++, it is licensed under the GPL.

Reference:

- <https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.hpp>
- <https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.cpp>

![cow_kf](https://github.com/user-attachments/assets/5519e2f4-170c-4489-807e-a302752180e8)
![cow_fbx](https://github.com/user-attachments/assets/0fe57b5a-9ebc-44e1-9088-a1282a2d3e28)
