use std::{
    fs,
    path::{Path, PathBuf},
};

/// Find ripgrep shipped beside a Desktop or package-manager executable, or in
/// the direct installer's versioned private tool directory. Source builds have
/// no managed tool and may use an explicitly granted or ambient system copy.
pub(super) fn managed_ripgrep() -> Result<Option<PathBuf>, &'static str> {
    let executable =
        std::env::current_exe().map_err(|_| "cannot locate the running Colossus executable")?;
    managed_ripgrep_at(
        &executable,
        env!("CARGO_PKG_VERSION"),
        std::env::var("COLOSSUS_BUNDLED_RIPGREP").ok().as_deref(),
    )
}

fn managed_ripgrep_at(
    executable: &Path,
    version: &str,
    bundled_marker: Option<&str>,
) -> Result<Option<PathBuf>, &'static str> {
    let parent = executable
        .parent()
        .ok_or("running executable has no parent directory")?;
    let name = if cfg!(windows) { "rg.exe" } else { "rg" };
    let direct_root = parent.join(".colossus-tools");
    let candidate = if direct_root.exists() {
        let version_directory = direct_root.join(version);
        for directory in [&direct_root, &version_directory] {
            let metadata = fs::symlink_metadata(directory)
                .map_err(|_| "managed ripgrep is missing; reinstall Colossus")?;
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                return Err("managed ripgrep directory is invalid; reinstall Colossus");
            }
        }
        version_directory.join(name)
    } else if executable
        .file_stem()
        .is_some_and(|stem| stem == "colossus-sidecar")
        || (executable
            .file_stem()
            .is_some_and(|stem| stem == "colossus")
            && parent
                .join(if cfg!(windows) {
                    "colossus-sidecar.exe"
                } else {
                    "colossus-sidecar"
                })
                .is_file())
        || bundled_marker == Some("1")
    {
        parent.join(name)
    } else {
        return Ok(None);
    };
    let metadata = fs::symlink_metadata(&candidate)
        .map_err(|_| "managed ripgrep is missing; reinstall Colossus")?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err("managed ripgrep is invalid; reinstall Colossus");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err("managed ripgrep is not executable; reinstall Colossus");
        }
    }
    fs::canonicalize(candidate)
        .map(Some)
        .map_err(|_| "managed ripgrep cannot be resolved; reinstall Colossus")
}

pub(super) fn is_ripgrep_name(name: &str) -> bool {
    name == "rg" || (cfg!(windows) && name.eq_ignore_ascii_case("rg.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_build_does_not_claim_a_system_ripgrep() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let executable = directory.path().join("colossus");
        assert_eq!(managed_ripgrep_at(&executable, "1.2.3", None), Ok(None));
    }

    #[test]
    fn direct_install_missing_tool_is_an_error() {
        let directory = tempfile::tempdir().expect("temporary directory");
        fs::create_dir(directory.path().join(".colossus-tools")).expect("tool root");
        let executable = directory.path().join("colossus");
        assert_eq!(
            managed_ripgrep_at(&executable, "1.2.3", None),
            Err("managed ripgrep is missing; reinstall Colossus")
        );
    }

    #[test]
    fn packaged_tool_precedes_a_system_copy() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let tools = directory
            .path()
            .join("prefix with spaces/.colossus-tools/1.2.3");
        fs::create_dir_all(&tools).expect("tool directory");
        let name = if cfg!(windows) { "rg.exe" } else { "rg" };
        let ripgrep = tools.join(name);
        fs::write(&ripgrep, "fixture").expect("ripgrep fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&ripgrep, fs::Permissions::from_mode(0o755))
                .expect("executable fixture");
        }
        let executable = directory.path().join("prefix with spaces/colossus");
        assert_eq!(
            managed_ripgrep_at(&executable, "1.2.3", None),
            Ok(Some(fs::canonicalize(ripgrep).expect("canonical ripgrep")))
        );
    }

    #[test]
    fn desktop_bundled_cli_resolves_its_sibling_tool() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let sidecar = directory.path().join(if cfg!(windows) {
            "colossus-sidecar.exe"
        } else {
            "colossus-sidecar"
        });
        fs::write(sidecar, "fixture").expect("sidecar fixture");
        let ripgrep = directory
            .path()
            .join(if cfg!(windows) { "rg.exe" } else { "rg" });
        fs::write(&ripgrep, "fixture").expect("ripgrep fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&ripgrep, fs::Permissions::from_mode(0o755))
                .expect("executable fixture");
        }
        let cli = directory.path().join(if cfg!(windows) {
            "colossus.exe"
        } else {
            "colossus"
        });
        assert_eq!(
            managed_ripgrep_at(&cli, "1.2.3", None),
            Ok(Some(fs::canonicalize(ripgrep).expect("canonical ripgrep")))
        );
    }

    #[cfg(unix)]
    #[test]
    fn linked_managed_tool_is_rejected() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().expect("temporary directory");
        let tools = directory.path().join(".colossus-tools/1.2.3");
        fs::create_dir_all(&tools).expect("tool directory");
        symlink("/bin/sh", tools.join("rg")).expect("linked tool");
        assert_eq!(
            managed_ripgrep_at(&directory.path().join("colossus"), "1.2.3", None),
            Err("managed ripgrep is invalid; reinstall Colossus")
        );
    }

    #[cfg(unix)]
    #[test]
    fn linked_managed_directory_is_rejected() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().expect("temporary directory");
        let external = directory.path().join("external");
        fs::create_dir(&external).expect("external directory");
        symlink(&external, directory.path().join(".colossus-tools"))
            .expect("linked tool directory");
        assert_eq!(
            managed_ripgrep_at(&directory.path().join("colossus"), "1.2.3", None),
            Err("managed ripgrep directory is invalid; reinstall Colossus")
        );
    }
}
