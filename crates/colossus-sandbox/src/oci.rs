use super::*;

pub(super) struct OciNetworkResources {
    pub(super) runtime: PathBuf,
    pub(super) names: OciResourceNames,
    pub(super) proxy_address: SocketAddr,
    pub(super) armed: bool,
}

impl OciNetworkResources {
    pub(super) fn start(job: &SandboxJob) -> Result<Self, SandboxHelperError> {
        let runtime = job
            .oci_runtime
            .as_ref()
            .ok_or_else(|| SandboxHelperError::Setup("OCI runtime is not configured".into()))?;
        oci_runtime_kind(runtime).ok_or_else(|| {
            SandboxHelperError::Setup("OCI runtime must be the Docker or Podman executable".into())
        })?;
        let proxy_image = job
            .oci_proxy_image
            .as_ref()
            .ok_or_else(|| SandboxHelperError::Setup("OCI proxy image is not configured".into()))?;
        if !valid_oci_image_reference(proxy_image) {
            return Err(SandboxHelperError::Setup(
                "OCI proxy image must use a complete immutable SHA-256 reference".into(),
            ));
        }
        let names = oci_resource_names(&job.job_id);
        let mut resources = Self {
            runtime: runtime.clone(),
            names,
            proxy_address: SocketAddr::from(([0, 0, 0, 0], OCI_PROXY_PORT)),
            armed: true,
        };
        run_oci_control(
            runtime,
            &[
                "network".into(),
                "create".into(),
                "--internal".into(),
                "--label".into(),
                format!("dev.colossus.job={}", job.job_id),
                resources.names.internal_network.clone(),
            ],
            &[],
            "create the internal OCI network",
        )?;
        run_oci_control(
            runtime,
            &[
                "network".into(),
                "create".into(),
                "--label".into(),
                format!("dev.colossus.job={}", job.job_id),
                resources.names.egress_network.clone(),
            ],
            &[],
            "create the OCI proxy egress network",
        )?;
        let bootstrap = OciProxyBootstrap {
            schema_version: 1,
            request_hash: job.request_hash.clone(),
            decision_id: job.decision_id.clone(),
            permit_nonce: job.permit_nonce.clone(),
            expires_at_unix_ms: job.permit_expires_at_unix_ms,
            allowed_origins: job.obligations.network_destinations.clone(),
            resolved_origins: resolve_oci_origins(&job.obligations.network_destinations)?,
            max_connections: usize::try_from(job.obligations.max_processes)
                .unwrap_or(256)
                .clamp(1, 256),
            connection_timeout_ms: job.timeout_ms,
        };
        let encoded = BASE64.encode(serde_json::to_vec(&bootstrap)?);
        let proxy_environment = [(OCI_PROXY_CONFIG_VARIABLE, encoded.as_str())];
        let proxy_arguments = oci_proxy_run_arguments(job, &resources.names, proxy_image)?;
        run_oci_control(
            runtime,
            &proxy_arguments,
            &proxy_environment,
            "start the OCI allowlist proxy",
        )?;
        run_oci_control(
            runtime,
            &[
                "network".into(),
                "connect".into(),
                resources.names.egress_network.clone(),
                resources.names.proxy.clone(),
            ],
            &[],
            "connect the OCI proxy to its egress network",
        )?;
        let inspected = run_oci_control(
            runtime,
            &[
                "container".into(),
                "inspect".into(),
                resources.names.proxy.clone(),
            ],
            &[],
            "inspect the OCI proxy address",
        )?;
        resources.proxy_address =
            oci_network_address(&inspected, &resources.names.internal_network)?;
        let mut ready = false;
        for _ in 0..20 {
            let logs = run_oci_control(
                runtime,
                &["logs".into(), resources.names.proxy.clone()],
                &[],
                "read OCI proxy readiness",
            )?;
            if logs
                .windows(b"colossus-oci-proxy-ready".len())
                .any(|window| window == b"colossus-oci-proxy-ready")
            {
                ready = true;
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        if !ready {
            return Err(SandboxHelperError::Setup(
                "OCI allowlist proxy did not become ready".into(),
            ));
        }
        Ok(resources)
    }

    pub(super) fn proxy_address(&self) -> SocketAddr {
        self.proxy_address
    }

    pub(super) fn observed_origins(&self) -> Result<Vec<String>, SandboxHelperError> {
        let logs = run_oci_control(
            &self.runtime,
            &["logs".into(), self.names.proxy.clone()],
            &[],
            "read OCI proxy observed origins",
        )?;
        let mut origins = BTreeSet::new();
        for line in String::from_utf8_lossy(&logs).lines() {
            if let Some(origin) = line.strip_prefix(OBSERVED_ORIGIN_PREFIX)
                && origins.len() < MAX_OBSERVED_ORIGINS
            {
                origins.insert(origin.to_owned());
            }
        }
        Ok(origins.into_iter().collect())
    }

    pub(super) fn cleanup(&mut self) {
        if self.armed {
            cleanup_oci_resources(&self.runtime, &self.names);
            self.armed = false;
        }
    }
}

pub(super) fn oci_proxy_run_arguments(
    job: &SandboxJob,
    names: &OciResourceNames,
    proxy_image: &str,
) -> Result<Vec<String>, SandboxHelperError> {
    let runtime = job
        .oci_runtime
        .as_ref()
        .ok_or_else(|| SandboxHelperError::Setup("OCI runtime is not configured".into()))?;
    oci_runtime_kind(runtime).ok_or_else(|| {
        SandboxHelperError::Setup("OCI runtime must be the Docker or Podman executable".into())
    })?;
    let mut arguments = vec![
        "run".into(),
        "--detach".into(),
        "--rm".into(),
        "--pull=never".into(),
    ];
    arguments.extend([
        "--network".into(),
        names.internal_network.clone(),
        "--read-only".into(),
        "--cap-drop=ALL".into(),
        "--security-opt=no-new-privileges".into(),
        "--pids-limit=16".into(),
        "--memory=67108864".into(),
        "--name".into(),
        names.proxy.clone(),
        "--env".into(),
        OCI_PROXY_CONFIG_VARIABLE.into(),
        proxy_image.into(),
    ]);
    Ok(arguments)
}

impl Drop for OciNetworkResources {
    fn drop(&mut self) {
        self.cleanup();
    }
}

pub(super) fn resolve_oci_origins(
    origins: &[String],
) -> Result<BTreeMap<String, Vec<SocketAddr>>, SandboxHelperError> {
    let origins = origins.to_vec();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = sender.send(resolve_oci_origins_blocking(&origins));
    });
    receiver
        .recv_timeout(Duration::from_millis(OCI_DNS_RESOLUTION_TIMEOUT_MS))
        .map_err(|_| SandboxHelperError::Setup("OCI proxy DNS resolution timed out".into()))?
}

