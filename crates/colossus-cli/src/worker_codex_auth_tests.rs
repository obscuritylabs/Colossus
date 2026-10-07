use super::*;
use crate::worker_codex_auth;

fn private_auth_file(directory: &Path) -> PathBuf {
    let path = directory.join("auth.json");
    // Deliberately not an auth document: selection must not load or parse contents.
    fs::write(&path, b"synthetic metadata-only fixture").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    path
}

#[test]
fn worker_auth_selection_forwards_exact_private_path_without_loading_credentials() {
    let directory = private_tempdir();
    let parent = fs::canonicalize(directory.path()).unwrap();
    let path = private_auth_file(&parent);
    let cli = Cli::try_parse_from([
        "colossus",
        "worker",
        "--codex-auth-path",
        path.to_str().unwrap(),
    ])
    .unwrap();
    let Command::Worker(worker) = cli.command else {
        panic!("worker");
    };
    let store = worker_codex_auth::select(worker.codex_auth_path.as_deref(), Path::new("."))
        .unwrap()
        .unwrap();
    assert_eq!(store.path(), path);
    assert_eq!(fs::read(&path).unwrap(), b"synthetic metadata-only fixture");
    assert!(
        worker_codex_auth::select(None, Path::new("missing-default-workspace"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn auth_path_is_serve_only_and_conflicts_with_other_worker_operations() {
    for operation in ["--once", "--status", "--shutdown"] {
        let error = Cli::try_parse_from([
            "colossus",
            "worker",
            "--codex-auth-path",
            "/private/auth.json",
            operation,
        ])
        .err()
        .unwrap();
        assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
        let error =
            Cli::try_parse_from(["colossus", "worker", "--no-model-network-tools", operation])
                .err()
                .unwrap();
        assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
    }
}

#[test]
fn worker_model_network_opt_out_is_host_only_and_preserves_provider_account_selection() {
    let directory = private_tempdir();
    let parent = fs::canonicalize(directory.path()).unwrap();
    let path = private_auth_file(&parent);
    let cli = Cli::try_parse_from([
        "colossus",
        "worker",
        "--no-model-network-tools",
        "--codex-auth-path",
        path.to_str().unwrap(),
    ])
    .unwrap();
    let Command::Worker(worker) = cli.command else {
        panic!("worker");
    };
    let options = RuntimeOpenOptions::for_workspace(&parent).unwrap();
    let configured = worker_codex_auth::runtime_options(&worker, options);
    assert!(!configured.model_network_tools_enabled());
    assert_eq!(configured.workspace, parent);
    let auth = worker_codex_auth::select(worker.codex_auth_path.as_deref(), Path::new("."))
        .unwrap()
        .unwrap();
    assert_eq!(auth.path(), path);
    let Command::Worker(default) = Cli::try_parse_from(["colossus", "worker"]).unwrap().command
    else {
        panic!("worker");
    };
    let default = worker_codex_auth::runtime_options(
        &default,
        RuntimeOpenOptions::for_workspace(&parent).unwrap(),
    );
    assert!(default.model_network_tools_enabled());
}

#[test]
fn invalid_auth_selection_never_creates_paths_or_falls_back() {
    let directory = private_tempdir();
    let parent = fs::canonicalize(directory.path()).unwrap();
    let missing = parent.join("must-remain-absent/auth.json");
    let overlong = PathBuf::from(format!("/{}", "x".repeat(4096)));
    for path in [
        Path::new("relative/auth.json"),
        Path::new("/private/auth\ninvalid.json"),
        overlong.as_path(),
        missing.as_path(),
        parent.as_path(),
    ] {
        let error = worker_codex_auth::select(Some(path), Path::new("."))
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(!error.to_string().contains(&path.display().to_string()));
    }
    assert!(!missing.parent().unwrap().exists());
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
}

#[test]
fn oversized_selected_file_is_refused_without_loading_or_modifying_it() {
    let directory = private_tempdir();
    let parent = fs::canonicalize(directory.path()).unwrap();
    let path = private_auth_file(&parent);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(256 * 1024 + 1)
        .unwrap();
    assert!(worker_codex_auth::select(Some(&path), Path::new(".")).is_err());
    assert_eq!(fs::metadata(path).unwrap().len(), 256 * 1024 + 1);
}

#[cfg(unix)]
#[test]
fn unsafe_private_file_permissions_and_link_aliases_are_refused() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let directory = private_tempdir();
    let parent = fs::canonicalize(directory.path()).unwrap();
    let path = private_auth_file(&parent);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(worker_codex_auth::select(Some(&path), Path::new(".")).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let alias = parent.join("alias.json");
    symlink(&path, &alias).unwrap();
    assert!(worker_codex_auth::select(Some(&alias), Path::new(".")).is_err());
    fs::remove_file(&alias).unwrap();
    fs::hard_link(&path, &alias).unwrap();
    assert!(worker_codex_auth::select(Some(&path), Path::new(".")).is_err());
    assert!(worker_codex_auth::select(Some(&alias), Path::new(".")).is_err());
}

#[test]
fn selected_account_file_cannot_be_inside_the_tool_workspace() {
    let directory = private_tempdir();
    let parent = fs::canonicalize(directory.path()).unwrap();
    let path = private_auth_file(&parent);
    assert!(worker_codex_auth::select(Some(&path), &parent).is_err());
    assert_eq!(fs::read(path).unwrap(), b"synthetic metadata-only fixture");
}
