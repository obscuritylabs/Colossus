use crate::{ConfinedRoot, HomeError, WorkspaceIdentityRef};
use directories::BaseDirs;
use sha2::{Digest as _, Sha256};
use std::{
    env, fs,
    path::{Component, Path, PathBuf},
};

const HOME_ENVIRONMENT: &str = "COLOSSUS_HOME";
const PARTITION_DOMAIN: &[u8] = b"colossus-home-workspace-partition-v1\0";

/// Isolated application surface below a workspace partition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HomeSurface {
    /// CLI and TUI state.
    Cli,
    /// Native Desktop state.
    Desktop,
}

impl HomeSurface {
    const fn directory_name(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Desktop => "desktop",
        }
    }
}

/// Validated owner-private Colossus home.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColossusHome {
    root: ConfinedRoot,
}

impl ColossusHome {
    /// Resolve `COLOSSUS_HOME`, falling back to `~/.colossus`, and ensure it is private.
    pub fn resolve_and_ensure() -> Result<Self, HomeError> {
        let root = resolve_root(
            env::var_os(HOME_ENVIRONMENT),
            BaseDirs::new().map(|directories| directories.home_dir().to_owned()),
        )?;
        Self::ensure_at(root)
    }

    /// Ensure and validate one explicit absolute home path.
    pub fn ensure_at(root: impl Into<PathBuf>) -> Result<Self, HomeError> {
        let root = root.into();
        if !root.is_absolute() {
            return Err(HomeError::HomeMustBeAbsolute(root));
        }
        Ok(Self {
            root: ConfinedRoot::bind(root)?,
        })
    }

    /// Absolute validated home root.
    pub fn root(&self) -> &Path {
        self.root.path()
    }

    /// Retained filesystem authority for paths beneath this home.
    pub fn confined_root(&self) -> &ConfinedRoot {
        &self.root
    }

    /// Global configuration path.
    pub fn config_path(&self) -> PathBuf {
        self.root.path().join("config.yaml")
    }

    /// Global user instruction path.
    pub fn agents_path(&self) -> PathBuf {
        self.root.path().join("AGENTS.md")
    }

    /// Ensure and return the native Desktop application directory.
    pub fn desktop_root(&self) -> Result<PathBuf, HomeError> {
        self.root.prepare_directory(Path::new("desktop"))
    }

    /// Derive the stable partition identifier for a canonical workspace and object identity.
    pub fn workspace_partition_id(
        &self,
        canonical_workspace: &Path,
        identity: WorkspaceIdentityRef<'_>,
    ) -> Result<String, HomeError> {
        identity.validate()?;
        let verified_canonical = fs::canonicalize(canonical_workspace)
            .map_err(|error| HomeError::io(canonical_workspace, error))?;
        let metadata = fs::symlink_metadata(canonical_workspace)
            .map_err(|error| HomeError::io(canonical_workspace, error))?;
        if !canonical_workspace.is_absolute()
            || canonical_workspace
                .components()
                .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
            || verified_canonical != canonical_workspace
            || metadata.file_type().is_symlink()
            || !metadata.is_dir()
        {
            return Err(HomeError::InvalidWorkspace(canonical_workspace.to_owned()));
        }
        let mut digest = Sha256::new();
        digest.update(PARTITION_DOMAIN);
        update_path_digest(&mut digest, canonical_workspace);
        digest.update(identity.version.to_le_bytes());
        digest.update(identity.sha256.as_bytes());
        Ok(hex::encode(digest.finalize()))
    }

    /// Ensure and return one isolated workspace surface directory.
    pub fn workspace_surface_dir(
        &self,
        canonical_workspace: &Path,
        identity: WorkspaceIdentityRef<'_>,
        surface: HomeSurface,
    ) -> Result<PathBuf, HomeError> {
        let partition = self.workspace_partition_id(canonical_workspace, identity)?;
        self.root.prepare_directory(
            &Path::new("workspaces")
                .join(partition)
                .join(surface.directory_name()),
        )
    }
}

#[cfg(unix)]
fn update_path_digest(digest: &mut Sha256, path: &Path) {
    use std::os::unix::ffi::OsStrExt as _;
    digest.update(path.as_os_str().as_bytes());
}

#[cfg(windows)]
fn update_path_digest(digest: &mut Sha256, path: &Path) {
    use std::os::windows::ffi::OsStrExt as _;
    for unit in path.as_os_str().encode_wide() {
        digest.update(unit.to_le_bytes());
    }
}

