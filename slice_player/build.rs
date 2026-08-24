// build.rs — compiles VelociLoops (C++) as a static library and links it.
fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = std::path::Path::new(&manifest).parent().unwrap(); // workspace root
    let vl = root.join("velocloops");

    let mut build = cc::Build::new();
    build.cpp(true)
        .file(vl.join("src/velociloops.cpp"))
        .include(vl.join("include"))
        .std("c++17");

    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("msvc") {
        build.flag("-O2").flag("-fPIC");
    }

    build.compile("velociloops");

    // Tell cargo to re-run if the C++ source changes.
    println!("cargo:rerun-if-changed={}", vl.join("src/velociloops.cpp").display());
    println!("cargo:rerun-if-changed={}", vl.join("include/velociloops.h").display());
}
