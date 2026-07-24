use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::{
    env, fs,
    io::Read as _,
    path::{Path, PathBuf},
};

const MAX_BUNDLED_BINARY_BYTES: u64 = 512 * 1024 * 1024;
const SIDECAR_FILE_STEM: &str = "colossus-sidecar";
const CLI_FILE_STEM: &str = "colossus";

const COMMANDS: &[&str] = &[
    "desktop_release_channel",
    "initialize_desktop",
    "desktop_status",
    "add_external_target",
    "remove_external_target",
    "choose_workspace",
    "configure_managed_runtime",
    "apply_managed_model_configuration",
    "run_managed_self_test",
    "restart_managed_runtime",
    "select_target",
    "set_terminal_enabled",
    "connect_colossus",
    "connection_status",
    "create_run",
    "get_run",
    "list_runs",
    "watch_run",
    "cancel_run",
    "respond_interaction",
    "show_terminal_window",
    "terminal_context",
    "open_terminal",
    "write_terminal",
    "resize_terminal",
    "signal_terminal",
    "close_terminal",
];

fn main() {
    export_release_trust_configuration();
    stage_connection_config();
    stage_bundle_manifest();
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to build the Colossus desktop manifest");
}

fn export_release_trust_configuration() {
    const TEAM_VARIABLE: &str = "COLOSSUS_DESKTOP_TEAM_ID";
    const CHANNEL_VARIABLE: &str = "COLOSSUS_DESKTOP_RELEASE_CHANNEL";

    println!("cargo:rerun-if-env-changed={TEAM_VARIABLE}");
    println!("cargo:rerun-if-env-changed={CHANNEL_VARIABLE}");
    if env::var("PROFILE").as_deref() == Ok("debug") {
        println!("cargo:rustc-env={CHANNEL_VARIABLE}=development");
        return;
    }
    let team_id = env::var(TEAM_VARIABLE).unwrap_or_else(|_| {
        panic!(
            "release desktop builds require {TEAM_VARIABLE}=ADHOC for a developer preview or the canonical Apple Team ID for stable"
        )
    });
    let release_channel = env::var(CHANNEL_VARIABLE).unwrap_or_else(|_| {
        panic!(
            "release desktop builds require {CHANNEL_VARIABLE}=stable, developer_preview, or validation_only"
        )
    });
    let canonical_team = team_id.len() == 10
        && team_id
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit());
    match release_channel.as_str() {
        "stable" => assert!(
            canonical_team,
            "stable releases require {TEAM_VARIABLE} to be a canonical 10-character Apple Team ID"
        ),
        "developer_preview" | "validation_only" => assert!(
            team_id == "ADHOC",
            "developer-preview and validation-only builds require {TEAM_VARIABLE}=ADHOC"
        ),
        _ => panic!("{CHANNEL_VARIABLE} must be stable, developer_preview, or validation_only"),
    }
    println!("cargo:rustc-env={TEAM_VARIABLE}={team_id}");
    println!("cargo:rustc-env={CHANNEL_VARIABLE}={release_channel}");
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleManifest<'a> {
    schema_version: u16,
    target_triple: &'a str,
    profile: &'a str,
    release_channel: &'a str,
    sidecar: BundledExecutable,
    cli: BundledExecutable,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BundledExecutable {
    file_name: String,
    sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    development_path: Option<PathBuf>,
}