pub(super) fn resolve_oci_origins_blocking(
    origins: &[String],
) -> Result<BTreeMap<String, Vec<SocketAddr>>, SandboxHelperError> {
    let mut resolved = BTreeMap::new();
    for origin in origins {
        if origin == "*" {
            continue;
        }
        let url =
            Url::parse(origin).map_err(|error| SandboxHelperError::Setup(error.to_string()))?;
        let host = url
            .host_str()
            .ok_or_else(|| SandboxHelperError::Setup("OCI proxy origin has no host".into()))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| SandboxHelperError::Setup("OCI proxy origin has no port".into()))?;
        let allow_non_public = host.eq_ignore_ascii_case("localhost")
            || host.parse::<IpAddr>().is_ok_and(non_public_network_address);
        let mut addresses = (host, port)
            .to_socket_addrs()
            .map_err(|error| SandboxHelperError::Setup(error.to_string()))?
            .filter(|address| allow_non_public || !non_public_ip(address.ip()))
            .collect::<Vec<_>>();
        addresses.sort_by_key(|address| usize::from(address.is_ipv6()));
        addresses.dedup();
        addresses.truncate(16);
        if addresses.is_empty() {
            return Err(SandboxHelperError::Setup(format!(
                "OCI proxy origin resolved to no permitted address: {origin}"
            )));
        }
        resolved.insert(origin.clone(), addresses);
    }
    Ok(resolved)
}

