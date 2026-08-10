# serde-hkx enhanced library

<div align="center">
  <a href="https://github.com/SARDONYX-sard/serde-hkx/releases/latest">
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
    <a href="https://github.com/SARDONYX-sard/serde-hkx-enhanced/actions/workflows/test.yaml">
      <img src="https://github.com/SARDONYX-sard/serde-hkx-enhanced/actions/workflows/test.yaml/badge.svg?style=flat-square" alt="Test (Cargo)">
    </a>
  </p>
</div>

## Features

- [x] skeleton + animation.hkx => kf
- [x] skeleton + animation.hkx => fbx

The following FIs are used here:

- The niflib FI implemented by cxx for kf
- The ufbx-write FI implemented by cbindgen

## build CLI

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
