/// xtask: build and deploy the SlicePlayer CLAP plugin.
///
/// Usage:
///   cargo xtask bundle   — release build, copy .clap to ~/.clap/mill/
///   cargo xtask clean    — clean target

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("bundle") | None => bundle(),
        Some("clean")         => clean(),
        Some(cmd)             => eprintln!("Unknown command: {cmd}"),
    }
}

fn bundle() {
    let root = workspace_root();
    println!("Building slice_player (release)...");

    let status = std::process::Command::new("cargo")
        .args(["build", "--release", "-p", "slice_player"])
        .current_dir(&root)
        .status()
        .expect("cargo build failed");

    if !status.success() {
        eprintln!("Build failed.");
        std::process::exit(1);
    }

    let so = root.join("target/release/libslice_player.so");
    let dest_dir = std::path::PathBuf::from(
        std::env::var("HOME").unwrap_or_else(|_| ".".into())
    ).join(".clap/mill");

    std::fs::create_dir_all(&dest_dir).unwrap();
    let dest = dest_dir.join("slice_player.clap");
    std::fs::copy(&so, &dest).expect("copy failed");
    println!("✅ Deployed → {}", dest.display());
}

fn clean() {
    let root = workspace_root();
    std::process::Command::new("cargo")
        .args(["clean"])
        .current_dir(&root)
        .status()
        .expect("cargo clean failed");
}

fn workspace_root() -> std::path::PathBuf {
    // xtask lives in <workspace>/xtask/; workspace root is one level up.
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("no parent dir")
        .to_path_buf()
}
