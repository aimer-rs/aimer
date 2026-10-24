mod build_support;

fn main() {
    println!("cargo:rerun-if-changed=icons");
    println!("cargo:rerun-if-changed=build_support.rs");
    println!("cargo:rustc-check-cfg=cfg(aimer_lucide_precompiled)");
    // PROFILE describes the target profile, unlike cfg!(debug_assertions),
    // which describes how Cargo compiled this host-side build script.
    let release = std::env::var("PROFILE").unwrap() == "release";
    if release {
        println!("cargo:rustc-cfg=aimer_lucide_precompiled");
    }
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    build_support::generate(&root.join("icons"), &output, release)
        .unwrap_or_else(|error| panic!("Lucide generation failed: {error}"));
}