pub(super) fn oci_network_address(
    inspection: &[u8],
    network_name: &str,
) -> Result<SocketAddr, SandboxHelperError> {
    let documents: Value = serde_json::from_slice(inspection)?;
    let address = documents
        .as_array()
        .and_then(|documents| documents.first())
        .and_then(|document| document.get("NetworkSettings"))
        .and_then(|settings| settings.get("Networks"))
        .and_then(|networks| networks.get(network_name))
        .and_then(|network| network.get("IPAddress"))
        .and_then(Value::as_str)
        .ok_or_else(|| SandboxHelperError::Setup("OCI proxy has no internal address".into()))?
        .parse::<IpAddr>()
        .map_err(|error| SandboxHelperError::Setup(error.to_string()))?;
    if address.is_unspecified() {
        return Err(SandboxHelperError::Setup(
            "OCI proxy internal address is unspecified".into(),
        ));
    }
    Ok(SocketAddr::new(address, OCI_PROXY_PORT))
}

pub(super) fn oci_command(
    job: &SandboxJob,
    proxy_address: Option<SocketAddr>,
) -> Result<Command, SandboxHelperError> {
    if job.proxy_port.is_some() {
        return Err(SandboxHelperError::Setup(
            "OCI jobs cannot use a host loopback proxy".into(),
        ));
    }
    if job.obligations.network_destinations.is_empty() != proxy_address.is_none() {
        return Err(SandboxHelperError::Setup(
            "OCI proxy resources do not match the network obligations".into(),
        ));
    }
    let runtime = job
        .oci_runtime
        .as_ref()
        .ok_or_else(|| SandboxHelperError::Setup("OCI runtime is not configured".into()))?;
    let runtime_kind = oci_runtime_kind(runtime).ok_or_else(|| {
        SandboxHelperError::Setup("OCI runtime must be the Docker or Podman executable".into())
    })?;
    let image = job
        .oci_image
        .as_ref()
        .ok_or_else(|| SandboxHelperError::Setup("OCI image is not configured".into()))?;
    if !valid_oci_image_reference(image) {
        return Err(SandboxHelperError::Setup(
            "OCI image must use a complete immutable SHA-256 reference".into(),
        ));
    }
    let mut command = Command::new(runtime);
    command
        .env_clear()
        .env("PATH", oci_runtime_search_path(runtime)?)
        .args(["run", "--rm", "--pull=never"]);
    if job.process.stdin_base64.is_some() {
        command.arg("--interactive");
    }
    let (uid, gid) = oci_mount_identity(&job.process.cwd)?;
    if runtime_kind == OciRuntimeKind::Podman {
        command.arg("--userns=keep-id");
    }
    command.arg("--user").arg(format!("{uid}:{gid}"));
    if proxy_address.is_some() {
        command
            .arg("--network")
            .arg(oci_resource_names(&job.job_id).internal_network)
            .arg("--dns=127.0.0.1");
    } else {
        command.arg("--network=none");
    }
    command.args([
        "--read-only",
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges",
    ]);
    command.arg("--name").arg(oci_container_name(&job.job_id));
    command.arg(format!("--pids-limit={}", job.obligations.max_processes));
    command.arg(format!("--memory={}", job.obligations.max_memory_bytes));
    let mut mounts = BTreeMap::<PathBuf, bool>::new();
    for grant in &job.obligations.filesystem {
        if grant.mode == "execute" {
            continue;
        }
        let root = fs::canonicalize(&grant.root)
            .map_err(|error| SandboxHelperError::Setup(error.to_string()))?;
        if root.to_string_lossy().contains([',', '\0']) {
            return Err(SandboxHelperError::Setup(
                "OCI bind mount paths may not contain commas or NUL".into(),
            ));
        }
        mounts
            .entry(root)
            .and_modify(|writable| *writable |= grant.mode == "write")
            .or_insert(grant.mode == "write");
    }
    for (root, writable) in mounts {
        let readonly = if writable { "" } else { ",readonly" };
        command.arg("--mount").arg(format!(
            "type=bind,source={},target={}{}",
            root.display(),
            root.display(),
            readonly
        ));
    }
    for protected in &job.obligations.protected_filesystem {
        let protected = fs::canonicalize(protected)
            .map_err(|error| SandboxHelperError::Setup(format!("protected path: {error}")))?;
        if !protected.is_dir() || protected.to_string_lossy().contains([',', '\0']) {
            return Err(SandboxHelperError::Setup(
                "OCI protected paths must be canonical directories without commas or NUL".into(),
            ));
        }
        command.arg("--mount").arg(format!(
            "type=tmpfs,target={},readonly,tmpfs-size=4096,tmpfs-mode=0000",
            protected.display()
        ));
    }
    let mut environment = job.process.environment.clone();
    if let Some(proxy_address) = proxy_address {
        let proxy = format!("http://{proxy_address}");
        environment.insert("HTTP_PROXY".into(), proxy.clone());
        environment.insert("HTTPS_PROXY".into(), proxy.clone());
        environment.insert("ALL_PROXY".into(), proxy.clone());
        environment.insert("NO_PROXY".into(), String::new());
        environment.insert("http_proxy".into(), proxy.clone());
        environment.insert("https_proxy".into(), proxy.clone());
        environment.insert("all_proxy".into(), proxy);
        environment.insert("no_proxy".into(), String::new());
    }
    for (name, value) in &environment {
        command.env(name, value).arg("--env").arg(name);
    }
    let mut bootstrap = "exec /usr/bin/env -i --".to_owned();
    for name in environment.keys() {
        if !valid_environment_name(name) {
            return Err(SandboxHelperError::Setup(format!(
                "invalid OCI environment name {name}"
            )));
        }
        bootstrap.push(' ');
        bootstrap.push_str(name);
        bootstrap.push_str("=\"${");
        bootstrap.push_str(name);
        bootstrap.push_str("}\"");
    }
    bootstrap.push_str(" \"$@\"");
    command
        .arg("--workdir")
        .arg(&job.process.cwd)
        .arg("--entrypoint")
        .arg("/bin/sh")
        .arg(image)
        .arg("-c")
        .arg(bootstrap)
        .arg("colossus-bootstrap")
        .arg(&job.executable)
        .args(&job.process.args);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command)
}

