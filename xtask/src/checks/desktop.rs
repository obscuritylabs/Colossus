use crate::{cli::DesktopProfile, repository::Repository};
use std::{
    env, fs,
    io::Write as _,
    path::{Path, PathBuf},
};

pub(super) fn prepare(
    repository: &Repository,
    profile: DesktopProfile,
    requested_target: Option<&str>,
) -> Result<(), String> {
    let host = repository
        .task("rustc")
        .args(["--print", "host-tuple"])
        .output()?
        .trim()
        .to_owned();
    let target = requested_target
        .map(str::to_owned)
        .or_else(|| env::var("COLOSSUS_DESKTOP_TARGET").ok())
        .unwrap_or_else(|| host.clone());
    validate_target(&target)?;

    let mut build = repository.task("cargo").args(["build", "--locked"]).args([
        "--package",
        "colossus-cli",
        "--package",
        "colossus-sidecar",
        "--bins",
    ]);
    if profile == DesktopProfile::Release {
        build = build.arg("--release");
    }
    if target != host {
        build = build.args(["--target", target.as_str()]);
    }
    build.run()?;

    let target_root = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository.path("target"));
    let target_root = if target_root.is_absolute() {
        target_root
    } else {
        repository.path(target_root)
    };
    let profile_name = match profile {
        DesktopProfile::Debug => "debug",
        DesktopProfile::Release => "release",
    };
    let artifact_root = if target == host {
        target_root.join(profile_name)
    } else {
        target_root.join(&target).join(profile_name)
    };
    let extension = if target.contains("-windows-") {
        ".exe"
    } else {
        ""
    };
    let destination = repository.path("apps/desktop/src-tauri/binaries");
    if fs::symlink_metadata(&destination).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("desktop binaries directory must not be a symlink".into());
    }
    fs::create_dir_all(&destination)
        .map_err(|error| format!("could not create desktop binaries directory: {error}"))?;
    for name in ["colossus-sidecar", "colossus"] {
        let source = artifact_root.join(format!("{name}{extension}"));
        let output = destination.join(format!("{name}-{target}{extension}"));
        stage(&source, &output)?;
    }
    eprintln!("Prepared Managed Local binaries for {target} ({profile_name}).");
    Ok(())
}

fn validate_target(target: &str) -> Result<(), String> {
    if target.is_empty()
        || target.len() > 128
        || !target
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        Err("desktop target triple contains unsafe characters".into())
    } else {
        Ok(())
    }
}

fn stage(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|error| {
        format!(
            "desktop binary {} is unavailable: {error}",
            source.display()
        )
    })?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > 512 * 1024 * 1024
    {
        return Err(format!(
            "desktop binary {} is not a bounded regular file",
            source.display()
        ));
    }
    let temporary = destination.with_extension(format!(
        "{}.{}.tmp",
        destination
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("bin"),
        std::process::id()
    ));
    let result = (|| {
        let mut input = fs::File::open(source)
            .map_err(|error| format!("could not open {}: {error}", source.display()))?;
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| format!("could not stage {}: {error}", destination.display()))?;
        std::io::copy(&mut input, &mut output)
            .map_err(|error| format!("could not copy {}: {error}", source.display()))?;
        output
            .flush()
            .and_then(|()| output.sync_all())
            .map_err(|error| format!("could not sync {}: {error}", temporary.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&temporary, fs::Permissions::from_mode(0o755))
                .map_err(|error| format!("could not secure {}: {error}", temporary.display()))?;
        }
        fs::rename(&temporary, destination)
            .map_err(|error| format!("could not publish {}: {error}", destination.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
