#[path = "src/cargo_target_rule.rs"]
mod cargo_target_rule;

fn main() {
    // Refuse a target under a literal `~` folder or a relative one before
    // anything else lands there (see cargo_target_rule.rs).
    println!("cargo:rerun-if-env-changed=CARGO_TARGET_DIR");
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        if let Some(problem) = cargo_target_rule::problem(&dir) {
            panic!("{problem}");
        }
    }
    if let Some(problem) = std::env::var("OUT_DIR")
        .ok()
        .and_then(|out| cargo_target_rule::out_dir_problem(&out))
    {
        panic!("{problem}");
    }
    let status = std::process::Command::new("node")
        .arg("../scripts/source-provenance.mjs")
        .status()
        .expect("Node is required to fingerprint Hollow build inputs");
    assert!(status.success(), "could not fingerprint Hollow source");
    println!("cargo:rerun-if-changed=../scripts/source-provenance.mjs");
    // The release build's 0.6.<n> stamp (scripts/release/build-app.mjs).
    println!("cargo:rerun-if-env-changed=HOLLOW_BUILD_VERSION");
    tauri_build::build()
}
