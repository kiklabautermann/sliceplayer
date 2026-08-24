// build.rs — compiles VelociLoops (C++) as a static library and links it.
fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = std::path::Path::new(&manifest).parent().unwrap(); // workspace root
    let vl = root.join("velocloops");

    cc::Build::new()
        .cpp(true)
        .file(vl.join("src/velociloops.cpp"))
        .include(vl.join("include"))
        .flag("-std=c++17")
        .flag("-O2")
        .flag("-fPIC")
        .compile("velociloops");

    // Tell cargo to re-run if the C++ source changes.
    println!("cargo:rerun-if-changed={}", vl.join("src/velociloops.cpp").display());
    println!("cargo:rerun-if-changed={}", vl.join("include/velociloops.h").display());
}
