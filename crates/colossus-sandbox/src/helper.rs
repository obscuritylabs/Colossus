use super::*;

/// Structured helper failure printed only to the trusted parent process.
#[derive(Debug, Error)]
pub enum SandboxHelperError {
    /// The authenticated job was malformed, expired, or tampered with.
    #[error("invalid sandbox job: {0}")]
    InvalidJob(String),
    /// Native or OCI isolation could not be established.
    #[error("sandbox setup failed: {0}")]
    Setup(String),
    /// The sandboxed command could not be supervised.
    #[error("sandbox execution failed: {0}")]
    Execution(String),
    /// Strict JSON IPC failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Helper standard I/O failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Run the trusted one-shot sandbox helper protocol over stdin/stdout.
pub fn run_helper_stdio() -> Result<(), SandboxHelperError> {
    let encoded_key = std::env::var(HELPER_KEY_VARIABLE)
        .map_err(|_| SandboxHelperError::InvalidJob("helper key is absent".into()))?;
    let key: [u8; 32] = hex::decode(encoded_key)
        .map_err(|error| SandboxHelperError::InvalidJob(error.to_string()))?
        .try_into()
        .map_err(|_| SandboxHelperError::InvalidJob("helper key length is invalid".into()))?;
    let mut input = std::io::stdin().take(
        u64::try_from(MAX_JOB_BYTES)
            .unwrap_or(u64::MAX)
            .saturating_add(1),
    );
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes)?;
    if bytes.len() > MAX_JOB_BYTES {
        return Err(SandboxHelperError::InvalidJob(
            "helper input exceeds IPC bound".into(),
        ));
    }
    let signed: SignedSandboxJob = serde_json::from_slice(&bytes)?;
    let job = signed.verify(&key)?;
    let result = execute_sandbox_job(job, &key)?;
    serde_json::to_writer(std::io::stdout(), &result)?;
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SandboxJobResult {
    pub(super) backend: String,
    pub(super) exit_code: Option<i32>,
    pub(super) success: bool,
    pub(super) timed_out: bool,
    pub(super) resource_limit_exceeded: Option<String>,
    pub(super) output_truncated: bool,
    pub(super) stdout_base64: String,
    pub(super) stderr_base64: String,
    #[serde(default)]
    pub(super) observed_origins: Vec<String>,
}

pub(super) fn execute_sandbox_job(
    job: SandboxJob,
    key: &[u8; 32],
) -> Result<SandboxJobResult, SandboxHelperError> {
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = key;
    let backend = job.obligations.sandbox_backend.clone();
    #[cfg(target_os = "windows")]
    if backend == "oci" {
        return Err(SandboxHelperError::Setup(
            "OCI execution is disabled on Windows until path mapping passes live acceptance".into(),
        ));
    }
    if backend == "windows_job" {
        #[cfg(target_os = "windows")]
        {
            return supervise_windows_job(&job, backend);
        }
        #[cfg(not(target_os = "windows"))]
        {
            return Err(SandboxHelperError::Setup(
                "windows_job is available only on Windows".into(),
            ));
        }
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if backend == "native" && std::env::var_os(NATIVE_INNER_VARIABLE).is_none() {
        return supervise_native_inner(&job, backend, key);
    }
    let mut oci_network = if backend == "oci" && !job.obligations.network_destinations.is_empty() {
        Some(OciNetworkResources::start(&job)?)
    } else {
        None
    };
    let mut command = match backend.as_str() {
        "native" => native_command(&job)?,
        "oci" => oci_command(
            &job,
            oci_network.as_ref().map(OciNetworkResources::proxy_address),
        )?,
        "broker" if job.obligations.allow_sandbox_downgrade => direct_command(&job),
        "broker" => {
            return Err(SandboxHelperError::Setup(
                "broker downgrade was not explicitly authorized".into(),
            ));
        }
        other => {
            return Err(SandboxHelperError::Setup(format!(
                "unknown sandbox backend {other}"
            )));
        }
    };
    let mut result = supervise(&mut command, &job, backend.clone());
    if let Some(network) = oci_network.as_mut() {
        if let Ok(result) = result.as_mut() {
            result.observed_origins = network.observed_origins()?;
        }
        network.cleanup();
    }
    if backend == "oci" && !ensure_oci_resources_absent(&job) {
        return Err(SandboxHelperError::Execution(
            "OCI container or network cleanup could not be confirmed".into(),
        ));
    }
    result
}

pub(super) fn direct_command(job: &SandboxJob) -> Command {
    let mut command = Command::new(&job.executable);
    configure_command(&mut command, job);
    command
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn supervise_native_inner(
    job: &SandboxJob,
    backend: String,
    key: &[u8; 32],
) -> Result<SandboxJobResult, SandboxHelperError> {
    let signed = SignedSandboxJob::sign(job.clone(), key)
        .map_err(|error| SandboxHelperError::InvalidJob(error.to_string()))?;
    let encoded = serde_json::to_vec(&signed)?;
    if encoded.len() > MAX_JOB_BYTES {
        return Err(SandboxHelperError::InvalidJob(
            "native inner job exceeds IPC bound".into(),
        ));
    }
    let executable = std::env::current_exe()
        .map_err(|error| SandboxHelperError::Setup(format!("native helper identity: {error}")))?;
    let mut command = Command::new(executable);
    command
        .arg("__sandbox-helper")
        .env_clear()
        .env(HELPER_KEY_VARIABLE, hex::encode(key))
        .env(NATIVE_INNER_VARIABLE, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    supervise_native_inner_process(&mut command, job, backend, &encoded)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn native_command(job: &SandboxJob) -> Result<Command, SandboxHelperError> {
    if !Sandbox::is_supported() {
        return Err(SandboxHelperError::Setup(
            "native kernel sandbox is unavailable".into(),
        ));
    }
    let mut capabilities = CapabilitySet::new();
    for grant in &job.obligations.filesystem {
        let path = fs::canonicalize(&grant.root)
            .map_err(|error| SandboxHelperError::Setup(error.to_string()))?;
        let access = if grant.mode == "write" {
            AccessMode::ReadWrite
        } else {
            AccessMode::Read
        };
        capabilities = if path.is_dir() {
            capabilities.allow_path(&path, access)
        } else {
            capabilities.allow_file(&path, access)
        }
        .map_err(|error| SandboxHelperError::Setup(error.to_string()))?;
    }
    for path in native_runtime_paths() {
        if !path.exists() {
            continue;
        }
        capabilities = if path.is_dir() {
            capabilities.allow_path(path, AccessMode::Read)
        } else {
            capabilities.allow_file(path, AccessMode::Read)
        }
        .map_err(|error| SandboxHelperError::Setup(error.to_string()))?;
    }
    apply_protected_filesystem(&mut capabilities, job)?;
    capabilities = if let Some(port) = job.proxy_port {
        capabilities.proxy_only(port)
    } else {
        capabilities.block_network()
    };
    Sandbox::apply_auto(&capabilities)
        .map_err(|error| SandboxHelperError::Setup(format!("native apply: {error}")))?;
    let mut command = Command::new(&job.executable);
    configure_command(&mut command, job);
    Ok(command)
}

#[cfg(target_os = "macos")]
fn apply_protected_filesystem(
    capabilities: &mut CapabilitySet,
    job: &SandboxJob,
) -> Result<(), SandboxHelperError> {
    for path in &job.obligations.protected_filesystem {
        let canonical = fs::canonicalize(path)
            .map_err(|error| SandboxHelperError::Setup(format!("protected path: {error}")))?;
        let path = canonical
            .to_str()
            .ok_or_else(|| SandboxHelperError::Setup("protected path is not valid UTF-8".into()))?;
        let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
        capabilities
            .add_platform_rule(format!("(deny file-read* (subpath \"{escaped}\"))"))
            .map_err(|error| SandboxHelperError::Setup(format!("Seatbelt deny rule: {error}")))?;
        capabilities
            .add_platform_rule(format!("(deny file-write* (subpath \"{escaped}\"))"))
            .map_err(|error| SandboxHelperError::Setup(format!("Seatbelt deny rule: {error}")))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn apply_protected_filesystem(
    _capabilities: &mut CapabilitySet,
    job: &SandboxJob,
) -> Result<(), SandboxHelperError> {
    use rustix::{
        mount::{MountFlags, MountPropagationFlags, mount_bind, mount_change, mount_remount},
        process::{getgid, getuid},
        thread::{UnshareFlags, unshare},
    };

    if job.obligations.protected_filesystem.is_empty() {
        return Ok(());
    }
    let uid = getuid().as_raw();
    let gid = getgid().as_raw();
    unshare(UnshareFlags::NEWUSER | UnshareFlags::NEWNS)
        .map_err(|error| SandboxHelperError::Setup(format!("unshare mount namespace: {error}")))?;
    if Path::new("/proc/self/setgroups").exists() {
        fs::write("/proc/self/setgroups", "deny")
            .map_err(|error| SandboxHelperError::Setup(format!("deny setgroups: {error}")))?;
    }
    fs::write("/proc/self/uid_map", format!("0 {uid} 1\n"))
        .map_err(|error| SandboxHelperError::Setup(format!("map sandbox uid: {error}")))?;
    fs::write("/proc/self/gid_map", format!("0 {gid} 1\n"))
        .map_err(|error| SandboxHelperError::Setup(format!("map sandbox gid: {error}")))?;
    mount_change(
        "/",
        MountPropagationFlags::PRIVATE | MountPropagationFlags::REC,
    )
    .map_err(|error| SandboxHelperError::Setup(format!("isolate mount propagation: {error}")))?;

    let mask = std::env::temp_dir().join(format!("colossus-mask-{}", job.job_id));
    fs::create_dir(&mask)
        .map_err(|error| SandboxHelperError::Setup(format!("create path mask: {error}")))?;
    for protected in &job.obligations.protected_filesystem {
        let protected = fs::canonicalize(protected)
            .map_err(|error| SandboxHelperError::Setup(format!("protected path: {error}")))?;
        if !protected.is_dir() {
            return Err(SandboxHelperError::Setup(
                "Linux protected path masking currently requires directories".into(),
            ));
        }
        mount_bind(&mask, &protected).map_err(|error| {
            SandboxHelperError::Setup(format!("bind protected path mask: {error}"))
        })?;
        mount_remount(
            &protected,
            MountFlags::BIND
                | MountFlags::RDONLY
                | MountFlags::NODEV
                | MountFlags::NOEXEC
                | MountFlags::NOSUID,
            "",
        )
        .map_err(|error| {
            SandboxHelperError::Setup(format!("make protected path mask read-only: {error}"))
        })?;
    }
    fs::remove_dir(&mask)
        .map_err(|error| SandboxHelperError::Setup(format!("remove path mask source: {error}")))?;
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn native_command(_job: &SandboxJob) -> Result<Command, SandboxHelperError> {
    Err(SandboxHelperError::Setup(
        "native sandboxing is unsupported on this platform".into(),
    ))
}

#[cfg(target_os = "windows")]
pub(super) struct WindowsProfileGuard(Option<AppContainerProfile>);

#[cfg(target_os = "windows")]
pub(super) struct WindowsTemporaryGuard(Option<PathBuf>);

#[cfg(target_os = "windows")]
pub(super) struct WindowsProtectedAclGuard {
    icacls: PathBuf,
    sid: String,
    paths: Vec<PathBuf>,
}

#[cfg(target_os = "windows")]
impl WindowsProfileGuard {
    fn remove(&mut self) -> Result<(), SandboxHelperError> {
        if let Some(profile) = self.0.take()
            && let Err(error) = profile.clone().delete()
        {
            self.0 = Some(profile);
            return Err(SandboxHelperError::Execution(format!(
                "profile cleanup: {error}"
            )));
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
impl WindowsTemporaryGuard {
    fn create(job: &SandboxJob, package: &AppContainerSid) -> Result<Self, SandboxHelperError> {
        let root = job.temporary_root.as_ref().ok_or_else(|| {
            SandboxHelperError::Setup("authenticated Windows temporary root is absent".into())
        })?;
        let root = fs::canonicalize(root).map_err(|error| {
            SandboxHelperError::Setup(format!("Windows temporary root: {error}"))
        })?;
        if !root.is_dir() {
            return Err(SandboxHelperError::Setup(
                "Windows temporary root is not a directory".into(),
            ));
        }
        let requested = root.join(format!("colossus-sandbox-{}", job.job_id));
        fs::create_dir(&requested).map_err(|error| {
            SandboxHelperError::Setup(format!(
                "create Windows sandbox temporary directory: {error}"
            ))
        })?;
        let mut guard = Self(Some(requested.clone()));
        let temporary = fs::canonicalize(&requested).map_err(|error| {
            SandboxHelperError::Setup(format!("Windows sandbox temporary directory: {error}"))
        })?;
        if temporary.parent() != Some(root.as_path()) {
            return Err(SandboxHelperError::Setup(
                "Windows sandbox temporary directory escaped its authenticated root".into(),
            ));
        }
        guard.0 = Some(temporary.clone());
        acl::grant_to_package(
            ResourcePath::Directory(temporary),
            package,
            AccessMask(0x0012_0089 | 0x0012_0116 | 0x0012_00A0),
        )
        .map_err(|error| SandboxHelperError::Setup(format!("sandbox temp ACL: {error}")))?;
        Ok(guard)
    }

    fn path(&self) -> &Path {
        self.0.as_deref().expect("temporary directory is present")
    }

    fn remove(&mut self) -> Result<(), SandboxHelperError> {
        if let Some(path) = self.0.take() {
            if let Err(error) = fs::remove_dir_all(&path) {
                let message = format!(
                    "remove Windows sandbox temporary directory {}: {error}",
                    path.display()
                );
                self.0 = Some(path);
                return Err(SandboxHelperError::Execution(message));
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
impl WindowsProtectedAclGuard {
    fn create(
        system_root: &Path,
        package: &AppContainerSid,
        paths: &[String],
    ) -> Result<Self, SandboxHelperError> {
        let icacls = fs::canonicalize(system_root.join("System32").join("icacls.exe"))
            .map_err(|error| SandboxHelperError::Setup(format!("Windows icacls: {error}")))?;
        let sid = package.as_string();
        let mut protected = paths
            .iter()
            .map(fs::canonicalize)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| SandboxHelperError::Setup(format!("protected path: {error}")))?;
        protected.sort();
        protected.dedup();
        let mut guard = Self {
            icacls,
            sid,
            paths: Vec::new(),
        };
        for path in protected {
            let inheritance = if path.is_dir() { "(OI)(CI)F" } else { "F" };
            let trustee = format!("*{}:{inheritance}", guard.sid);
            let output = Command::new(&guard.icacls)
                .env_clear()
                .arg(&path)
                .arg("/deny")
                .arg(trustee)
                .arg("/Q")
                .output()
                .map_err(|error| {
                    SandboxHelperError::Setup(format!("protect AppContainer path: {error}"))
                })?;
            if !output.status.success() {
                return Err(SandboxHelperError::Setup(format!(
                    "protect AppContainer path {} failed",
                    path.display()
                )));
            }
            guard.paths.push(path);
        }
        Ok(guard)
    }

    fn remove(&mut self) -> Result<(), SandboxHelperError> {
        while let Some(path) = self.paths.pop() {
            let output = Command::new(&self.icacls)
                .env_clear()
                .arg(&path)
                .arg("/remove:d")
                .arg(format!("*{}", self.sid))
                .arg("/Q")
                .output()
                .map_err(|error| {
                    SandboxHelperError::Execution(format!(
                        "remove protected AppContainer ACL: {error}"
                    ))
                })?;
            if !output.status.success() {
                self.paths.push(path.clone());
                return Err(SandboxHelperError::Execution(format!(
                    "remove protected AppContainer ACL {} failed",
                    path.display()
                )));
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
pub(super) fn collect_windows_acl_targets(
    root: &Path,
    mask: u32,
    targets: &mut BTreeMap<PathBuf, u32>,
) -> Result<(), SandboxHelperError> {
    use std::os::windows::fs::MetadataExt as _;

    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| SandboxHelperError::Setup(format!("grant target: {error}")))?;
        if metadata.file_type().is_symlink()
            || metadata.file_attributes() & WINDOWS_REPARSE_POINT_ATTRIBUTE != 0
        {
            return Err(SandboxHelperError::Setup(format!(
                "Windows filesystem grant contains a reparse point: {}",
                path.display()
            )));
        }
        let canonical = fs::canonicalize(&path)
            .map_err(|error| SandboxHelperError::Setup(format!("grant target: {error}")))?;
        if !canonical.starts_with(root) {
            return Err(SandboxHelperError::Setup(format!(
                "Windows filesystem grant escaped its canonical root: {}",
                canonical.display()
            )));
        }
        if !targets.contains_key(&canonical) && targets.len() >= MAX_WINDOWS_ACL_TARGETS {
            return Err(SandboxHelperError::Setup(format!(
                "Windows filesystem grants exceed {MAX_WINDOWS_ACL_TARGETS} ACL targets"
            )));
        }
        targets
            .entry(canonical)
            .and_modify(|combined| *combined |= mask)
            .or_insert(mask);
        if metadata.is_dir() {
            let entries = fs::read_dir(&path).map_err(|error| {
                SandboxHelperError::Setup(format!("enumerate grant target: {error}"))
            })?;
            for entry in entries {
                pending.push(
                    entry
                        .map_err(|error| {
                            SandboxHelperError::Setup(format!("enumerate grant target: {error}"))
                        })?
                        .path(),
                );
            }
        } else if !metadata.is_file() {
            return Err(SandboxHelperError::Setup(format!(
                "Windows filesystem grant is not a regular file or directory: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
impl Drop for WindowsProfileGuard {
    fn drop(&mut self) {
        if let Some(profile) = self.0.take() {
            let _ = profile.delete();
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for WindowsTemporaryGuard {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for WindowsProtectedAclGuard {
    fn drop(&mut self) {
        while let Some(path) = self.paths.pop() {
            let _ = Command::new(&self.icacls)
                .env_clear()
                .arg(path)
                .arg("/remove:d")
                .arg(format!("*{}", self.sid))
                .arg("/Q")
                .output();
        }
    }
}

#[cfg(target_os = "windows")]
pub(super) fn supervise_windows_job(
    job: &SandboxJob,
    backend: String,
) -> Result<SandboxJobResult, SandboxHelperError> {
    let networked = !job.obligations.network_destinations.is_empty();
    if networked != job.proxy_port.is_some() || networked != job.proxy_credential.is_some() {
        return Err(SandboxHelperError::Setup(
            "windows_job network destinations require a paired authenticated proxy".into(),
        ));
    }
    let profile_name = format!("colossus.sandbox.{}", job.job_id.replace('-', ""));
    let profile = AppContainerProfile::ensure(
        &profile_name,
        "Colossus sandbox",
        Some("Ephemeral Colossus process isolation profile"),
    )
    .map_err(|error| SandboxHelperError::Setup(format!("AppContainer profile: {error}")))?;
    let mut profile = WindowsProfileGuard(Some(profile));
    let package = &profile.0.as_ref().expect("profile is present").sid;
    let system_root = std::env::var("SystemRoot")
        .or_else(|_| std::env::var("WINDIR"))
        .map_err(|_| SandboxHelperError::Setup("Windows system root is unavailable".into()))?;
    let canonical_system_root = fs::canonicalize(&system_root)
        .map_err(|error| SandboxHelperError::Setup(format!("Windows system root: {error}")))?;
    let local_app_data = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        SandboxHelperError::Setup("Windows local app-data root is unavailable".into())
    })?;
    let local_app_data = fs::canonicalize(local_app_data).map_err(|error| {
        SandboxHelperError::Setup(format!("Windows local app-data root: {error}"))
    })?;

    let mut grants = BTreeMap::<PathBuf, u32>::new();
    for grant in &job.obligations.filesystem {
        let root = fs::canonicalize(&grant.root)
            .map_err(|error| SandboxHelperError::Setup(format!("grant root: {error}")))?;
        if grant.mode == "execute" && root.starts_with(&canonical_system_root) {
            // Windows grants AppContainers read/execute access to operating-system binaries.
            // Avoid mutating protected System32 ACLs while retaining the exact executable
            // identity check performed before the helper is invoked.
            continue;
        }
        let mask = match grant.mode.as_str() {
            "execute" => 0x0012_0089 | 0x0012_00A0,
            "write" => 0x0012_0089 | 0x0012_0116 | 0x0012_00A0,
            "read" | "metadata" => 0x0012_0089 | 0x0012_00A0,
            other => {
                return Err(SandboxHelperError::Setup(format!(
                    "unsupported Windows filesystem grant mode {other}"
                )));
            }
        };
        grants
            .entry(root)
            .and_modify(|combined| *combined |= mask)
            .or_insert(mask);
    }
    let mut acl_targets = BTreeMap::<PathBuf, u32>::new();
    for (root, mask) in grants {
        collect_windows_acl_targets(&root, mask, &mut acl_targets)?;
    }
    for (path, mask) in acl_targets {
        let metadata = fs::metadata(&path)
            .map_err(|error| SandboxHelperError::Setup(format!("grant target: {error}")))?;
        let resource = if metadata.is_dir() {
            ResourcePath::Directory(path.clone())
        } else if metadata.is_file() {
            ResourcePath::File(path.clone())
        } else {
            return Err(SandboxHelperError::Setup(format!(
                "Windows filesystem grant is not a regular file or directory: {}",
                path.display()
            )));
        };
        acl::grant_to_package(resource, package, AccessMask(mask)).map_err(|error| {
            SandboxHelperError::Setup(format!("AppContainer ACL {}: {error}", path.display()))
        })?;
    }
    let mut protected_acl = WindowsProtectedAclGuard::create(
        &canonical_system_root,
        package,
        &job.obligations.protected_filesystem,
    )?;

    let mut temporary = WindowsTemporaryGuard::create(job, package)?;
    let mut environment = job
        .process
        .environment
        .iter()
        .map(|(name, value)| (name.clone(), windows_process_value(value)))
        .collect::<BTreeMap<_, _>>();
    for reserved in [
        "systemroot",
        "windir",
        "comspec",
        "path",
        "pathext",
        "localappdata",
        "temp",
        "tmp",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ] {
        if environment
            .keys()
            .any(|name| name.eq_ignore_ascii_case(reserved))
        {
            return Err(SandboxHelperError::Setup(format!(
                "Windows sandbox environment name {reserved} is reserved"
            )));
        }
    }
    environment.insert("SystemRoot".into(), system_root.clone());
    environment.insert("WINDIR".into(), system_root.clone());
    environment.insert(
        "ComSpec".into(),
        Path::new(&system_root)
            .join("System32")
            .join("cmd.exe")
            .display()
            .to_string(),
    );
    environment.insert(
        "PATH".into(),
        Path::new(&system_root)
            .join("System32")
            .display()
            .to_string(),
    );
    environment.insert("PATHEXT".into(), ".COM;.EXE;.BAT;.CMD".into());
    environment.insert("LOCALAPPDATA".into(), windows_process_path(&local_app_data));
    environment.insert("TEMP".into(), windows_process_path(temporary.path()));
    environment.insert("TMP".into(), windows_process_path(temporary.path()));
    if let Some(port) = job.proxy_port {
        let proxy = authenticated_proxy_url(port, job.proxy_credential.as_deref());
        environment.insert("HTTP_PROXY".into(), proxy.clone());
        environment.insert("HTTPS_PROXY".into(), proxy.clone());
        environment.insert("ALL_PROXY".into(), proxy);
        environment.insert("NO_PROXY".into(), String::new());
    }

    let loopback = if networked {
        Some(LoopbackExemptionGuard::new(package).map_err(|error| {
            SandboxHelperError::Setup(format!("AppContainer loopback exemption: {error}"))
        })?)
    } else {
        None
    };

    let executable = fs::canonicalize(&job.executable)
        .map_err(|error| SandboxHelperError::Setup(format!("executable: {error}")))?;
    let cwd = fs::canonicalize(&job.process.cwd)
        .map_err(|error| SandboxHelperError::Setup(format!("cwd: {error}")))?;
    let request = WindowsSpawnRequest {
        executable,
        arguments: job.process.args.clone(),
        cwd,
        environment,
        appcontainer_sid: package.as_string().into(),
        max_processes: job.obligations.max_processes,
        max_memory_bytes: job.obligations.max_memory_bytes,
        proxy_port: job.proxy_port,
        network_filter_id: job
            .proxy_port
            .map(|_| {
                Uuid::parse_str(&job.job_id)
                    .map(|id| id.as_u128())
                    .map_err(|error| SandboxHelperError::Setup(error.to_string()))
            })
            .transpose()?,
    };
    let mut child = colossus_windows_process::spawn(&request)
        .map_err(|error| SandboxHelperError::Execution(error.to_string()))?;
    if let Some(encoded) = &job.process.stdin_base64 {
        let input = BASE64
            .decode(encoded)
            .map_err(|error| SandboxHelperError::Execution(error.to_string()))?;
        child
            .stdin
            .as_mut()
            .ok_or_else(|| SandboxHelperError::Execution("child stdin is absent".into()))?
            .write_all(&input)?;
    }
    drop(child.stdin.take());
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| SandboxHelperError::Execution("child stdout is absent".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| SandboxHelperError::Execution("child stderr is absent".into()))?;
    let output_limit = usize::try_from(job.obligations.max_output_bytes).unwrap_or(usize::MAX);
    let capture_limit = output_limit.saturating_sub(1024).saturating_mul(3) / 4;
    let state = Arc::new(Mutex::new(CaptureState {
        remaining: capture_limit,
        ..CaptureState::default()
    }));
    let stdout_handle = capture(stdout, Arc::clone(&state), CaptureStream::Stdout);
    let stderr_handle = capture(stderr, Arc::clone(&state), CaptureStream::Stderr);
    let started = Instant::now();
    let timeout = Duration::from_millis(job.timeout_ms);
    let mut timed_out = false;
    let mut resource_limit_exceeded = None;
    let exit_code = loop {
        if let Some(code) = child
            .wait_timeout(Duration::from_millis(10))
            .map_err(|error| SandboxHelperError::Execution(error.to_string()))?
        {
            if resource_limit_exceeded.is_none() {
                resource_limit_exceeded = windows_resource_limit(&child)?;
            }
            break code;
        }
        if let Some(limit) = windows_resource_limit(&child)? {
            resource_limit_exceeded = Some(limit);
            child
                .terminate(0xC000_0044)
                .map_err(|error| SandboxHelperError::Execution(error.to_string()))?;
            break wait_for_windows_termination(&child)?;
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            child
                .terminate(0xC000_013A)
                .map_err(|error| SandboxHelperError::Execution(error.to_string()))?;
            break wait_for_windows_termination(&child)?;
        }
    };
    stdout_handle
        .join()
        .map_err(|_| SandboxHelperError::Execution("stdout capture panicked".into()))??;
    stderr_handle
        .join()
        .map_err(|_| SandboxHelperError::Execution("stderr capture panicked".into()))??;
    let state = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let stdout = redact_proxy_credential(&state.stdout, job.proxy_credential.as_deref());
    let stderr = redact_proxy_credential(&state.stderr, job.proxy_credential.as_deref());
    let result = SandboxJobResult {
        backend,
        exit_code: Some(i32::from_ne_bytes(exit_code.to_ne_bytes())),
        success: exit_code == 0 && !timed_out && resource_limit_exceeded.is_none(),
        timed_out,
        resource_limit_exceeded,
        output_truncated: state.truncated,
        stdout_base64: BASE64.encode(stdout),
        stderr_base64: BASE64.encode(stderr),
        observed_origins: Vec::new(),
    };
    drop(state);
    drop(child);
    drop(loopback);
    temporary.remove()?;
    protected_acl.remove()?;
    profile.remove()?;
    Ok(result)
}

#[cfg(target_os = "windows")]
pub(super) fn windows_process_path(path: &Path) -> String {
    windows_process_value(&path.display().to_string())
}

#[cfg(target_os = "windows")]
pub(super) fn windows_process_value(value: &str) -> String {
    value
        .strip_prefix(r"\\?\")
        .filter(|path| {
            path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                && path.as_bytes().get(1) == Some(&b':')
        })
        .unwrap_or(value)
        .to_owned()
}

#[cfg(target_os = "windows")]
pub(super) fn windows_resource_limit(
    child: &colossus_windows_process::SandboxedChild,
) -> Result<Option<String>, SandboxHelperError> {
    child
        .resource_limit_violation()
        .map(|limit| {
            limit.map(|limit| match limit {
                ResourceLimitViolation::Memory => "memory".into(),
                ResourceLimitViolation::ProcessCount => "process-count".into(),
            })
        })
        .map_err(|error| SandboxHelperError::Execution(error.to_string()))
}

#[cfg(target_os = "windows")]
pub(super) fn wait_for_windows_termination(
    child: &colossus_windows_process::SandboxedChild,
) -> Result<u32, SandboxHelperError> {
    child
        .wait_timeout(Duration::from_secs(5))
        .map_err(|error| SandboxHelperError::Execution(error.to_string()))?
        .ok_or_else(|| {
            SandboxHelperError::Execution(
                "Windows Job Object termination could not be confirmed".into(),
            )
        })
}

#[cfg(target_os = "macos")]
pub(super) fn native_runtime_paths() -> Vec<&'static Path> {
    [
        "/System/Library",
        "/usr/lib",
        "/Library/Apple/usr/lib",
        "/dev/null",
        "/dev/urandom",
    ]
    .into_iter()
    .map(Path::new)
    .collect()
}

#[cfg(target_os = "linux")]
pub(super) fn native_runtime_paths() -> Vec<&'static Path> {
    [
        "/lib",
        "/lib64",
        "/usr/lib",
        "/usr/lib64",
        "/dev/null",
        "/dev/urandom",
    ]
    .into_iter()
    .map(Path::new)
    .collect()
}
