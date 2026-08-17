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
    "desktop_release_metadata",
    "check_desktop_update",
    "install_desktop_update",
    "export_diagnostics",
    "initialize_desktop",
    "desktop_status",
    "codex_auth_status",
    "codex_auth_login",
    "codex_auth_logout",
    "import_ca_bundle",
    "remove_ca_bundle",
    "add_external_target",
    "remove_external_target",
    "choose_workspace",
    "create_space",
    "list_spaces",
    "select_space",
    "rename_space",
    "archive_space",
    "restore_space",
    "search_space_threads",
    "configure_managed_runtime",
    "apply_managed_model_configuration",
    "run_managed_self_test",
    "restart_managed_runtime",
    "get_thread_delegate",
    "get_session_map",
    "select_target",
    "set_approval_mode",
    "set_terminal_enabled",
    "connect_colossus",
    "connection_status",
    "create_run",
    "choose_run_attachment",
    "read_artifact_content",
    "get_run",
    "list_runs",
    "list_asides",
    "watch_run",
    "cancel_run",
    "archive_thread",
    "restore_thread",
    "respond_interaction",
    "list_workspace_directory",
    "read_workspace_file",
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
    const SIGNING_VARIABLE: &str = "COLOSSUS_DESKTOP_CODE_SIGNING_STATUS";
    const UPDATE_ENDPOINT_VARIABLE: &str = "COLOSSUS_DESKTOP_UPDATE_ENDPOINT";
    const UPDATE_PUBLIC_KEY_VARIABLE: &str = "COLOSSUS_DESKTOP_UPDATE_PUBLIC_KEY";

    println!("cargo:rerun-if-env-changed={TEAM_VARIABLE}");
    println!("cargo:rerun-if-env-changed={CHANNEL_VARIABLE}");
    println!("cargo:rerun-if-env-changed={UPDATE_ENDPOINT_VARIABLE}");
    println!("cargo:rerun-if-env-changed={UPDATE_PUBLIC_KEY_VARIABLE}");
    if env::var("PROFILE").as_deref() == Ok("debug") {
        println!("cargo:rustc-env={CHANNEL_VARIABLE}=development");
        println!("cargo:rustc-env={SIGNING_VARIABLE}=development");
        println!("cargo:rustc-env={UPDATE_ENDPOINT_VARIABLE}=");
        println!("cargo:rustc-env={UPDATE_PUBLIC_KEY_VARIABLE}=");
        return;
    }
    let team_id = env::var(TEAM_VARIABLE).unwrap_or_else(|_| {
        panic!("release desktop builds require an explicit platform signing identity marker")
    });
    let release_channel = env::var(CHANNEL_VARIABLE).unwrap_or_else(|_| {
        panic!(
            "release desktop builds require {CHANNEL_VARIABLE}=stable, developer_preview, or validation_only"
        )
    });
    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("Cargo must provide target OS");
    if target_os == "windows" {
        match release_channel.as_str() {
            "developer_preview" | "validation_only" => assert!(
                team_id == "UNSIGNED",
                "unsigned Windows preview builds require {TEAM_VARIABLE}=UNSIGNED"
            ),
            "stable" => panic!(
                "stable Windows Desktop is disabled until an Authenticode signer is configured"
            ),
            _ => panic!("{CHANNEL_VARIABLE} must be stable, developer_preview, or validation_only"),
        }
    } else {
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
    }
    let signing_status = match (target_os.as_str(), release_channel.as_str()) {
        ("windows", "developer_preview" | "validation_only") => "unsigned",
        ("macos", "stable") => "verified",
        ("macos", "developer_preview" | "validation_only") => "ad_hoc",
        _ => "unsupported",
    };
    let updates_enabled = release_channel == "stable";
    let update_endpoint = env::var(UPDATE_ENDPOINT_VARIABLE).unwrap_or_default();
    let update_public_key = env::var(UPDATE_PUBLIC_KEY_VARIABLE).unwrap_or_default();
    if updates_enabled {
        assert!(
            valid_update_endpoint(&update_endpoint),
            "{release_channel} Desktop builds require a bounded HTTPS {UPDATE_ENDPOINT_VARIABLE}"
        );
        assert!(
            valid_update_public_key(&update_public_key),
            "{release_channel} Desktop builds require a one-line base64 Tauri updater public key in {UPDATE_PUBLIC_KEY_VARIABLE}"
        );
    } else {
        assert!(
            update_endpoint.is_empty() && update_public_key.is_empty(),
            "unsigned Developer Preview and validation-only Desktop builds must not advertise an update channel"
        );
    }
    println!("cargo:rustc-env={TEAM_VARIABLE}={team_id}");
    println!("cargo:rustc-env={CHANNEL_VARIABLE}={release_channel}");
    println!("cargo:rustc-env={SIGNING_VARIABLE}={signing_status}");
    println!("cargo:rustc-env={UPDATE_ENDPOINT_VARIABLE}={update_endpoint}");
    println!("cargo:rustc-env={UPDATE_PUBLIC_KEY_VARIABLE}={update_public_key}");
}

fn valid_update_endpoint(value: &str) -> bool {
    value.len() <= 2_048
        && value.starts_with("https://")
        && !value.bytes().any(|byte| byte.is_ascii_whitespace())
}

fn valid_update_public_key(value: &str) -> bool {
    (32..=4_096).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
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
    let windows_target =
        env::var_os("CARGO_CFG_TARGET_OS").as_deref() == Some(std::ffi::OsStr::new("windows"));
    let extension = if windows_target { ".exe" } else { "" };
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
    if windows_target {
        stage_unsealed_windows_resource(
            &staged_directory,
            &target,
            &release_channel,
            format!("{SIDECAR_FILE_STEM}{extension}"),
            format!("{CLI_FILE_STEM}{extension}"),
        );
    }

    println!("cargo:rerun-if-changed={}", sidecar_source.display());
    println!("cargo:rerun-if-changed={}", cli_source.display());
    println!("cargo:rustc-env=COLOSSUS_DESKTOP_TARGET_TRIPLE={target}");
    println!(
        "cargo:rustc-env=COLOSSUS_DESKTOP_BUNDLE_MANIFEST={}",
        output.display()
    );
}

fn stage_unsealed_windows_resource(
    staged_directory: &Path,
    target: &str,
    release_channel: &str,
    sidecar_name: String,
    cli_name: String,
) {
    // Tauri validates configured resources while compiling, before the Windows
    // packaging script creates and binds the sealed release manifest. Keep this
    // compile-time resource explicitly unusable and free of development paths.
    // `package-desktop-windows.ps1` replaces it with the strict manifest only
    // after the release executable has been built.
    let manifest = BundleManifest {
        schema_version: 2,
        target_triple: target,
        profile: "unsealed_release",
        release_channel,
        sidecar: unusable_release_entry(sidecar_name),
        cli: unusable_release_entry(cli_name),
    };
    let encoded =
        serde_json::to_vec(&manifest).expect("failed to encode unsealed Windows resource");
    let output = staged_directory.join("colossus-bundle-manifest.json");
    fs::write(output, encoded).expect("failed to stage unsealed Windows bundle resource");
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
