use crate::{DirectUpdateFailure, DirectUpdateInstaller, DirectUpdateOutcome, DirectUpdateRequest};
use async_trait::async_trait;
#[cfg(windows)]
use std::fs;
#[cfg(unix)]
use std::time::Duration;
use std::{fs::OpenOptions, io::Write as _, path::Path, process::Stdio};
use tempfile::{Builder, TempDir};

#[cfg(unix)]
const UPDATE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[cfg(unix)]
const BOOTSTRAP: &[u8] = include_bytes!("../../../release/bootstrap/install.sh");
#[cfg(windows)]
const BOOTSTRAP: &[u8] = include_bytes!("../../../release/bootstrap/install.ps1");

#[cfg(windows)]
const WINDOWS_LAUNCHER: &[u8] = br#"param(
    [Parameter(Mandatory = $true)][int]$ParentProcessId,
    [Parameter(Mandatory = $true)][string]$Bootstrap,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$Prefix,
    [Parameter(Mandatory = $true)][string]$CleanupRoot
)
$ErrorActionPreference = "Stop"
try {
    Wait-Process -Id $ParentProcessId -ErrorAction SilentlyContinue
    & $Bootstrap -Version $Version -Prefix $Prefix -Channel stable -NoModifyPath -Yes
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    Remove-Item -LiteralPath $CleanupRoot -Recurse -Force -ErrorAction SilentlyContinue
}
"#;

/// Platform adapter that runs the exact repository-owned bootstrap embedded at build time.
#[derive(Clone, Copy, Debug, Default)]
pub struct EmbeddedBootstrapInstaller;

#[async_trait]
impl DirectUpdateInstaller for EmbeddedBootstrapInstaller {
    async fn install(
        &self,
        request: &DirectUpdateRequest,
    ) -> Result<DirectUpdateOutcome, DirectUpdateFailure> {
        install_embedded(request).await
    }
}

fn stage_private_file(path: &Path, bytes: &[u8]) -> Result<(), DirectUpdateFailure> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| DirectUpdateFailure::LaunchFailed)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| DirectUpdateFailure::LaunchFailed)
}

fn staging_directory() -> Result<TempDir, DirectUpdateFailure> {
    Builder::new()
        .prefix("colossus-update.")
        .tempdir()
        .map_err(|_| DirectUpdateFailure::LaunchFailed)
}

#[cfg(unix)]
async fn install_embedded(
    request: &DirectUpdateRequest,
) -> Result<DirectUpdateOutcome, DirectUpdateFailure> {
    let staging = staging_directory()?;
    let bootstrap = staging.path().join("install.sh");
    stage_private_file(&bootstrap, BOOTSTRAP)?;
    let mut child = tokio::process::Command::new("/bin/sh")
        .arg(&bootstrap)
        .args(["--version", &format!("v{}", request.version)])
        .args(["--prefix", &request.prefix])
        .args(["--channel", "stable", "--no-modify-path", "--yes"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| DirectUpdateFailure::LaunchFailed)?;
    let status = match tokio::time::timeout(UPDATE_TIMEOUT, child.wait()).await {
        Ok(result) => result.map_err(|_| DirectUpdateFailure::InstallFailed)?,
        Err(_) => {
            let _ = child.kill().await;
            return Err(DirectUpdateFailure::TimedOut);
        }
    };
    if status.success() {
        Ok(DirectUpdateOutcome::Updated)
    } else {
        Err(DirectUpdateFailure::InstallFailed)
    }
}

#[cfg(windows)]
async fn install_embedded(
    request: &DirectUpdateRequest,
) -> Result<DirectUpdateOutcome, DirectUpdateFailure> {
    use std::os::windows::process::CommandExt as _;

    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    let staging = staging_directory()?;
    let bootstrap = staging.path().join("install.ps1");
    let launcher = staging.path().join("apply-update.ps1");
    stage_private_file(&bootstrap, BOOTSTRAP)?;
    stage_private_file(&launcher, WINDOWS_LAUNCHER)?;
    let cleanup_root = staging.keep();
    let system_root = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or(DirectUpdateFailure::LaunchFailed)?;
    let powershell = fs::canonicalize(
        system_root
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe"),
    )
    .map_err(|_| DirectUpdateFailure::LaunchFailed)?;
    let status = std::process::Command::new(powershell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
        ])
        .arg("-File")
        .arg(&launcher)
        .arg("-ParentProcessId")
        .arg(std::process::id().to_string())
        .arg("-Bootstrap")
        .arg(&bootstrap)
        .arg("-Version")
        .arg(format!("v{}", request.version))
        .arg("-Prefix")
        .arg(&request.prefix)
        .arg("-CleanupRoot")
        .arg(&cleanup_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn();
    if status.is_err() {
        let _ = fs::remove_dir_all(cleanup_root);
        return Err(DirectUpdateFailure::LaunchFailed);
    }
    Ok(DirectUpdateOutcome::Scheduled)
}

#[cfg(not(any(unix, windows)))]
async fn install_embedded(
    _request: &DirectUpdateRequest,
) -> Result<DirectUpdateOutcome, DirectUpdateFailure> {
    Err(DirectUpdateFailure::LaunchFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_bootstrap_keeps_the_fixed_public_origin_and_proxy_policy() {
        let source = std::str::from_utf8(BOOTSTRAP).expect("bootstrap is UTF-8");
        assert!(source.contains("obscuritylabs/Colossus"));
        assert!(source.contains("api.github.com"));
        #[cfg(unix)]
        assert!(source.contains("--noproxy '*'"));
        #[cfg(windows)]
        assert!(source.contains("$handler.UseProxy = $false"));
    }
}
