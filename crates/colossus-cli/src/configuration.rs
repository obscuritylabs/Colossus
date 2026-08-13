use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConfigSource {
    Explicit,
    Workspace,
    Global,
}

impl ConfigSource {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Workspace => "workspace",
            Self::Global => "global",
        }
    }

    pub(super) const fn scope(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Workspace => "local",
            Self::Global => "global",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ConfigSelection {
    pub(super) path: PathBuf,
    pub(super) source: ConfigSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ConfigInitTarget {
    pub(super) config_path: PathBuf,
    pub(super) storage_location: StorageLocation,
    storage_path: PathBuf,
    resolved_state_path: PathBuf,
    anchor_path: PathBuf,
    resolved_anchor_path: PathBuf,
    confined_config_root: Option<ConfinedRoot>,
    workspace_config_root: Option<PathBuf>,
}

pub(super) fn select_config(
    explicit: Option<&Path>,
    workspace: &Path,
    home: &ColossusHome,
) -> Result<ConfigSelection, Box<dyn Error>> {
    if let Some(explicit) = explicit {
        return Ok(ConfigSelection {
            path: workspace_path(workspace, explicit),
            source: ConfigSource::Explicit,
        });
    }
    let workspace_config = workspace.join(".colossus/config.yaml");
    if open_workspace_config(workspace)?.is_some() {
        return Ok(ConfigSelection {
            path: workspace_config,
            source: ConfigSource::Workspace,
        });
    }
    let global_config = home.config_path();
    if open_global_config(home)?.is_some() {
        return Ok(ConfigSelection {
            path: global_config,
            source: ConfigSource::Global,
        });
    }
    Err(format!(
        "no Colossus configuration found; run `colossus config init` for {} or `colossus config init --local` for {}",
        global_config.display(),
        workspace_config.display()
    )
    .into())
}

pub(super) fn config_init_target(
    explicit: Option<&Path>,
    local: bool,
    workspace: &Path,
    home: &ColossusHome,
    home_workspace: &Path,
    development: bool,
) -> ConfigInitTarget {
    let state_name = if development {
        "state.dev.redb"
    } else {
        "state.redb"
    };
    let anchor_name = if development {
        "secure-anchor.dev.json"
    } else {
        "secure-anchor.json"
    };
    if let Some(explicit) = explicit {
        let config_path = workspace_path(workspace, explicit);
        let parent = config_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(workspace)
            .to_owned();
        return ConfigInitTarget {
            config_path,
            storage_location: StorageLocation::Workspace,
            storage_path: parent.join(state_name),
            resolved_state_path: parent.join(state_name),
            anchor_path: parent.join(anchor_name),
            resolved_anchor_path: parent.join(anchor_name),
            confined_config_root: None,
            workspace_config_root: None,
        };
    }
    if local {
        let storage_path = PathBuf::from(".colossus").join(state_name);
        let anchor_path = PathBuf::from(".colossus").join(anchor_name);
        return ConfigInitTarget {
            config_path: workspace.join(".colossus/config.yaml"),
            storage_location: StorageLocation::Workspace,
            resolved_state_path: workspace.join(&storage_path),
            resolved_anchor_path: workspace.join(&anchor_path),
            storage_path,
            anchor_path,
            confined_config_root: None,
            workspace_config_root: Some(workspace.to_owned()),
        };
    }
    ConfigInitTarget {
        config_path: home.config_path(),
        storage_location: StorageLocation::HomeWorkspace,
        storage_path: state_name.into(),
        resolved_state_path: home_workspace.join(state_name),
        anchor_path: anchor_name.into(),
        resolved_anchor_path: home_workspace.join(anchor_name),
        confined_config_root: Some(home.confined_root().clone()),
        workspace_config_root: None,
    }
}

pub(super) fn validate_config_init_scope(
    explicit: Option<&Path>,
    local: bool,
) -> Result<(), Box<dyn Error>> {
    if explicit.is_some() && local {
        Err("config init --local cannot be combined with an explicit --config path".into())
    } else {
        Ok(())
    }
}

fn workspace_path(workspace: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        workspace.join(path)
    }
}

pub(super) fn load_selected_config(
    selection: &ConfigSelection,
    workspace: &Path,
    home: &ColossusHome,
) -> Result<RuntimeConfig, Box<dyn Error>> {
    match selection.source {
        ConfigSource::Explicit => Ok(RuntimeConfig::from_path(&selection.path)?),
        ConfigSource::Workspace => {
            let file = open_workspace_config(workspace)?.ok_or_else(|| {
                cli_error("the selected repository configuration disappeared before it was read")
            })?;
            runtime_config_from_file(file)
        }
        ConfigSource::Global => {
            let file = open_global_config(home)?.ok_or_else(|| {
                cli_error("the selected global configuration disappeared before it was read")
            })?;
            runtime_config_from_file(file)
        }
    }
}

fn runtime_config_from_file(mut file: fs::File) -> Result<RuntimeConfig, Box<dyn Error>> {
    let mut yaml = String::new();
    file.read_to_string(&mut yaml)?;
    Ok(RuntimeConfig::from_yaml(&yaml)?)
}

fn open_global_config(home: &ColossusHome) -> Result<Option<fs::File>, Box<dyn Error>> {
    match home
        .confined_root()
        .open_existing_file(Path::new("config.yaml"))
    {
        Ok(file) => Ok(Some(file.into_file())),
        Err(HomeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("global configuration is unsafe: {error}").into()),
    }
}

#[cfg(unix)]
fn create_workspace_config(workspace: &Path, contents: &[u8]) -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::MetadataExt as _;

    let identity = detect_workspace_identity(workspace)?;
    identity.revalidate()?;
    let directory = rustix::fs::open(
        identity.canonical_path(),
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map(fs::File::from)?;
    let config_directory = match rustix::fs::openat(
        &directory,
        ".colossus",
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    ) {
        Ok(directory) => fs::File::from(directory),
        Err(error) if error == rustix::io::Errno::NOENT => {
            rustix::fs::mkdirat(
                &directory,
                ".colossus",
                rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR | rustix::fs::Mode::XUSR,
            )?;
            fs::File::from(rustix::fs::openat(
                &directory,
                ".colossus",
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )?)
        }
        Err(error) => return Err(error.into()),
    };
    let mut file = fs::File::from(rustix::fs::openat(
        &config_directory,
        "config.yaml",
        rustix::fs::OFlags::WRONLY
            | rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::EXCL
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )?);
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.nlink() != 1
    {
        return Err(cli_error("repository configuration file is unsafe").into());
    }
    file.write_all(contents)?;
    file.sync_all()?;
    identity.revalidate()?;
    Ok(())
}

#[cfg(windows)]
fn create_workspace_config(workspace: &Path, contents: &[u8]) -> Result<(), Box<dyn Error>> {
    let identity = detect_workspace_identity(workspace)?;
    identity.revalidate()?;
    let directory = identity.canonical_path().join(".colossus");
    if !directory.exists() {
        colossus_windows_native::create_private_directory(&directory)?;
    }
    let binding = colossus_windows_native::BoundPath::open_directory(&directory)?;
    binding.revalidate()?;
    colossus_windows_native::create_private_file(&directory.join("config.yaml"), contents)?;
    binding.revalidate()?;
    identity.revalidate()?;
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn create_workspace_config(workspace: &Path, contents: &[u8]) -> Result<(), Box<dyn Error>> {
    let identity = detect_workspace_identity(workspace)?;
    let directory = identity.canonical_path().join(".colossus");
    fs::create_dir(&directory)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("config.yaml"))?;
    file.write_all(contents)?;
    identity.revalidate()?;
    Ok(())
}

#[cfg(unix)]
fn open_workspace_config(workspace: &Path) -> Result<Option<fs::File>, Box<dyn Error>> {
    use std::os::unix::fs::MetadataExt as _;

    let identity = detect_workspace_identity(workspace)?;
    identity.revalidate()?;
    let directory = rustix::fs::open(
        identity.canonical_path(),
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map(fs::File::from)
    .map_err(|error| cli_error(format!("repository configuration root is unsafe: {error}")))?;
    let config_directory = match rustix::fs::openat(
        &directory,
        ".colossus",
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    ) {
        Ok(directory) => fs::File::from(directory),
        Err(error) if error == rustix::io::Errno::NOENT => {
            identity.revalidate()?;
            return Ok(None);
        }
        Err(error) => {
            return Err(cli_error(format!(
                "repository configuration directory is unsafe: {error}"
            ))
            .into());
        }
    };
    let file = match rustix::fs::openat(
        &config_directory,
        "config.yaml",
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    ) {
        Ok(file) => fs::File::from(file),
        Err(error) if error == rustix::io::Errno::NOENT => {
            identity.revalidate()?;
            return Ok(None);
        }
        Err(error) => {
            return Err(
                cli_error(format!("repository configuration file is unsafe: {error}")).into(),
            );
        }
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(
            cli_error("repository configuration must be a regular single-link file").into(),
        );
    }
    identity.revalidate()?;
    Ok(Some(file))
}

#[cfg(windows)]
fn open_workspace_config(workspace: &Path) -> Result<Option<fs::File>, Box<dyn Error>> {
    let identity = detect_workspace_identity(workspace)?;
    identity.revalidate()?;
    let directory_path = identity.canonical_path().join(".colossus");
    match fs::symlink_metadata(&directory_path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let directory =
        colossus_windows_native::BoundPath::open_directory(&directory_path).map_err(|error| {
            cli_error(format!(
                "repository configuration directory is unsafe: {error}"
            ))
        })?;
    directory.revalidate()?;
    let path = directory_path.join("config.yaml");
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let file = colossus_windows_native::BoundPath::open_file(&path)
        .map_err(|error| cli_error(format!("repository configuration file is unsafe: {error}")))?;
    file.revalidate()?;
    if file.link_count()? != 1 {
        return Err(
            cli_error("repository configuration must be a regular single-link file").into(),
        );
    }
    identity.revalidate()?;
    Ok(Some(file.try_clone_file()?))
}

#[cfg(not(any(unix, windows)))]
fn open_workspace_config(workspace: &Path) -> Result<Option<fs::File>, Box<dyn Error>> {
    let identity = detect_workspace_identity(workspace)?;
    let path = identity.canonical_path().join(".colossus/config.yaml");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            identity.revalidate()?;
            Ok(Some(fs::File::open(path)?))
        }
        Ok(_) => Err(cli_error("repository configuration file is unsafe").into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn config_resolution_report(
    selection: &ConfigSelection,
    home: &ColossusHome,
    workspace_partition_id: &str,
    resolved_state_path: &Path,
) -> Value {
    json!({
        "configSource": selection.source.as_str(),
        "configScope": selection.source.scope(),
        "configPath": selection.path,
        "colossusHome": home.root(),
        "workspacePartitionId": workspace_partition_id,
        "statePath": resolved_state_path,
    })
}

pub(super) fn attach_config_resolution(
    report: &mut Value,
    resolution: &Value,
) -> Result<(), Box<dyn Error>> {
    report
        .as_object_mut()
        .ok_or_else(|| cli_error("effective configuration report is not an object"))?
        .insert("resolution".into(), resolution.clone());
    Ok(())
}

pub(super) fn integration_auth(
    mode: IntegrationAuthMode,
    header: String,
    scheme: Option<String>,
) -> IntegrationAuth {
    match mode {
        IntegrationAuthMode::None => IntegrationAuth::None,
        IntegrationAuthMode::Bearer => IntegrationAuth::Bearer {
            header,
            scheme: scheme.unwrap_or_else(|| "Bearer".into()),
        },
        IntegrationAuthMode::ApiKey => IntegrationAuth::ApiKey { header, scheme },
        IntegrationAuthMode::Basic => IntegrationAuth::Basic { header },
        IntegrationAuthMode::ServiceAccount => IntegrationAuth::ServiceAccount { header },
    }
}

pub(super) async fn parse_json_argument(
    runtime: &Runtime,
    source: &str,
) -> Result<Value, Box<dyn Error>> {
    let document = if let Some(path) = source.strip_prefix('@') {
        runtime.read_text_file(path).await?
    } else {
        source.to_owned()
    };
    Ok(serde_json::from_str(&document)?)
}

pub(super) fn init_config_at(
    target: &ConfigInitTarget,
    development: bool,
    from: Option<&Path>,
    access_profile: AccessProfile,
    sandbox_profile: Option<SandboxProfile>,
    storage_keys: StorageKeys,
) -> Result<(), Box<dyn Error>> {
    let path = &target.config_path;
    if target.confined_config_root.is_none()
        && target.workspace_config_root.is_none()
        && path.exists()
    {
        return Err(format!("refusing to overwrite {}", path.display()).into());
    }
    if !development && from.is_some() {
        return Err("--from requires --development".into());
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if target.confined_config_root.is_none()
        && target.workspace_config_root.is_none()
        && !parent.exists()
    {
        fs::create_dir_all(parent)?;
    }
    if development
        && (target.resolved_state_path.exists()
            || (storage_keys == StorageKeys::Environment && target.resolved_anchor_path.exists()))
    {
        return Err(format!(
            "refusing to create {} while isolated development state or anchor already exists; restore the matching config or remove both {} and {}",
            path.display(),
            target.resolved_state_path.display(),
            target.resolved_anchor_path.display()
        )
        .into());
    }
    let mut config = if let Some(source) = from {
        RuntimeConfig::from_path(source)?
    } else {
        RuntimeConfig::offline_template(&target.storage_path)
    };
    config.set_access_profile(access_profile);
    config.set_sandbox_profile(
        sandbox_profile
            .unwrap_or_else(|| {
                if access_profile == AccessProfile::Development {
                    SandboxProfile::WorkspaceDevelopment
                } else {
                    SandboxProfile::OfflineDefault
                }
            })
            .as_str(),
    );
    let mut config = if development {
        config.with_isolated_development_storage(&target.storage_path, target.anchor_path.clone())
    } else {
        config
    };
    config.storage.location = target.storage_location;
    match storage_keys {
        StorageKeys::None => config.use_plaintext_storage(),
        StorageKeys::Platform => config.use_platform_storage(),
        StorageKeys::Environment => config.use_environment_storage(&target.anchor_path),
    }
    let encoded = config.to_yaml()?;
    if let Some(root) = &target.confined_config_root {
        let opened = root.open_file(Path::new("config.yaml"))?;
        if !opened.was_created() {
            return Err(format!("refusing to overwrite {}", path.display()).into());
        }
        let mut destination = opened.into_file();
        destination.write_all(encoded.as_bytes())?;
        destination.sync_all()?;
    } else if let Some(workspace) = &target.workspace_config_root {
        create_workspace_config(workspace, encoded.as_bytes())?;
    } else {
        let mut destination = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        destination.write_all(encoded.as_bytes())?;
    }
    println!("created {}", path.display());
    emit_security_posture_warning(&config.security_posture())?;
    Ok(())
}

#[cfg(test)]
pub(super) fn init_config(
    path: &Path,
    development: bool,
    from: Option<&Path>,
    access_profile: AccessProfile,
    sandbox_profile: Option<SandboxProfile>,
    storage_keys: StorageKeys,
) -> Result<(), Box<dyn Error>> {
    let state_name = if development {
        "state.dev.redb"
    } else {
        "state.redb"
    };
    let anchor_name = if development {
        "secure-anchor.dev.json"
    } else {
        "secure-anchor.json"
    };
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let target = ConfigInitTarget {
        config_path: path.to_owned(),
        storage_location: StorageLocation::Workspace,
        storage_path: parent.join(state_name),
        resolved_state_path: parent.join(state_name),
        anchor_path: parent.join(anchor_name),
        resolved_anchor_path: parent.join(anchor_name),
        confined_config_root: None,
        workspace_config_root: None,
    };
    init_config_at(
        &target,
        development,
        from,
        access_profile,
        sandbox_profile,
        storage_keys,
    )
}
