//! Compila o xatlas (C++ vendorizado em `vendor/xatlas`) + a fronteira C
//! (`native/anigo_xatlas_shim.cpp`) apenas quando a feature `xatlas` está ligada.
//! Só precisa de um compilador C++ (MSVC/clang/g++) e do crate `cc` — sem cmake,
//! sem libclang/bindgen.

#[cfg(feature = "xatlas")]
fn main() {
    let vendor = "vendor/xatlas/xatlas.cpp";
    let shim = "native/anigo_xatlas_shim.cpp";
    for f in [
        vendor,
        "vendor/xatlas/xatlas.h",
        shim,
        "native/anigo_xatlas_shim.h",
    ] {
        println!("cargo:rerun-if-changed={f}");
    }

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file(vendor)
        .file(shim)
        .include("native")
        .include("vendor/xatlas")
        // O xatlas é pesado em laço interno: mesmo em `dev` compila otimizado,
        // senão o auto-unwrap de uma malha de 20k faces leva minutos.
        .opt_level(2)
        .debug(false)
        .define("NDEBUG", None)
        .warnings(false)
        // MSVC ignora/rejeita -std=c++11; gcc/clang aceitam.
        .flag_if_supported("-std=c++11")
        // `XA_MULTITHREADED` usa std::thread: em glibc antiga precisa de -pthread.
        .flag_if_supported("-pthread");
    build.compile("anigo_xatlas");

    // libstdc++/libc++ são adicionadas pelo `cc` (cpp(true)); pthread só no Linux.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-lib=pthread");
    }
}

#[cfg(not(feature = "xatlas"))]
fn main() {}
