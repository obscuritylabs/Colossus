use super::*;
#[cfg(target_os = "macos")]
use colossus_sidecar_protocol::{
    DESKTOP_TUI_PROTOCOL_VERSION, DesktopTuiAuthenticated, DesktopTuiChildFrame,
    DesktopTuiParentFrame, DesktopTuiReady, WorkspaceIdentity, decode_worker_authentication,
    read_frame, write_frame,
};
#[cfg(target_os = "macos")]
use colossus_worker::WorkerAuthenticationKey;
#[cfg(target_os = "macos")]
use uuid::Uuid;

/// Exact selected workspace object opened by the signed CLI before it requests worker
/// authentication. The retained descriptor is also used as the process working directory,
/// so a pathname flip cannot make the child attest one object while running in another.
#[cfg(target_os = "macos")]
pub(super) struct DesktopTuiWorkspaceBinding {
    directory: fs::File,
    identity: WorkspaceIdentity,
}

#[cfg(target_os = "macos")]
impl DesktopTuiWorkspaceBinding {
    fn open(path: &Path) -> Result<Self, Box<dyn Error>> {
        use std::os::macos::fs::MetadataExt as _;

        let canonical =
            fs::canonicalize(path).map_err(|_| "the native TUI workspace is unavailable")?;
        let before =
            fs::symlink_metadata(path).map_err(|_| "the native TUI workspace is unavailable")?;
        if canonical != path
            || !canonical.is_absolute()
            || canonical.parent().is_none()
            || before.file_type().is_symlink()
            || !before.is_dir()
        {
            return Err("the native TUI workspace is unavailable".into());
        }
        let directory = fs::File::from(
            rustix::fs::open(
                &canonical,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .map_err(|_| "the native TUI workspace is unavailable")?,
        );
        let opened = directory
            .metadata()
            .map_err(|_| "the native TUI workspace is unavailable")?;
        let after = fs::symlink_metadata(&canonical)
            .map_err(|_| "the native TUI workspace is unavailable")?;
        if !opened.is_dir()
            || after.file_type().is_symlink()
            || !after.is_dir()
            || before.st_dev() != opened.st_dev()
            || before.st_ino() != opened.st_ino()
            || before.st_birthtime() != opened.st_birthtime()
            || before.st_birthtime_nsec() != opened.st_birthtime_nsec()
            || after.st_dev() != opened.st_dev()
            || after.st_ino() != opened.st_ino()
            || after.st_birthtime() != opened.st_birthtime()
            || after.st_birthtime_nsec() != opened.st_birthtime_nsec()
        {
            return Err("the native TUI workspace is unavailable".into());
        }
        let identity = WorkspaceIdentity::from_macos_parts(
            opened.st_dev(),
            opened.st_ino(),
            opened.st_birthtime(),
            opened.st_birthtime_nsec(),
        )
        .map_err(|_| "the native TUI workspace is unavailable")?;
        Ok(Self {
            directory,
            identity,
        })
    }

    pub(super) fn enter(&self) -> Result<(), Box<dyn Error>> {
        rustix::process::fchdir(&self.directory)
            .map_err(|_| "the native TUI workspace is unavailable".into())
    }

    fn identity(&self) -> &WorkspaceIdentity {
        &self.identity
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) struct DesktopTuiWorkspaceBinding;

#[cfg(not(target_os = "macos"))]
impl DesktopTuiWorkspaceBinding {
    pub(super) fn enter(&self) -> Result<(), Box<dyn Error>> {
        Err("the native TUI workspace is unsupported on this platform".into())
    }
}

#[cfg(target_os = "macos")]
pub(super) fn bind_desktop_tui_workspace(
    workspace: &Path,
) -> Result<DesktopTuiWorkspaceBinding, Box<dyn Error>> {
    DesktopTuiWorkspaceBinding::open(workspace)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn bind_desktop_tui_workspace(
    _workspace: &Path,
) -> Result<DesktopTuiWorkspaceBinding, Box<dyn Error>> {
    Err("the native TUI workspace is unsupported on this platform".into())
}

#[cfg(target_os = "macos")]
fn desktop_tui_ready(
    exchange_id: String,
    workspace: &DesktopTuiWorkspaceBinding,
) -> DesktopTuiChildFrame {
    DesktopTuiChildFrame::Ready(DesktopTuiReady {
        protocol_version: DESKTOP_TUI_PROTOCOL_VERSION,
        exchange_id,
        workspace_identity: workspace.identity().clone(),
    })
}

/// Consume one native-host worker key from fixed inherited anonymous pipes before the
/// ordinary TUI receives terminal input. Authentication never traverses the PTY.
#[cfg(target_os = "macos")]
pub(super) fn inherited_desktop_worker_client(
    config: &RuntimeConfig,
    workspace: &DesktopTuiWorkspaceBinding,
) -> Result<WorkerClient, Box<dyn Error>> {
    let colossus_darwin_process::DesktopTuiAuthenticationChannels {
        mut input,
        mut output,
    } = colossus_darwin_process::take_desktop_tui_authentication_channels()
        .map_err(|_| "the native TUI authentication channel is unavailable")?;

    let exchange_id = Uuid::now_v7().to_string();
    let authentication = (|| {
        write_frame(
            &mut output,
            &desktop_tui_ready(exchange_id.clone(), workspace),
        )?;
        let DesktopTuiParentFrame::Authenticate(request) =
            read_frame::<_, DesktopTuiParentFrame>(&mut input)?;
        request.validate()?;
        if request.exchange_id != exchange_id {
            return Err(colossus_sidecar_protocol::ProtocolError::InvalidFrame);
        }
        let authentication = decode_worker_authentication(&request.worker_ipc_authentication)?;
        write_frame(
            &mut output,
            &DesktopTuiChildFrame::Authenticated(DesktopTuiAuthenticated {
                protocol_version: DESKTOP_TUI_PROTOCOL_VERSION,
                exchange_id,
            }),
        )?;
        Ok(WorkerAuthenticationKey::from_zeroizing(authentication))
    })()
    .map_err(|_| "the native TUI authentication exchange was rejected")?;
    WorkerClient::from_config_with_authentication(config, authentication)
        .map_err(|error| error.into())
}

#[cfg(not(target_os = "macos"))]
pub(super) fn inherited_desktop_worker_client(
    _config: &RuntimeConfig,
    _workspace: &DesktopTuiWorkspaceBinding,
) -> Result<WorkerClient, Box<dyn Error>> {
    Err("the native TUI authentication channel is unsupported on this platform".into())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn desktop_workspace_binding_attests_the_securely_opened_object() {
        use std::os::macos::fs::MetadataExt as _;

        let workspace = tempfile::tempdir().expect("workspace");
        let workspace = fs::canonicalize(workspace.path()).expect("canonical workspace");
        let metadata = fs::symlink_metadata(&workspace).expect("workspace metadata");
        let binding = DesktopTuiWorkspaceBinding::open(&workspace).expect("workspace binding");

        assert_eq!(
            binding.identity(),
            &WorkspaceIdentity::from_macos_parts(
                metadata.st_dev(),
                metadata.st_ino(),
                metadata.st_birthtime(),
                metadata.st_birthtime_nsec(),
            )
            .expect("current identity")
        );
        let DesktopTuiChildFrame::Ready(ready) =
            desktop_tui_ready(Uuid::now_v7().to_string(), &binding)
        else {
            panic!("wrong desktop TUI frame");
        };
        assert_eq!(&ready.workspace_identity, binding.identity());
        assert!(!format!("{ready:?}").contains(&ready.workspace_identity.sha256));
    }

    #[test]
    fn desktop_workspace_binding_does_not_follow_a_same_path_replacement() {
        let root = tempfile::tempdir().expect("workspace parent");
        let workspace = root.path().join("workspace");
        let moved = root.path().join("workspace-moved");
        fs::create_dir(&workspace).expect("workspace");
        let workspace = fs::canonicalize(workspace).expect("canonical workspace");
        let original = DesktopTuiWorkspaceBinding::open(&workspace).expect("original binding");

        fs::rename(&workspace, &moved).expect("move original");
        fs::create_dir(&workspace).expect("replacement");
        let replacement =
            DesktopTuiWorkspaceBinding::open(&workspace).expect("replacement binding");

        assert_ne!(original.identity(), replacement.identity());
    }
}
