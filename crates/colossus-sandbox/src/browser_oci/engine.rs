use std::{os::unix::fs::MetadataExt as _, path::Path, process::Stdio, time::Duration};

use colossus_ports::BrowserDriverError;
use serde::Deserialize;
use tokio::{io::AsyncReadExt as _, process::Command};

use super::Installation;

const MAX_OUTPUT: u64 = 16 * 1024;
const INSPECT: &str = r#"{"id":{{json .Id}},"pid":{{.State.Pid}},"running":{{.State.Running}},"exit_code":{{.State.ExitCode}},"oom_killed":{{.State.OOMKilled}},"image":{{json .Image}},"label":{{json (index .Config.Labels "dev.colossus.browser.nonce")}},"network":{{json .HostConfig.NetworkMode}},"user":{{json .Config.User}}}"#;

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub(super) struct ExecutableIdentity {
    dev: u64,
    inode: u64,
    size: u64,
    mtime: i64,
    mtime_ns: i64,
    ctime: i64,
    ctime_ns: i64,
}
impl ExecutableIdentity {
    pub(super) fn bind(path: &Path, diagnostic: bool) -> Result<Self, BrowserDriverError> {
        let metadata = path
            .symlink_metadata()
            .map_err(|_| BrowserDriverError::Unavailable)?;
        let owner = metadata.uid();
        let diagnostic_mapped_root =
            diagnostic && owner == 65534 && path == Path::new("/usr/local/bin/docker");
        if !path.is_absolute()
            || path
                .canonicalize()
                .map_err(|_| BrowserDriverError::Denied)?
                != path
            || !metadata.is_file()
            || metadata.file_type().is_symlink()
            || (owner != 0 && !diagnostic_mapped_root)
            || metadata.mode() & 0o022 != 0
            || metadata.mode() & 0o111 == 0
        {
            return Err(BrowserDriverError::Denied);
        }
        for parent in path.ancestors().skip(1) {
            let ancestor = parent
                .symlink_metadata()
                .map_err(|_| BrowserDriverError::Denied)?;
            // Production requires actual visible administrator ownership. The
            // explicit developer constructor alone admits this managed fixture's
            // fixed system executable whose host-root UID is displayed unmapped.
            if !ancestor.is_dir()
                || ancestor.file_type().is_symlink()
                || ancestor.mode() & 0o022 != 0
                || (ancestor.uid() != 0 && !(diagnostic_mapped_root && ancestor.uid() == 65534))
            {
                return Err(BrowserDriverError::Denied);
            }
        }
        Ok(Self {
            dev: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.len(),
            mtime: metadata.mtime(),
            mtime_ns: metadata.mtime_nsec(),
            ctime: metadata.ctime(),
            ctime_ns: metadata.ctime_nsec(),
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Inspection {
    pub(super) id: String,
    pub(super) pid: u32,
    pub(super) running: bool,
    pub(super) exit_code: i32,
    pub(super) oom_killed: bool,
    pub(super) image: String,
    pub(super) label: String,
    pub(super) network: String,
    pub(super) user: String,
}

pub(super) fn nonce() -> Result<String, BrowserDriverError> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| BrowserDriverError::Unavailable)?;
    Ok(hex::encode(bytes))
}

pub(super) fn container_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// No ambient Docker context, credential helper, environment, or model-selected endpoint.
/// The retained native supervisor alone has access to this administrator-selected engine.
pub(super) async fn run(
    installation: &Installation,
    arguments: &[String],
) -> Result<(bool, Vec<u8>), BrowserDriverError> {
    installation
        .root
        .revalidate()
        .map_err(|_| BrowserDriverError::Denied)?;
    if ExecutableIdentity::bind(&installation.docker, installation.diagnostic_owner)?
        != installation.docker_identity
    {
        return Err(BrowserDriverError::Denied);
    }
    let mut command = Command::new(&installation.docker);
    command
        .args(["--host", "unix:///var/run/docker.sock", "--config"])
        .arg(&installation.docker_config)
        .args(arguments)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &installation.docker_config)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|_| BrowserDriverError::Unavailable)?;
    let stdout = child
        .stdout
        .take()
        .ok_or(BrowserDriverError::OutcomeUnknown)?;
    let mut bytes = Vec::new();
    let operation = async {
        stdout
            .take(MAX_OUTPUT + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if bytes.len() as u64 > MAX_OUTPUT {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let status = child
            .wait()
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        Ok((status.success(), bytes))
    };
    // OCI creation can materialize a large pinned image (notably the VFS engine).
    // Bound it by the already accepted per-session operation budget rather than
    // an unrelated five-second cutoff which leaves daemon allocation uncertain.
    tokio::time::timeout(
        Duration::from_millis(u64::from(
            installation.capabilities.limits.navigation_timeout_ms,
        )),
        operation,
    )
    .await
    .map_err(|_| BrowserDriverError::OutcomeUnknown)?
}

pub(super) async fn verify_image(installation: &Installation) -> Result<(), BrowserDriverError> {
    let (success, bytes) = run(
        installation,
        &[
            "image".into(),
            "inspect".into(),
            "--format".into(),
            "{{.Id}}".into(),
            installation.image.clone(),
        ],
    )
    .await?;
    if !success
        || std::str::from_utf8(&bytes).map(str::trim).ok() != Some(installation.image.as_str())
    {
        return Err(BrowserDriverError::Unavailable);
    }
    Ok(())
}

pub(super) async fn inspect(
    installation: &Installation,
    identity: &str,
) -> Result<Option<Inspection>, BrowserDriverError> {
    let (success, bytes) = run(
        installation,
        &[
            "container".into(),
            "inspect".into(),
            "--format".into(),
            INSPECT.into(),
            identity.into(),
        ],
    )
    .await?;
    if !success {
        // A failed inspect is not evidence of absence: daemon outages and missing
        // objects share this status. Exact list lookup must independently succeed.
        let filter = if container_id(identity) {
            format!("id={identity}")
        } else {
            format!("name=^/{identity}$")
        };
        let (listed, bytes) = run(
            installation,
            &[
                "container".into(),
                "ls".into(),
                "--all".into(),
                "--no-trunc".into(),
                "--filter".into(),
                filter,
                "--format".into(),
                "{{.ID}}".into(),
            ],
        )
        .await?;
        if !listed || !bytes.is_empty() {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        return Ok(None);
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| BrowserDriverError::OutcomeUnknown)
}

pub(super) fn arguments(
    installation: &Installation,
    name: &str,
    nonce: &str,
    control: &std::path::Path,
    profile: Option<&crate::BrowserProfileLease>,
) -> Result<Vec<String>, BrowserDriverError> {
    let text = |path: &std::path::Path| -> Result<String, BrowserDriverError> {
        let value = path.to_str().ok_or(BrowserDriverError::Denied)?;
        if value.contains([',', '\n', '\r']) {
            return Err(BrowserDriverError::Denied);
        }
        Ok(value.to_owned())
    };
    let bounds = installation.limits;
    let user = format!("{}:{}", installation.uid, installation.gid);
    let mut args = vec![
        "create".into(),
        "--pull=never".into(),
        "--name".into(),
        name.into(),
        "--label".into(),
        format!("dev.colossus.browser.nonce={nonce}"),
        "--network=none".into(),
        "--read-only".into(),
        "--cap-drop=ALL".into(),
        "--security-opt=no-new-privileges".into(),
        "--security-opt".into(),
        format!("seccomp={}", text(&installation.seccomp)?),
        "--user".into(),
        user.clone(),
        "--pids-limit".into(),
        bounds.max_processes.to_string(),
        "--memory".into(),
        bounds.memory_bytes.to_string(),
        "--memory-swap".into(),
        bounds.memory_bytes.to_string(),
        "--cpu-period=100000".into(),
        "--cpu-quota".into(),
        (u64::from(bounds.cpu_millis) * 100).to_string(),
        "--ipc=private".into(),
        "--cgroupns=private".into(),
        "--shm-size=67108864".into(),
        "--ulimit=nofile=1024:1024".into(),
        "--log-driver=none".into(),
        "--env=LANG=C.UTF-8".into(),
        "--mount".into(),
        format!(
            "type=bind,src={},dst=/opt/colossus-browser,readonly,bind-propagation=rprivate",
            text(&installation.component)?
        ),
        "--mount".into(),
        format!(
            "type=bind,src={},dst=/run/colossus-browser-control,readonly,bind-propagation=rprivate",
            text(control)?
        ),
        "--tmpfs".into(),
        format!(
            "/var/colossus-browser:rw,noexec,nosuid,nodev,size={},mode=0700,uid={},gid={}",
            bounds.profile_bytes, installation.uid, installation.gid
        ),
        "--tmpfs".into(),
        format!(
            "/tmp:rw,noexec,nosuid,nodev,size={},mode=1777",
            bounds.temporary_bytes
        ),
        "--entrypoint=/opt/colossus-browser/colossus-native-browser-host".into(),
    ];
    if let Some(profile) = profile {
        profile
            .revalidate()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        // The store supplies a positively bound private cache. The model's
        // opaque profile ID never becomes a mount source or native pathname.
        args.push("--mount".into());
        args.push(format!(
            "type=bind,src={},dst=/var/colossus-browser/profile,bind-propagation=rprivate",
            text(profile.cache_path())?
        ));
    }
    args.push(installation.image.clone());
    args.push(
        if installation.presentation {
            "--oci-presentation-sockets"
        } else {
            "--oci-sockets"
        }
        .into(),
    );
    Ok(args)
}