#[cfg(not(any(unix, windows)))]
fn update_path_digest(digest: &mut Sha256, path: &Path) {
    digest.update(path.to_string_lossy().as_bytes());
}

#[cfg(unix)]
pub(crate) fn ensure_private_directory(path: &Path) -> Result<(), HomeError> {
    use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _};

    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(HomeError::UnsafePrivateDirectory(current));
                }
                // Reject namespaces where another process could rename or replay an
                // otherwise private home. Sticky shared directories such as /tmp keep
                // their normal per-entry ownership protection.
                if !safe_unix_ancestor_authority(
                    metadata.uid(),
                    metadata.mode(),
                    rustix::process::geteuid().as_raw(),
                ) {
                    return Err(HomeError::UnsafePrivateDirectory(current));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut builder = fs::DirBuilder::new();
                builder.mode(0o700);
                match builder.create(&current) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(HomeError::io(&current, error)),
                }
                let metadata = fs::symlink_metadata(&current)
                    .map_err(|error| HomeError::io(&current, error))?;
                if metadata.file_type().is_symlink()
                    || !metadata.is_dir()
                    || metadata.uid() != rustix::process::geteuid().as_raw()
                    || metadata.mode() & 0o077 != 0
                {
                    return Err(HomeError::UnsafePrivateDirectory(current));
                }
            }
            Err(error) => return Err(HomeError::io(&current, error)),
        }
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| HomeError::io(path, error))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(HomeError::UnsafePrivateDirectory(path.to_owned()));
    }
    Ok(())
}

#[cfg(unix)]
const fn safe_unix_ancestor_authority(owner: u32, mode: u32, effective_user: u32) -> bool {
    (owner == 0 || owner == effective_user) && (mode & 0o022 == 0 || mode & 0o1000 != 0)
}

fn resolve_root(
    environment: Option<std::ffi::OsString>,
    user_home: Option<PathBuf>,
) -> Result<PathBuf, HomeError> {
    match environment {
        Some(value) => {
            let root = PathBuf::from(value);
            if root.is_absolute() {
                Ok(root)
            } else {
                Err(HomeError::HomeMustBeAbsolute(root))
            }
        }
        None => user_home
            .map(|home| home.join(".colossus"))
            .ok_or(HomeError::HomeDirectoryUnavailable),
    }
}

