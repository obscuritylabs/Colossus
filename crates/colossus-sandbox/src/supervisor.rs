use super::*;

#[derive(Default)]
pub(super) struct CaptureState {
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
    pub(super) remaining: usize,
    pub(super) truncated: bool,
}

#[derive(Clone, Copy)]
pub(super) enum CaptureStream {
    Stdout,
    Stderr,
}

pub(super) fn capture<R: Read + Send + 'static>(
    mut reader: R,
    state: Arc<Mutex<CaptureState>>,
    stream: CaptureStream,
) -> thread::JoinHandle<Result<(), std::io::Error>> {
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                return Ok(());
            }
            let mut state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let retained = count.min(state.remaining);
            match stream {
                CaptureStream::Stdout => state.stdout.extend_from_slice(&buffer[..retained]),
                CaptureStream::Stderr => state.stderr.extend_from_slice(&buffer[..retained]),
            }
            state.remaining = state.remaining.saturating_sub(retained);
            state.truncated |= retained < count;
        }
    })
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ProcessTreeUsage {
    pub(super) processes: usize,
    pub(super) memory: u64,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn supervise_native_inner_process(
    command: &mut Command,
    job: &SandboxJob,
    backend: String,
    encoded_job: &[u8],
) -> Result<SandboxJobResult, SandboxHelperError> {
    let mut child = command
        .group_spawn()
        .map_err(|error| SandboxHelperError::Execution(error.to_string()))?;
    let mut stdin = child
        .inner()
        .stdin
        .take()
        .ok_or_else(|| SandboxHelperError::Execution("native inner stdin is absent".into()))?;
    stdin.write_all(encoded_job)?;
    drop(stdin);
    let stdout = child
        .inner()
        .stdout
        .take()
        .ok_or_else(|| SandboxHelperError::Execution("native inner stdout is absent".into()))?;
    let stderr = child
        .inner()
        .stderr
        .take()
        .ok_or_else(|| SandboxHelperError::Execution("native inner stderr is absent".into()))?;
    let output_limit = usize::try_from(job.obligations.max_output_bytes).unwrap_or(usize::MAX);
    let state = Arc::new(Mutex::new(CaptureState {
        remaining: output_limit.saturating_add(16 * 1024),
        ..CaptureState::default()
    }));
    let stdout_handle = capture(stdout, Arc::clone(&state), CaptureStream::Stdout);
    let stderr_handle = capture(stderr, Arc::clone(&state), CaptureStream::Stderr);
    let started = Instant::now();
    let timeout = Duration::from_millis(job.timeout_ms);
    let mut system = System::new();
    let root_pid = SystemPid::from_u32(child.id());
    let mut target_pid = None;
    let (status, timed_out, resource_limit_exceeded) = loop {
        if let Some(status) = child.try_wait()? {
            let _ = child.kill();
            break (status, false, None);
        }
        if started.elapsed() >= timeout {
            terminate_process_tree(&mut system, root_pid);
            let _ = child.kill();
            break (child.wait()?, true, None);
        }
        if target_pid.is_none() {
            let state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            target_pid = native_target_pid(&state.stderr);
        }
        let limit = target_pid.and_then(|target_pid| {
            let usage = process_tree_usage(&mut system, target_pid);
            if usage.processes
                > usize::try_from(job.obligations.max_processes).unwrap_or(usize::MAX)
            {
                Some("process-count")
            } else if usage.memory > job.obligations.max_memory_bytes {
                Some("memory")
            } else {
                None
            }
        });
        if let Some(limit) = limit {
            if let Some(target_pid) = target_pid {
                terminate_process_tree(&mut system, target_pid);
            }
            terminate_process_tree(&mut system, root_pid);
            let _ = child.kill();
            break (child.wait()?, false, Some(limit.into()));
        };
        thread::sleep(Duration::from_millis(10));
    };
    stdout_handle.join().map_err(|_| {
        SandboxHelperError::Execution("native inner stdout capture panicked".into())
    })??;
    stderr_handle.join().map_err(|_| {
        SandboxHelperError::Execution("native inner stderr capture panicked".into())
    })??;
    let state = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let stderr = native_helper_diagnostics(&state.stderr)?;
    let stderr = redact_proxy_credential(&stderr, job.proxy_credential.as_deref());
    if timed_out || resource_limit_exceeded.is_some() {
        return Ok(SandboxJobResult {
            backend,
            exit_code: status.code(),
            success: false,
            timed_out,
            resource_limit_exceeded,
            output_truncated: state.truncated,
            stdout_base64: String::new(),
            stderr_base64: String::new(),
            observed_origins: Vec::new(),
        });
    }
    if state.truncated {
        return Err(SandboxHelperError::Execution(
            "native inner result exceeds IPC bound".into(),
        ));
    }
    if !status.success() {
        return Err(SandboxHelperError::Execution(format!(
            "native inner helper failed: {}",
            String::from_utf8_lossy(&stderr)
        )));
    }
    if stderr.iter().any(|byte| !byte.is_ascii_whitespace()) {
        return Err(SandboxHelperError::Execution(
            "native inner helper emitted unexpected diagnostics".into(),
        ));
    }
    serde_json::from_slice(&state.stdout).map_err(SandboxHelperError::from)
}

pub(super) fn supervise(
    command: &mut Command,
    job: &SandboxJob,
    backend: String,
) -> Result<SandboxJobResult, SandboxHelperError> {
    let mut child = command
        .group_spawn()
        .map_err(|error| SandboxHelperError::Execution(error.to_string()))?;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if std::env::var_os(NATIVE_INNER_VARIABLE).is_some() {
        announce_native_target(child.id())?;
    }
    let stdout = child
        .inner()
        .stdout
        .take()
        .ok_or_else(|| SandboxHelperError::Execution("child stdout is absent".into()))?;
    let stderr = child
        .inner()
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
    let mut stdin = child.inner().stdin.take();
    if let Some(encoded) = &job.process.stdin_base64 {
        let input = BASE64
            .decode(encoded)
            .map_err(|error| SandboxHelperError::Execution(error.to_string()))?;
        let stdin = stdin
            .as_mut()
            .ok_or_else(|| SandboxHelperError::Execution("child stdin is absent".into()))?;
        stdin.write_all(&input)?;
        stdin.flush()?;
    }
    let mut completion = job
        .process
        .stdin_completion
        .as_ref()
        .map(StdinCompletionMonitor::new);
    if completion.is_none() {
        drop(stdin.take());
    }
    let started = Instant::now();
    let timeout = Duration::from_millis(job.timeout_ms);
    let mut system = System::new();
    let root_pid = SystemPid::from_u32(child.id());
    let (status, timed_out, resource_limit_exceeded) = loop {
        if let Some(status) = child.try_wait()? {
            // The group/job can outlive its leader when an executable backgrounds work.
            // Always terminate remaining descendants before returning a terminal result.
            let _ = child.kill();
            break (status, false, None);
        }
        if let Some(completion) = completion.as_mut()
            && stdin.is_some()
        {
            let close = {
                let state = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                completion.should_close(&state.stdout, state.truncated)
            };
            if close {
                drop(stdin.take());
            }
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            break (child.wait()?, true, None);
        }
        // OCI policy limits belong to the workload container. The trusted Docker/Podman
        // control process can legitimately exceed those limits while it creates the
        // already bounded container, so host accounting applies only to direct targets.
        let limit = if host_process_limits_apply(&backend) {
            let usage = process_tree_usage(&mut system, root_pid);
            if usage.processes
                > usize::try_from(job.obligations.max_processes).unwrap_or(usize::MAX)
            {
                Some("process-count")
            } else if usage.memory > job.obligations.max_memory_bytes {
                Some("memory")
            } else {
                None
            }
        } else {
            None
        };
        if let Some(limit) = limit {
            let _ = child.kill();
            break (child.wait()?, false, Some(limit.into()));
        }
        thread::sleep(Duration::from_millis(10));
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
    Ok(SandboxJobResult {
        backend,
        exit_code: status.code(),
        success: status.success() && !timed_out,
        timed_out,
        resource_limit_exceeded,
        output_truncated: state.truncated,
        stdout_base64: BASE64.encode(stdout),
        stderr_base64: BASE64.encode(stderr),
        observed_origins: Vec::new(),
    })
}

pub(super) fn host_process_limits_apply(backend: &str) -> bool {
    backend != "oci"
}

pub(super) fn process_tree_usage(system: &mut System, root: SystemPid) -> ProcessTreeUsage {
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );
    let members = process_tree_members(system, root);
    let (processes, memory) = members
        .iter()
        .filter_map(|pid| system.process(*pid))
        .filter(|process| process.thread_kind().is_none())
        .fold((0_usize, 0_u64), |(count, total), process| {
            (
                count.saturating_add(1),
                total.saturating_add(process.memory()),
            )
        });
    ProcessTreeUsage { processes, memory }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn announce_native_target(pid: u32) -> Result<(), SandboxHelperError> {
    let mut stderr = std::io::stderr().lock();
    stderr.write_all(NATIVE_TARGET_PID_PREFIX)?;
    writeln!(stderr, "{pid}")?;
    stderr.flush()?;
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn native_target_pid(stderr: &[u8]) -> Option<SystemPid> {
    let line_end = stderr.iter().position(|byte| *byte == b'\n')?;
    let pid = stderr[..line_end]
        .strip_prefix(NATIVE_TARGET_PID_PREFIX)
        .and_then(|value| std::str::from_utf8(value).ok())?
        .parse::<u32>()
        .ok()?;
    Some(SystemPid::from_u32(pid))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn native_helper_diagnostics(stderr: &[u8]) -> Result<Vec<u8>, SandboxHelperError> {
    let Some(line_end) = stderr.iter().position(|byte| *byte == b'\n') else {
        return Ok(stderr.to_vec());
    };
    if !stderr[..line_end].starts_with(NATIVE_TARGET_PID_PREFIX) {
        return Ok(stderr.to_vec());
    }
    native_target_pid(stderr).ok_or_else(|| {
        SandboxHelperError::Execution("native inner helper emitted an invalid target PID".into())
    })?;
    Ok(stderr[line_end.saturating_add(1)..].to_vec())
}

pub(super) fn process_tree_members(
    system: &System,
    root: SystemPid,
) -> std::collections::HashSet<SystemPid> {
    let mut members = std::collections::HashSet::from([root]);
    loop {
        let before = members.len();
        for (pid, process) in system.processes() {
            if process
                .parent()
                .is_some_and(|parent| members.contains(&parent))
            {
                members.insert(*pid);
            }
        }
        if members.len() == before {
            break;
        }
    }
    members
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn terminate_process_tree(system: &mut System, root: SystemPid) {
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let mut members = process_tree_members(system, root);
    for pid in &members {
        if let Some(process) = system.process(*pid) {
            let _ = process.kill_with(sysinfo::Signal::Stop);
        }
    }
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    members.extend(process_tree_members(system, root));
    for pid in members {
        if let Some(process) = system.process(pid) {
            let _ = process.kill_with(sysinfo::Signal::Kill);
        }
    }
}
