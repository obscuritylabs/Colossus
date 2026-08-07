use super::*;

pub(super) const WORKSPACE_DEVELOPMENT_PROFILE: &str = "workspace-development";
const MAX_COMMAND_ROOTS: usize = 64;

#[derive(Clone, Debug, Default)]
pub(super) struct DevelopmentSandbox {
    pub(super) filesystem: Vec<FilesystemGrant>,
    pub(super) executables: Vec<PathBuf>,
    pub(super) protected_filesystem: Vec<String>,
    pub(super) shell: Option<PathBuf>,
    pub(super) path: String,
}

pub(super) fn derive_development_sandbox(
    config: &RuntimeConfig,
    workspace: &Path,
) -> Result<DevelopmentSandbox, RuntimeError> {
    if config.sandbox.profile != WORKSPACE_DEVELOPMENT_PROFILE {
        return Ok(DevelopmentSandbox::default());
    }
    if matches!(config.policy, PolicyConfig::Opa { .. }) {
        return Err(RuntimeError::Config(
            "sandbox.profile workspace-development is unavailable with policy.kind opa; OPA must return explicit workspace and executable obligations"
                .into(),
        ));
    }
    if matches!(
        config.sandbox.backend.as_str(),
        "broker" | "external" | "danger_full_access"
    ) {
        return Err(RuntimeError::Config(
            "workspace-development requires an isolating sandbox backend".into(),
        ));
    }
    let protected_paths_supported = match config.sandbox.backend.as_str() {
        "native" => cfg!(any(target_os = "linux", target_os = "macos")),
        "windows_job" => cfg!(target_os = "windows"),
        "oci" => true,
        _ => false,
    };
    if !protected_paths_supported {
        return Err(RuntimeError::Config(format!(
            "sandbox backend {} cannot enforce workspace-development protected-path exclusions on this platform",
            config.sandbox.backend
        )));
    }

    let shell = resolve_platform_shell()?;
    let mut command_roots = platform_command_roots()
        .into_iter()
        .filter_map(|path| fs::canonicalize(path).ok())
        .filter(|path| path.is_dir() && !path.starts_with(workspace))
        .collect::<Vec<_>>();
    if let Some(path) = std::env::var_os("PATH") {
        command_roots.extend(
            std::env::split_paths(&path)
                .filter(|path| path.is_absolute())
                .filter_map(|path| fs::canonicalize(path).ok())
                .filter(|path| path.is_dir() && !path.starts_with(workspace)),
        );
    }
    command_roots.sort();
    command_roots.dedup();
    command_roots.truncate(MAX_COMMAND_ROOTS);

    let mut runtime_roots = platform_runtime_roots()
        .into_iter()
        .filter_map(|path| fs::canonicalize(path).ok())
        .collect::<Vec<_>>();
    runtime_roots.sort();
    runtime_roots.dedup();

    let mut filesystem = vec![FilesystemGrant {
        root: workspace.display().to_string(),
        mode: "write".into(),
    }];
    filesystem.extend(
        command_roots
            .iter()
            .chain(&runtime_roots)
            .map(|path| FilesystemGrant {
                root: path.display().to_string(),
                mode: "read".into(),
            }),
    );
    filesystem.push(FilesystemGrant {
        root: shell.display().to_string(),
        mode: "execute".into(),
    });

    let mut executables = vec![shell.clone()];
    if let Some(git) = resolve_in_roots("git", &command_roots) {
        filesystem.push(FilesystemGrant {
            root: git.display().to_string(),
            mode: "execute".into(),
        });
        executables.push(git);
    }
    dedupe_grants(&mut filesystem);
    executables.sort();
    executables.dedup();

    let control = workspace.join(".colossus");
    fs::create_dir_all(&control)?;
    let protected_filesystem = vec![fs::canonicalize(control)?.display().to_string()];
    let path = std::env::join_paths(&command_roots)
        .map_err(|error| RuntimeError::Config(format!("invalid development PATH: {error}")))?
        .to_string_lossy()
        .into_owned();
    Ok(DevelopmentSandbox {
        filesystem,
        executables,
        protected_filesystem,
        shell: Some(shell),
        path,
    })
}