#[cfg(windows)]
pub(crate) fn ensure_private_directory(path: &Path) -> Result<(), HomeError> {
    if !path.exists() {
        let parent = path
            .parent()
            .ok_or_else(|| HomeError::UnsafePrivateDirectory(path.to_owned()))?;
        if !parent.exists() {
            ensure_private_directory(parent)?;
        }
        let parent_binding = colossus_windows_native::BoundPath::open_directory(parent)
            .map_err(|_| HomeError::UnsafePrivateDirectory(parent.to_owned()))?;
        parent_binding
            .validate_namespace_authority()
            .and_then(|()| parent_binding.revalidate())
            .map_err(|_| HomeError::UnsafePrivateDirectory(parent.to_owned()))?;
        match colossus_windows_native::create_private_directory(path) {
            Ok(()) => {}
            Err(_) if path.exists() => {}
            Err(_) => return Err(HomeError::UnsafePrivateDirectory(path.to_owned())),
        }
        parent_binding
            .revalidate()
            .map_err(|_| HomeError::UnsafePrivateDirectory(parent.to_owned()))?;
    }
    let binding = colossus_windows_native::BoundPath::open_directory(path)
        .map_err(|_| HomeError::UnsafePrivateDirectory(path.to_owned()))?;
    binding
        .validate_ancestor_namespace_authority()
        .and_then(|()| binding.validate_private_owner_dacl())
        .and_then(|()| binding.revalidate())
        .map_err(|_| HomeError::UnsafePrivateDirectory(path.to_owned()))
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn ensure_private_directory(path: &Path) -> Result<(), HomeError> {
    fs::create_dir_all(path).map_err(|error| HomeError::io(path, error))?;
    let metadata = fs::symlink_metadata(path).map_err(|error| HomeError::io(path, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(HomeError::UnsafePrivateDirectory(path.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect_workspace_identity;

    #[test]
    fn explicit_home_builds_isolated_stable_workspace_surfaces() {
        let temporary = tempfile::tempdir().expect("temporary root");
        let canonical_temporary = temporary.path().canonicalize().expect("canonical root");
        let home = ColossusHome::ensure_at(canonical_temporary.join("home")).expect("home");
        let workspace = tempfile::tempdir().expect("workspace");
        let identity = detect_workspace_identity(workspace.path()).expect("identity");
        let cli = home
            .workspace_surface_dir(
                identity.canonical_path(),
                identity.as_ref(),
                HomeSurface::Cli,
            )
            .expect("CLI surface");
        let desktop = home
            .workspace_surface_dir(
                identity.canonical_path(),
                identity.as_ref(),
                HomeSurface::Desktop,
            )
            .expect("Desktop surface");
        assert_eq!(cli.parent(), desktop.parent());
        assert_ne!(cli, desktop);
        assert_eq!(cli.file_name().and_then(|name| name.to_str()), Some("cli"));
        assert_eq!(
            desktop.file_name().and_then(|name| name.to_str()),
            Some("desktop")
        );
        fs::create_dir(identity.canonical_path().join("nested")).expect("nested workspace dir");
        // Join the indirect path textually: `PathBuf::push` removes `..` from Windows
        // verbatim paths, which would collapse this back onto the canonical path.
        let mut indirect = identity.canonical_path().as_os_str().to_owned();
        indirect.push(std::path::MAIN_SEPARATOR_STR);
        indirect.push("nested");
        indirect.push(std::path::MAIN_SEPARATOR_STR);
        indirect.push("..");
        let indirect = PathBuf::from(indirect);
        assert_ne!(indirect, identity.canonical_path());
        assert!(
            home.workspace_partition_id(&indirect, identity.as_ref())
                .is_err(),
            "partition callers must supply the exact canonical path"
        );
    }

    #[test]
    fn same_path_object_replacement_gets_a_new_partition() {
        let temporary = tempfile::tempdir().expect("temporary root");
        let root = temporary.path().canonicalize().expect("canonical root");
        let home = ColossusHome::ensure_at(root.join("home")).expect("home");
        let workspace = root.join("workspace");
        fs::create_dir(&workspace).expect("workspace");
        let first = detect_workspace_identity(&workspace).expect("first identity");
        let first_partition = home
            .workspace_partition_id(first.canonical_path(), first.as_ref())
            .expect("first partition");

        fs::rename(&workspace, root.join("displaced-workspace")).expect("displace workspace");
        fs::create_dir(&workspace).expect("replacement workspace");
        let replacement = detect_workspace_identity(&workspace).expect("replacement identity");
        let replacement_partition = home
            .workspace_partition_id(replacement.canonical_path(), replacement.as_ref())
            .expect("replacement partition");
        assert_ne!(first.sha256(), replacement.sha256());
        assert_ne!(first_partition, replacement_partition);
    }

    #[test]
    fn renamed_workspace_object_gets_a_new_path_bound_partition() {
        let temporary = tempfile::tempdir().expect("temporary root");
        let root = temporary.path().canonicalize().expect("canonical root");
        let home = ColossusHome::ensure_at(root.join("home")).expect("home");
        let workspace = root.join("workspace");
        let renamed_workspace = root.join("renamed-workspace");
        fs::create_dir(&workspace).expect("workspace");
        let before = detect_workspace_identity(&workspace).expect("identity before rename");
        let before_partition = home
            .workspace_partition_id(before.canonical_path(), before.as_ref())
            .expect("partition before rename");

        fs::rename(&workspace, &renamed_workspace).expect("rename workspace");
        let after =
            detect_workspace_identity(&renamed_workspace).expect("identity after workspace rename");
        let after_partition = home
            .workspace_partition_id(after.canonical_path(), after.as_ref())
            .expect("partition after rename");

        assert_eq!(before.sha256(), after.sha256());
        assert_ne!(before.canonical_path(), after.canonical_path());
        assert_ne!(before_partition, after_partition);
    }

    #[cfg(unix)]
    #[test]
    fn replacing_the_bound_home_namespace_fails_closed() {
        use std::os::unix::fs::PermissionsExt as _;

        let temporary = tempfile::tempdir().expect("temporary root");
        let root = temporary.path().canonicalize().expect("canonical root");
        let home_path = root.join("home");
        let home = ColossusHome::ensure_at(&home_path).expect("home");
        fs::rename(&home_path, root.join("displaced-home")).expect("displace home");
        fs::create_dir(&home_path).expect("replacement home");
        fs::set_permissions(&home_path, fs::Permissions::from_mode(0o700))
            .expect("replacement permissions");

        assert!(home.desktop_root().is_err());
    }

    #[test]
    fn relative_explicit_home_is_rejected() {
        assert!(matches!(
            ColossusHome::ensure_at(".colossus"),
            Err(HomeError::HomeMustBeAbsolute(_))
        ));
    }

    #[test]
    fn empty_colossus_home_override_is_rejected() {
        assert!(matches!(
            resolve_root(Some(std::ffi::OsString::new()), Some(PathBuf::from("/unused"))),
            Err(HomeError::HomeMustBeAbsolute(path)) if path.as_os_str().is_empty()
        ));
    }

    #[test]
    fn absolute_colossus_home_override_wins_the_default_home() {
        let temporary = tempfile::tempdir().expect("temporary root");
        let root = temporary.path().canonicalize().expect("canonical root");
        let override_path = root.join("override");
        let user_home = root.join("user-home");
        assert_eq!(
            resolve_root(
                Some(override_path.clone().into_os_string()),
                Some(user_home)
            )
            .expect("absolute override"),
            override_path
        );
    }

    #[test]
    fn absent_override_uses_dot_colossus_beneath_the_user_home() {
        let temporary = tempfile::tempdir().expect("temporary root");
        let user_home = temporary.path().canonicalize().expect("canonical home");
        assert_eq!(
            resolve_root(None, Some(user_home.clone())).expect("default home"),
            user_home.join(".colossus")
        );
    }

    #[cfg(unix)]
    #[test]
    fn linked_or_shared_home_is_rejected() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let temporary = tempfile::tempdir().expect("temporary root");
        let shared = temporary.path().join("shared");
        fs::create_dir(&shared).expect("shared directory");
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o755))
            .expect("shared permissions");
        assert!(matches!(
            ColossusHome::ensure_at(&shared),
            Err(HomeError::UnsafePrivateDirectory(_))
        ));

        let target = temporary.path().join("target");
        fs::create_dir(&target).expect("target");
        let linked = temporary.path().join("linked");
        symlink(&target, &linked).expect("link");
        assert!(matches!(
            ColossusHome::ensure_at(&linked),
            Err(HomeError::UnsafePrivateDirectory(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn non_sticky_writable_ancestor_is_rejected_but_sticky_namespace_is_allowed() {
        use std::os::unix::fs::PermissionsExt as _;

        let temporary = tempfile::tempdir().expect("temporary root");
        let root = temporary.path().canonicalize().expect("canonical root");
        let unsafe_parent = root.join("unsafe-parent");
        fs::create_dir(&unsafe_parent).expect("unsafe parent");
        fs::set_permissions(&unsafe_parent, fs::Permissions::from_mode(0o777))
            .expect("unsafe parent permissions");
        assert!(matches!(
            ColossusHome::ensure_at(unsafe_parent.join("home")),
            Err(HomeError::UnsafePrivateDirectory(_))
        ));

        let sticky_parent = root.join("sticky-parent");
        fs::create_dir(&sticky_parent).expect("sticky parent");
        fs::set_permissions(&sticky_parent, fs::Permissions::from_mode(0o1777))
            .expect("sticky parent permissions");
        ColossusHome::ensure_at(sticky_parent.join("home"))
            .expect("sticky shared namespace remains safe");
    }

    #[cfg(unix)]
    #[test]
    fn foreign_owned_ancestor_authority_is_rejected_even_when_not_world_writable() {
        let effective_user = rustix::process::geteuid().as_raw();
        let foreign_user = effective_user.saturating_add(1).max(1);
        assert!(!safe_unix_ancestor_authority(
            foreign_user,
            0o755,
            effective_user
        ));
        assert!(safe_unix_ancestor_authority(0, 0o755, effective_user));
        assert!(safe_unix_ancestor_authority(
            effective_user,
            0o700,
            effective_user
        ));
    }

    #[cfg(windows)]
    #[test]
    fn ordinary_windows_home_spelling_binds_without_verbatim_prefix_rejection() {
        let temporary = tempfile::tempdir().expect("temporary root");
        let home_path = temporary.path().join(".colossus");
        let home = ColossusHome::ensure_at(&home_path).expect("ordinary Windows home");

        assert_eq!(home.root(), home_path);
        home.confined_root()
            .revalidate()
            .expect("retained ordinary path binding");
    }
}