fn stage_bundle_manifest() {
    let target = env::var("TARGET").expect("Cargo must provide TARGET");
    let profile = env::var("PROFILE").expect("Cargo must provide PROFILE");
    let release_channel = if profile == "debug" {
        "development".to_owned()
    } else {
        env::var("COLOSSUS_DESKTOP_RELEASE_CHANNEL")
            .expect("release trust configuration must provide the release channel")
    };
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("Cargo must provide CARGO_MANIFEST_DIR"),
    );
    let extension =
        if env::var_os("CARGO_CFG_TARGET_OS").as_deref() == Some(std::ffi::OsStr::new("windows")) {
            ".exe"
        } else {
            ""
        };
    let staged_directory = manifest_dir.join("binaries");
    let sidecar_source = staged_directory.join(format!("{SIDECAR_FILE_STEM}-{target}{extension}"));
    let cli_source = staged_directory.join(format!("{CLI_FILE_STEM}-{target}{extension}"));
    let sidecar_name = format!("{SIDECAR_FILE_STEM}{extension}");
    let cli_name = format!("{CLI_FILE_STEM}{extension}");

    let (manifest_profile, sidecar, cli) = if profile == "debug" {
        (
            "debug",
            debug_entry(&sidecar_source, sidecar_name),
            debug_entry(&cli_source, cli_name),
        )
    } else {
        // macOS signing mutates Mach-O bytes after the Rust build. A release may only
        // trust the strict manifest generated from the signed nested executables and
        // sealed into Contents/Resources before the outer app signature is applied.
        // Zero digests make this compile-time marker unusable as executable authority.
        (
            "unsealed_release",
            unusable_release_entry(sidecar_name),
            unusable_release_entry(cli_name),
        )
    };
    let manifest = BundleManifest {
        schema_version: 2,
        target_triple: &target,
        profile: manifest_profile,
        release_channel: &release_channel,
        sidecar,
        cli,
    };
    let encoded = serde_json::to_vec(&manifest).expect("failed to encode bundle manifest");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR"))
        .join("bundle-manifest.json");
    fs::write(&output, encoded).expect("failed to stage the desktop bundle manifest");

    println!("cargo:rerun-if-changed={}", sidecar_source.display());
    println!("cargo:rerun-if-changed={}", cli_source.display());
    println!("cargo:rustc-env=COLOSSUS_DESKTOP_TARGET_TRIPLE={target}");
    println!(
        "cargo:rustc-env=COLOSSUS_DESKTOP_BUNDLE_MANIFEST={}",
        output.display()
    );
}

fn debug_entry(source: &Path, file_name: String) -> BundledExecutable {
    let canonical = validate_staged_binary(source);
    BundledExecutable {
        file_name,
        sha256: sha256_file(&canonical),
        development_path: Some(canonical),
    }
}

fn unusable_release_entry(file_name: String) -> BundledExecutable {
    BundledExecutable {
        file_name,
        sha256: "0".repeat(64),
        development_path: None,
    }
}

fn validate_staged_binary(path: &Path) -> PathBuf {
    let metadata = fs::symlink_metadata(path).unwrap_or_else(|_| {
        panic!(
            "missing staged desktop binary {}; run scripts/prepare-desktop-binaries first",
            path.display()
        )
    });
    assert!(
        metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
        "staged desktop binary must be a regular non-symlink file: {}",
        path.display()
    );
    assert!(
        metadata.len() > 0 && metadata.len() <= MAX_BUNDLED_BINARY_BYTES,
        "staged desktop binary has an unsafe size: {}",
        path.display()
    );
    fs::canonicalize(path).expect("failed to canonicalize staged desktop binary")
}

fn sha256_file(path: &Path) -> String {
    let mut file = fs::File::open(path).expect("failed to open staged desktop binary");
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .expect("failed to hash staged desktop binary");
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    format!("{:x}", hasher.finalize())
}

fn stage_connection_config() {
    const MAX_CONFIG_BYTES: usize = 16 * 1024;

    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("Cargo must provide CARGO_MANIFEST_DIR"),
    );
    let local = manifest_dir.join("connection.local.json");
    let template = manifest_dir.join("connection.json");
    // A developer's optional External target must never become signed release
    // configuration. Release builds always embed the inert placeholder template.
    let source = if env::var("PROFILE").as_deref() == Ok("debug") && local.is_file() {
        &local
    } else {
        &template
    };
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
