#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

// Intended:
//
// niflib_animation/
// ├── Cargo.toml
// ├── build.rs
// ├── src/
// │   ├── lib.rs
// │   └── ffi.rs
// │
// └── cxx/
//     ├── niflib/
//     │   ├── include/
//     │   └── lib/
//     │
//     └── src/
//         ├── convert_kf.cc
//         ├── export_kf.cc
//         ├── load_nif.cc
//         │
//         └── niflib_animation.h

fn main() {
    let crate_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // e.g., "x86_64-pc-windows-msvc"
    let target_triple = std::env::var("TARGET").expect("TARGET env var is not set");

    let cxx_root = crate_root.join("cxx");
    let ffi_root = cxx_root.join("src");
    let niflib_root = cxx_root.join("niflib");

    let include_dir = niflib_root.join("include");
    let lib_dir = niflib_root.join("lib");

    // Download C++ libraries if they are not available locally.
    let libs_existed = std::fs::exists(&include_dir).unwrap_or_default();
    let lib_dir_existed = std::fs::exists(&lib_dir).unwrap_or_default();

    if !libs_existed || !lib_dir_existed {
        fetch_libs(&niflib_root, &target_triple);
    }

    cxx_build::bridge("src/ffi.rs")
        .std("c++20")
        .include(&cxx_root)
        .include(&ffi_root)
        .include(&include_dir)
        .define("NIFLIB_STATIC_LINK", None)
        .file(cxx_root.join("src/convert_kf.cc"))
        .file(cxx_root.join("src/export_kf.cc"))
        .file(cxx_root.join("src/load_nif.cc"))
        .compile("cxxbridge_niflib_animation");

    println!("cargo:rerun-if-changed=src/ffi.rs");
    println!("cargo:rerun-if-changed=cxx/src/export_kf.cc");
    println!("cargo:rerun-if-changed=cxx/src/convert_kf.cc");
    println!("cargo:rerun-if-changed=cxx/src/load_nif.cc");

    println!("cargo:rerun-if-changed=cxx/src/niflib_animation.h");

    println!("cargo:rustc-link-search={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=niflib_static");
}

fn fetch_libs<P>(out_dir: P, target_triple: &str)
where
    P: AsRef<std::path::Path>,
{
    use std::io::Cursor;

    let url = format!(
        "https://github.com/SARDONYX-sard/niflib_ffi/releases/download/cpp/niflib_{target_triple}.zip",
    );

    let out_dir = out_dir.as_ref();

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60 * 30))
        .build()
        .unwrap();

    let response = client.get(&url).send().unwrap_or_else(|e| {
        panic!("Failed to download ZIP. url: {url}, err: {e}");
    });

    let bytes = response.bytes().expect("Failed to read response bytes");

    let mut archive =
        zip::read::ZipArchive::new(Cursor::new(bytes)).unwrap_or_else(|err| panic!("{err}"));

    archive
        .extract(out_dir)
        .unwrap_or_else(|err| panic!("{err}"));
}
