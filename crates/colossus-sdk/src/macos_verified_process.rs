//! macOS process launch that binds inherited channels to an exact signed image.

use std::{ffi::OsString, io, path::Path, process::ExitStatus, time::Duration};

use colossus_darwin_process::{
    DarwinChild, SpawnedTty, spawn_suspended_pipes,
    spawn_suspended_tty as darwin_spawn_suspended_tty,
};
use security_framework::os::macos::code_signing::{
    Flags, GuestAttributes, SecCode, SecRequirement,
};
use tokio::{fs::File as TokioFile, time::sleep};

use crate::{
    SdkError, SdkResult,
    macos_code_identity::{CodeDirectoryHash, MacosCodeIdentity},
};

const _: () = {
    assert!(
        colossus_darwin_process::DESKTOP_TUI_AUTH_INPUT_FD
            == colossus_sidecar_protocol::DESKTOP_TUI_AUTH_INPUT_FD
    );
    assert!(
        colossus_darwin_process::DESKTOP_TUI_AUTH_OUTPUT_FD
            == colossus_sidecar_protocol::DESKTOP_TUI_AUTH_OUTPUT_FD
    );
};

pub(super) struct BootstrapPipes {
    pub(super) guardian: TokioFile,
    pub(super) responses: TokioFile,
}

pub(super) struct MacosChild {
    inner: DarwinChild,
}

impl MacosChild {
    pub(super) fn id(&self) -> Option<u32> {
        Some(self.inner.pid())
    }

    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.inner.try_wait()
    }

    pub(super) fn start_kill(&mut self) -> io::Result<()> {
        self.inner.start_kill()
    }

    pub(super) async fn wait(&mut self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            sleep(Duration::from_millis(10)).await;
        }
    }
}

/// Spawn an exact path kernel-suspended in a new session, validate its dynamic
/// code object, and resume it only after it matches the manifest-bound identity.
pub(super) async fn spawn_verified(
    path: &Path,
    expected: CodeDirectoryHash,
) -> SdkResult<(MacosChild, BootstrapPipes)> {
    let arguments = [OsString::from("__managed-sidecar-v1")];
    let mut spawned =
        spawn_suspended_pipes(path, &arguments, &[]).map_err(|_| SdkError::SidecarFailed)?;
    let validation = validate_dynamic_identity(spawned.child.pid(), expected).and_then(|()| {
        spawned
            .child
            .resume()
            .map_err(|_| SdkError::IdentityMismatch)
    });
    if let Err(error) = validation {
        let _ = spawned.child.kill_and_reap();
        return Err(error);
    }
    Ok((
        MacosChild {
            inner: spawned.child,
        },
        BootstrapPipes {
            guardian: TokioFile::from_std(spawned.input),
            responses: TokioFile::from_std(spawned.output),
        },
    ))
}

/// Spawn a fixed native executable attached to a PTY and stopped before userspace.
///
/// The caller must validate the returned child's live identity with
/// [`validate_suspended_process`] before calling [`DarwinChild::resume`]. Authentication
/// uses the returned anonymous parent handles; it never traverses the PTY.
pub fn spawn_suspended_tty(
    executable: &Path,
    arguments: &[OsString],
    environment: &[OsString],
    tty: &Path,
) -> SdkResult<SpawnedTty> {
    darwin_spawn_suspended_tty(executable, arguments, environment, tty)
        .map_err(|_| SdkError::SidecarFailed)
}

/// Bind a kernel-suspended direct child to a manifest-verified code identity.
///
/// This function deliberately does not resume the child. The opaque child owner
/// ensures the stop event was observed before this validation boundary.
pub fn validate_suspended_process(
    child: &DarwinChild,
    expected: &MacosCodeIdentity,
) -> SdkResult<()> {
    validate_dynamic_identity(child.pid(), expected.0)
}

fn validate_dynamic_identity(pid: u32, expected: CodeDirectoryHash) -> SdkResult<()> {
    let pid = i32::try_from(pid).map_err(|_| SdkError::IdentityMismatch)?;
    let mut attributes = GuestAttributes::new();
    attributes.set_pid(pid);
    let code = SecCode::copy_guest_with_attribues(None, &attributes, Flags::NONE)
        .map_err(|_| SdkError::IdentityMismatch)?;
    let requirement: SecRequirement = expected
        .requirement()
        .parse()
        .map_err(|_| SdkError::IdentityMismatch)?;
    code.check_validity(
        Flags::STRICT_VALIDATE | Flags::NO_NETWORK_ACCESS,
        &requirement,
    )
    .map_err(|_| SdkError::IdentityMismatch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Sha256Digest, VerifiedExecutable, verify_macos_executable_identity};
    use sha2::{Digest as _, Sha256};
    use std::{fs::File, io::Read as _, os::unix::fs::PermissionsExt as _};

    fn digest(path: &Path) -> [u8; 32] {
        let mut file = File::open(path).expect("open executable");
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).expect("read executable");
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        hasher.finalize().into()
    }

    #[test]
    fn swap_between_manifest_verification_and_spawn_never_runs_unverified_image() {
        let directory = tempfile::tempdir().expect("executable directory");
        let path = directory.path().join("sidecar");
        std::fs::copy(std::env::current_exe().expect("test executable"), &path)
            .expect("copy expected image");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500))
            .expect("expected permissions");
        let executable = VerifiedExecutable::new(&path, Sha256Digest::from_bytes(digest(&path)))
            .expect("verified executable");
        let expected =
            verify_macos_executable_identity(&executable).expect("manifest-bound code identity");

        // Replace the leaf after manifest verification but before spawn. Darwin's
        // start-suspended flag prevents the replacement's first userspace instruction
        // from running before exact dynamic validation rejects its CodeDirectory.
        std::fs::remove_file(&path).expect("remove expected image");
        std::fs::write(&path, b"#!/bin/sh\nprintf unverified-image-ran\n")
            .expect("write replacement script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500))
            .expect("replacement permissions");
        let mut spawned = spawn_suspended_pipes(&path, &[], &[]).expect("suspended replacement");
        let result = validate_suspended_process(&spawned.child, &expected);
        assert!(matches!(result, Err(SdkError::IdentityMismatch)));

        spawned
            .child
            .kill_and_reap()
            .expect("kill and reap rejected replacement");
        drop(spawned.input);
        let mut output = Vec::new();
        spawned
            .output
            .read_to_end(&mut output)
            .expect("read replacement output");
        assert!(output.is_empty(), "replacement executed before validation");
    }
}
