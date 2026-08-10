#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::Path;

fn main() {
    let ffi_dir = Path::new("ffi");
    let header = ffi_dir.join("ufbx_write.h");
    let source = ffi_dir.join("ufbx_write.c");

    cc::Build::new()
        .file(&source)
        .include(ffi_dir)
        .flag_if_supported("-std=c99")
        .compile("ufbx_write");

    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed={}", header.display());

    #[cfg(feature = "generate")]
    generate_binding(&header, ffi_dir);
}

#[cfg(feature = "generate")]
fn generate_binding(header: &Path, ffi_dir: &Path) {
    let bindings = bindgen::Builder::default()
        .header(header.to_string_lossy())
        .clang_arg(format!("-I{}", ffi_dir.display()))
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("failed to generate ufbx_write bindings");

    bindings
        .write_to_file("src/bindings.rs")
        .expect("failed to write src/bindings.rs");
}
