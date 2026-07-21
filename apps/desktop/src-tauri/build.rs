use std::{env, fs, path::PathBuf};

const COMMANDS: &[&str] = &[
    "connect_colossus",
    "connection_status",
    "create_run",
    "get_run",
    "list_runs",
    "watch_run",
    "cancel_run",
    "respond_interaction",
];

fn main() {
    stage_connection_config();
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to build the Colossus desktop manifest");
}

fn stage_connection_config() {
    const MAX_CONFIG_BYTES: usize = 16 * 1024;

    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("Cargo must provide CARGO_MANIFEST_DIR"),
    );
    let local = manifest_dir.join("connection.local.json");
    let template = manifest_dir.join("connection.json");
    let source = if local.is_file() { &local } else { &template };
    let contents = fs::read(source).expect("failed to read the desktop connection configuration");
    assert!(
        contents.len() <= MAX_CONFIG_BYTES,
        "desktop connection configuration exceeds {MAX_CONFIG_BYTES} bytes"
    );

    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR"))
        .join("connection.json");
    fs::write(output, contents).expect("failed to stage the desktop connection configuration");

    println!("cargo:rerun-if-changed={}", template.display());
    println!("cargo:rerun-if-changed={}", local.display());
}