pub(super) fn development_environment_names() -> Vec<String> {
    ["HOME", "PATH", "TEMP", "TMP", "TMPDIR"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn dedupe_grants(grants: &mut Vec<FilesystemGrant>) {
    grants.sort_by(|left, right| {
        left.root
            .cmp(&right.root)
            .then_with(|| left.mode.cmp(&right.mode))
    });
    grants.dedup_by(|left, right| left.root == right.root && left.mode == right.mode);
}

fn resolve_in_roots(name: &str, roots: &[PathBuf]) -> Option<PathBuf> {
    roots.iter().find_map(|root| {
        let candidate = root.join(name);
        fs::canonicalize(candidate)
            .ok()
            .filter(|path| path.is_file())
    })
}

#[cfg(target_os = "macos")]
fn resolve_platform_shell() -> Result<PathBuf, RuntimeError> {
    resolve_first(&["/bin/zsh", "/bin/sh"])
}

#[cfg(target_os = "linux")]
fn resolve_platform_shell() -> Result<PathBuf, RuntimeError> {
    resolve_first(&["/bin/bash", "/bin/sh"])
}

#[cfg(target_os = "windows")]
fn resolve_platform_shell() -> Result<PathBuf, RuntimeError> {
    let system_root = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .ok_or_else(|| RuntimeError::Config("Windows system root is unavailable".into()))?;
    resolve_first_paths(&[
        Path::new(&system_root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe"),
        Path::new(&system_root).join("System32").join("cmd.exe"),
    ])
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn resolve_platform_shell() -> Result<PathBuf, RuntimeError> {
    Err(RuntimeError::Config(
        "workspace-development has no supported platform shell".into(),
    ))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn resolve_first(candidates: &[&str]) -> Result<PathBuf, RuntimeError> {
    resolve_first_paths(&candidates.iter().map(PathBuf::from).collect::<Vec<_>>())
}

fn resolve_first_paths(candidates: &[PathBuf]) -> Result<PathBuf, RuntimeError> {
    candidates
        .iter()
        .find_map(|path| fs::canonicalize(path).ok().filter(|path| path.is_file()))
        .ok_or_else(|| RuntimeError::Config("no trusted platform shell is available".into()))
}

#[cfg(target_os = "macos")]
fn platform_command_roots() -> Vec<PathBuf> {
    ["/bin", "/sbin", "/usr/bin", "/usr/sbin"]
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

#[cfg(target_os = "linux")]
fn platform_command_roots() -> Vec<PathBuf> {
    ["/bin", "/sbin", "/usr/bin", "/usr/sbin"]
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

#[cfg(target_os = "windows")]
fn platform_command_roots() -> Vec<PathBuf> {
    std::env::var_os("SystemRoot")
        .map_or_else(Vec::new, |root| vec![Path::new(&root).join("System32")])
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn platform_command_roots() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(target_os = "macos")]
fn platform_runtime_roots() -> Vec<PathBuf> {
    ["/System/Library", "/usr/lib", "/Library/Apple/usr/lib"]
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

#[cfg(target_os = "linux")]
fn platform_runtime_roots() -> Vec<PathBuf> {
    ["/lib", "/lib64", "/usr/lib", "/usr/lib64"]
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn platform_runtime_roots() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn development_preset_derives_workspace_shell_and_control_state_protection() {
        let workspace = tempfile::tempdir().expect("workspace");
        let mut config = RuntimeConfig::offline_template("state.redb");
        config.sandbox.profile = WORKSPACE_DEVELOPMENT_PROFILE.into();
        config.sandbox.backend = if cfg!(target_os = "windows") {
            "windows_job".into()
        } else {
            "native".into()
        };

        let derived =
            derive_development_sandbox(&config, workspace.path()).expect("development sandbox");

        assert!(derived.shell.as_ref().is_some_and(|shell| shell.is_file()));
        assert!(
            derived
                .filesystem
                .iter()
                .any(|grant| grant.root == workspace.path().display().to_string()
                    && grant.mode == "write")
        );
        assert_eq!(
            derived.protected_filesystem,
            vec![
                fs::canonicalize(workspace.path().join(".colossus"))
                    .expect("canonical control state")
                    .display()
                    .to_string()
            ]
        );
        assert!(workspace.path().join(".colossus").is_dir());
        assert!(!derived.path.is_empty());
    }

    #[test]
    fn opa_rejects_automatic_development_grants() {
        let workspace = tempfile::tempdir().expect("workspace");
        let mut config = RuntimeConfig::offline_template("state.redb");
        config.sandbox.profile = WORKSPACE_DEVELOPMENT_PROFILE.into();
        config.policy = PolicyConfig::Opa {
            base_url: "https://opa.example".into(),
            decision_path: "/v1/data/colossus/allow".into(),
            ca_pem_path: None,
            identity_pem_path: None,
            full_content_disclosure_acknowledged: true,
            decision_log_masking_verified: true,
            timeout_ms: 5_000,
        };
        let error = derive_development_sandbox(&config, workspace.path())
            .expect_err("OPA development grants");
        assert!(error.to_string().contains("OPA must return explicit"));
    }
}