pub(super) fn valid_oci_image_reference(image: &str) -> bool {
    if let Some(digest) = image.strip_prefix("sha256:") {
        return digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    let Some((repository, digest)) = image.rsplit_once("@sha256:") else {
        return false;
    };
    !repository.is_empty()
        && digest.len() == 64
        && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn oci_container_name(job_id: &str) -> String {
    let sanitized = job_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>();
    format!("colossus-{sanitized}")
}

pub(super) fn oci_resource_names(job_id: &str) -> OciResourceNames {
    let workload = oci_container_name(job_id);
    let suffix = workload.trim_start_matches("colossus-").to_owned();
    OciResourceNames {
        workload,
        proxy: format!("colossus-proxy-{suffix}"),
        internal_network: format!("colossus-int-{suffix}"),
        egress_network: format!("colossus-egress-{suffix}"),
    }
}

pub(super) fn bounded_control_command(
    mut command: Command,
) -> Option<(std::process::ExitStatus, Vec<u8>, Vec<u8>)> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().ok()?;
    let deadline = Instant::now() + Duration::from_millis(OCI_CONTROL_COMMAND_TIMEOUT_MS);
    loop {
        if let Some(status) = child.try_wait().ok()? {
            let mut stdout = Vec::new();
            child.stdout.take()?.read_to_end(&mut stdout).ok()?;
            let mut stderr = Vec::new();
            child
                .stderr
                .take()?
                .take(u64::try_from(MAX_OCI_CONTROL_DIAGNOSTIC_BYTES).unwrap_or(u64::MAX))
                .read_to_end(&mut stderr)
                .ok()?;
            return Some((status, stdout, stderr));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn run_oci_control(
    runtime: &Path,
    arguments: &[String],
    environment: &[(&str, &str)],
    operation: &str,
) -> Result<Vec<u8>, SandboxHelperError> {
    let mut command = Command::new(runtime);
    command
        .env_clear()
        .env("PATH", oci_runtime_search_path(runtime)?)
        .envs(environment.iter().copied())
        .args(arguments);
    match bounded_control_command(command) {
        Some((status, stdout, _)) if status.success() => Ok(stdout),
        Some((status, _, stderr)) => {
            let diagnostic = String::from_utf8_lossy(&stderr)
                .chars()
                .filter(|character| !character.is_control() || character.is_ascii_whitespace())
                .collect::<String>();
            Err(SandboxHelperError::Setup(format!(
                "failed to {operation}: runtime exited with {status}: {}",
                diagnostic.trim()
            )))
        }
        None => Err(SandboxHelperError::Setup(format!(
            "failed to {operation}: runtime command timed out"
        ))),
    }
}

pub(super) fn oci_runtime_search_path(runtime: &Path) -> Result<&Path, SandboxHelperError> {
    runtime
        .parent()
        .filter(|parent| parent.is_absolute())
        .ok_or_else(|| {
            SandboxHelperError::Setup(
                "OCI runtime must have an absolute parent directory for helper resolution".into(),
            )
        })
}

pub(super) fn cleanup_oci_resources(runtime: &Path, names: &OciResourceNames) {
    for name in [&names.workload, &names.proxy] {
        let Some(arguments) = oci_remove_arguments(runtime, name) else {
            continue;
        };
        let mut remove = Command::new(runtime);
        remove.env_clear().args(arguments);
        let _ = bounded_control_command(remove);
    }
    for name in [&names.internal_network, &names.egress_network] {
        let mut remove = Command::new(runtime);
        remove.env_clear().args(["network", "rm", "--force", name]);
        let _ = bounded_control_command(remove);
    }
}

pub(super) fn oci_resources_absent(runtime: &Path, names: &OciResourceNames) -> bool {
    let containers_absent = [&names.workload, &names.proxy].iter().all(|name| {
        let mut list = Command::new(runtime);
        list.env_clear().args([
            "container",
            "ls",
            "--all",
            "--filter",
            &format!("name=^/{name}$"),
            "--format",
            "{{.ID}}",
        ]);
        bounded_control_command(list).is_some_and(|(status, stdout, _)| {
            status.success() && stdout.iter().all(u8::is_ascii_whitespace)
        })
    });
    let networks_absent = [&names.internal_network, &names.egress_network]
        .iter()
        .all(|name| {
            let mut list = Command::new(runtime);
            list.env_clear().args([
                "network",
                "ls",
                "--filter",
                &format!("name=^{name}$"),
                "--format",
                "{{.Name}}",
            ]);
            bounded_control_command(list).is_some_and(|(status, stdout, _)| {
                status.success() && stdout.iter().all(u8::is_ascii_whitespace)
            })
        });
    containers_absent && networks_absent
}

pub(super) fn ensure_oci_resources_absent(job: &SandboxJob) -> bool {
    let Some(runtime) = job.oci_runtime.as_ref() else {
        return false;
    };
    let names = oci_resource_names(&job.job_id);
    cleanup_oci_resources(runtime, &names);
    oci_resources_absent(runtime, &names)
}

pub(super) async fn ensure_oci_resources_absent_async(
    runtime: Option<&Path>,
    names: &OciResourceNames,
) -> bool {
    let Some(runtime) = runtime else {
        return false;
    };
    for name in [&names.workload, &names.proxy] {
        let Some(remove_arguments) = oci_remove_arguments(runtime, name) else {
            return false;
        };
        let mut remove = TokioCommand::new(runtime);
        remove
            .env_clear()
            .args(remove_arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let _ = tokio::time::timeout(
            Duration::from_millis(OCI_CONTROL_COMMAND_TIMEOUT_MS),
            remove.status(),
        )
        .await;
    }
    for name in [&names.internal_network, &names.egress_network] {
        let mut remove = TokioCommand::new(runtime);
        remove
            .env_clear()
            .args(["network", "rm", "--force", name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let _ = tokio::time::timeout(
            Duration::from_millis(OCI_CONTROL_COMMAND_TIMEOUT_MS),
            remove.status(),
        )
        .await;
    }
    let mut containers_absent = true;
    for name in [&names.workload, &names.proxy] {
        let mut list = TokioCommand::new(runtime);
        list.env_clear()
            .args([
                "container",
                "ls",
                "--all",
                "--filter",
                &format!("name=^/{name}$"),
                "--format",
                "{{.ID}}",
            ])
            .stdin(Stdio::null())
            .kill_on_drop(true);
        containers_absent &= tokio::time::timeout(
            Duration::from_millis(OCI_CONTROL_COMMAND_TIMEOUT_MS),
            list.output(),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .is_some_and(|output| {
            output.status.success() && output.stdout.iter().all(u8::is_ascii_whitespace)
        });
    }
    let mut networks_absent = true;
    for name in [&names.internal_network, &names.egress_network] {
        let mut list = TokioCommand::new(runtime);
        list.env_clear()
            .args([
                "network",
                "ls",
                "--filter",
                &format!("name=^{name}$"),
                "--format",
                "{{.Name}}",
            ])
            .stdin(Stdio::null())
            .kill_on_drop(true);
        networks_absent &= tokio::time::timeout(
            Duration::from_millis(OCI_CONTROL_COMMAND_TIMEOUT_MS),
            list.output(),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .is_some_and(|output| {
            output.status.success() && output.stdout.iter().all(u8::is_ascii_whitespace)
        });
    }
    containers_absent && networks_absent
}

pub(super) fn configure_command(command: &mut Command, job: &SandboxJob) {
    command
        .args(&job.process.args)
        .current_dir(&job.process.cwd)
        .env_clear()
        .envs(&job.process.environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(port) = job.proxy_port {
        let proxy = authenticated_proxy_url(port, job.proxy_credential.as_deref());
        configure_proxy_environment(command, &proxy);
    }
}

pub(super) fn configure_proxy_environment(command: &mut Command, proxy: &str) {
    command
        .env("HTTP_PROXY", proxy)
        .env("HTTPS_PROXY", proxy)
        .env("ALL_PROXY", proxy)
        .env("NO_PROXY", "");
    #[cfg(unix)]
    command
        // curl deliberately ignores uppercase HTTP_PROXY, so Unix tools need the
        // conventional lowercase spellings as well. These overwrite any values
        // supplied by the untrusted process specification.
        .env("http_proxy", proxy)
        .env("https_proxy", proxy)
        .env("all_proxy", proxy)
        .env("no_proxy", "");
}

pub(super) fn authenticated_proxy_url(port: u16, credential: Option<&str>) -> String {
    credential.map_or_else(
        || format!("http://127.0.0.1:{port}"),
        |credential| format!("http://colossus:{credential}@127.0.0.1:{port}"),
    )
}

pub(super) fn redact_proxy_credential(bytes: &[u8], credential: Option<&str>) -> Vec<u8> {
    let Some(credential) = credential else {
        return bytes.to_vec();
    };
    let basic = BASE64.encode(format!("colossus:{credential}"));
    let mut redacted = bytes.to_vec();
    for secret in [credential.as_bytes(), basic.as_bytes()] {
        while let Some(offset) = redacted
            .windows(secret.len())
            .position(|window| window == secret)
        {
            redacted.splice(offset..offset + secret.len(), b"[REDACTED]".iter().copied());
        }
    }
    redacted
}
