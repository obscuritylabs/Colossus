//! Bounded helper frames and a cancellation pipe owned by the trusted parent.
use super::*;
use std::{
    io::BufRead as _,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

pub(super) const MAX_PROCESS_FRAME_BYTES: usize = 32 * 1024;
const OUTPUT_CHUNK_BYTES: usize = 4096;
// Started + Completed metadata, including the longest built-in backend/status.
// Output budgets include JSON/base64 framing, not just raw stdout/stderr.
pub(super) const PROCESS_RESULT_RESERVE: usize = 512;

pub(super) fn process_result_reserve(obligations: &PolicyObligations) -> usize {
    if !matches!(
        obligations.sandbox_backend.as_str(),
        "native" | "windows_job" | "oci"
    ) {
        return PROCESS_RESULT_RESERVE;
    }
    // The parent adds native/Windows proxy origins; OCI adds them in the helper.
    // Reserve their full bounded evidence before either path captures any logs.
    let origins = if obligations
        .network_destinations
        .iter()
        .any(|origin| origin == "*")
    {
        MAX_OBSERVED_ORIGINS * (MAX_OBSERVED_ORIGIN_JSON_BYTES + 1)
    } else {
        let mut sizes = obligations
            .network_destinations
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|origin| {
                serde_json::to_vec(origin)
                    .map_or(usize::MAX, |bytes| bytes.len())
                    .min(MAX_OBSERVED_ORIGIN_JSON_BYTES)
                    .saturating_add(1)
            })
            .collect::<Vec<_>>();
        sizes.sort_unstable_by(|a, b| b.cmp(a));
        sizes.into_iter().take(MAX_OBSERVED_ORIGINS).sum()
    };
    PROCESS_RESULT_RESERVE + origins
}

pub(super) fn process_capture_limit(output_limit: usize, metadata_reserve: usize) -> usize {
    // Two base64 fields can each add padding. Reserve before converting to raw bytes.
    output_limit
        .saturating_sub(metadata_reserve)
        .saturating_sub(8)
        / 4
        * 3
}

fn bounded_output_frame(
    mut frame: ProcessFrame,
    budget: usize,
) -> Result<Option<(Vec<u8>, bool)>, SandboxHelperError> {
    let complete = serde_json::to_vec(&frame)?;
    if complete.len() < budget {
        return Ok(Some((complete, false)));
    }
    let ProcessFrame::Output {
        stdout_base64,
        stderr_base64,
    } = &mut frame
    else {
        return Err(SandboxHelperError::Execution(
            "expected output frame".into(),
        ));
    };
    // Base64 groups are independently decodable. Keep a prefix even when the
    // first frame is larger than a small requested budget.
    let overhead = complete.len() - stdout_base64.len() - stderr_base64.len() + 1;
    let mut available = budget.saturating_sub(overhead) / 4 * 4;
    if available == 0 {
        return Ok(None);
    }
    let stdout_len = stdout_base64.len().min(available);
    stdout_base64.truncate(stdout_len);
    available -= stdout_len;
    stderr_base64.truncate(stderr_base64.len().min(available));
    Ok(Some((serde_json::to_vec(&frame)?, true)))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ProcessFrame {
    Started {
        deadline_ms: u64,
        timeout_ms: u64,
        max_output_bytes: u64,
    },
    Output {
        stdout_base64: String,
        stderr_base64: String,
    },
    Completed {
        result: SandboxJobResult,
    },
}

/// The supervisor never blocks on a slow consumer while it owns a live process.
/// Exhausted IPC/output capacity truncates logs; cancellation remains independent.
pub(super) struct HelperControl {
    cancelled: Arc<AtomicBool>,
    truncated: AtomicBool,
    output_budget: Mutex<usize>,
    sender: Option<mpsc::SyncSender<Vec<u8>>>,
    writer: Option<thread::JoinHandle<Result<(), std::io::Error>>>,
}

impl HelperControl {
    pub(super) fn new(
        mut input: impl Read + Send + 'static,
        obligations: &PolicyObligations,
    ) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&cancelled);
        thread::spawn(move || {
            // EOF, a stop byte, or an invalid control all close execution authority.
            let mut byte = [0_u8; 1];
            let _ = input.read(&mut byte);
            signal.store(true, Ordering::Release);
        });
        let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(16);
        let signal = Arc::clone(&cancelled);
        let writer = thread::spawn(move || {
            let mut output = std::io::stdout().lock();
            for bytes in receiver {
                if let Err(error) = output.write_all(&bytes).and_then(|()| output.flush()) {
                    signal.store(true, Ordering::Release);
                    return Err(error);
                }
            }
            Ok(())
        });
        Self {
            cancelled,
            truncated: AtomicBool::new(false),
            output_budget: Mutex::new(
                usize::try_from(obligations.max_output_bytes)
                    .unwrap_or(usize::MAX)
                    .saturating_sub(process_result_reserve(obligations)),
            ),
            sender: Some(sender),
            writer: Some(writer),
        }
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(super) fn is_truncated(&self) -> bool {
        self.truncated.load(Ordering::Acquire)
    }

    pub(super) fn emit(&self, frame: ProcessFrame) -> Result<(), SandboxHelperError> {
        let mut bytes = serde_json::to_vec(&frame)?;
        if bytes.len() > MAX_PROCESS_FRAME_BYTES {
            return Err(SandboxHelperError::Execution(
                "process frame exceeds IPC bound".into(),
            ));
        }
        bytes.push(b'\n');
        let sender = self
            .sender
            .as_ref()
            .ok_or_else(|| SandboxHelperError::Execution("process stream is closed".into()))?;
        if matches!(frame, ProcessFrame::Output { .. }) {
            let mut budget = self
                .output_budget
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.is_truncated() {
                return Ok(());
            }
            let Some((bounded, truncated)) = bounded_output_frame(frame, *budget)? else {
                self.truncated.store(true, Ordering::Release);
                return Ok(());
            };
            bytes = bounded;
            bytes.push(b'\n');
            self.truncated.store(truncated, Ordering::Release);
            *budget -= bytes.len();
            match sender.try_send(bytes) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(_)) => self.truncated.store(true, Ordering::Release),
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    return Err(SandboxHelperError::Execution(
                        "process stream disconnected".into(),
                    ));
                }
            }
        } else {
            // Started has an empty queue; Completed is sent only after cleanup.
            sender
                .send(bytes)
                .map_err(|_| SandboxHelperError::Execution("process stream disconnected".into()))?;
        }
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<(), SandboxHelperError> {
        self.sender.take();
        if let Some(writer) = self.writer.take() {
            writer.join().map_err(|_| {
                SandboxHelperError::Execution("process stream writer panicked".into())
            })??;
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct OutputCursor {
    stdout: usize,
    stderr: usize,
}

impl OutputCursor {
    pub(super) fn emit(
        &mut self,
        state: &CaptureState,
        control: &HelperControl,
        secret: Option<&str>,
        final_output: bool,
    ) -> Result<(), SandboxHelperError> {
        let stdout = next_output(&state.stdout, &mut self.stdout, secret, final_output);
        let stderr = next_output(&state.stderr, &mut self.stderr, secret, final_output);
        if !stdout.is_empty() || !stderr.is_empty() {
            control.emit(ProcessFrame::Output {
                stdout_base64: BASE64.encode(stdout),
                stderr_base64: BASE64.encode(stderr),
            })?;
        }
        Ok(())
    }

    pub(super) fn exhausted(&self, state: &CaptureState) -> bool {
        self.stdout == state.stdout.len() && self.stderr == state.stderr.len()
    }
}

/// Retain a credential-sized suffix and never split a complete credential across frames.
fn next_output(
    bytes: &[u8],
    cursor: &mut usize,
    secret: Option<&str>,
    final_output: bool,
) -> Vec<u8> {
    let retained_tail = if final_output {
        0
    } else {
        secret.map_or(0, |s| s.len().saturating_sub(1))
    };
    let available = bytes.len().saturating_sub(retained_tail).max(*cursor);
    let mut end = available.min(cursor.saturating_add(OUTPUT_CHUNK_BYTES));
    if let Some(secret) = secret.filter(|secret| !secret.is_empty()) {
        for start in *cursor..end {
            if bytes[start..].starts_with(secret.as_bytes()) && start + secret.len() > end {
                end = start;
                break;
            }
        }
    }
    let output = redact_proxy_credential(&bytes[*cursor..end], secret);
    *cursor = end;
    output
}

pub(super) fn read_helper_job()
-> Result<(Vec<u8>, std::io::BufReader<std::io::Stdin>), SandboxHelperError> {
    let mut input = std::io::BufReader::new(std::io::stdin());
    let mut bytes = Vec::new();
    (&mut input)
        .take(
            u64::try_from(MAX_JOB_BYTES)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )
        .read_until(b'\n', &mut bytes)?;
    if bytes.len() > MAX_JOB_BYTES {
        return Err(SandboxHelperError::InvalidJob(
            "helper input exceeds IPC bound".into(),
        ));
    }
    Ok((bytes, input))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn relay_native_frames(
    bytes: &[u8],
    cursor: &mut usize,
    terminal: &mut Option<SandboxJobResult>,
    control: &HelperControl,
) -> Result<(), SandboxHelperError> {
    while let Some(length) = bytes[*cursor..].iter().position(|byte| *byte == b'\n') {
        if length > MAX_PROCESS_FRAME_BYTES || terminal.is_some() {
            return Err(SandboxHelperError::Execution(
                "invalid native inner stream frame".into(),
            ));
        }
        let frame: ProcessFrame = serde_json::from_slice(&bytes[*cursor..*cursor + length])?;
        *cursor += length + 1;
        match frame {
            ProcessFrame::Completed { result } => *terminal = Some(result),
            frame => control.emit(frame)?,
        }
    }
    if bytes.len().saturating_sub(*cursor) > MAX_PROCESS_FRAME_BYTES {
        return Err(SandboxHelperError::Execution(
            "native inner frame exceeds IPC bound".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepted_small_budgets_retain_both_streams_and_bound_framing() {
        for limit in [1024, 4096] {
            let result = SandboxJobResult {
                backend: "windows-appcontainer".into(),
                exit_code: Some(i32::MIN),
                success: false,
                timed_out: false,
                stopped: false,
                resource_limit_exceeded: None,
                output_truncated: true,
                stdout_base64: String::new(),
                stderr_base64: String::new(),
                observed_origins: Vec::new(),
            };
            let metadata = serde_json::to_vec(&ProcessFrame::Started {
                deadline_ms: u64::MAX,
                timeout_ms: u64::MAX,
                max_output_bytes: u64::MAX,
            })
            .unwrap()
            .len()
                + 1
                + serde_json::to_vec(&ProcessFrame::Completed { result })
                    .unwrap()
                    .len()
                + 1;
            assert!(metadata <= PROCESS_RESULT_RESERVE);
            let output = ProcessFrame::Output {
                stdout_base64: BASE64.encode("out\n"),
                stderr_base64: BASE64.encode("err\n"),
            };
            let (bytes, truncated) = bounded_output_frame(output, limit - PROCESS_RESULT_RESERVE)
                .unwrap()
                .unwrap();
            assert!(!truncated);
            assert!(metadata + bytes.len() < limit);
            let ProcessFrame::Output {
                stdout_base64,
                stderr_base64,
            } = serde_json::from_slice(&bytes).unwrap()
            else {
                panic!("output expected")
            };
            assert_eq!(BASE64.decode(stdout_base64).unwrap(), b"out\n");
            assert_eq!(BASE64.decode(stderr_base64).unwrap(), b"err\n");
            assert!(process_capture_limit(limit, PROCESS_RESULT_RESERVE) > 0);
            let oversized = ProcessFrame::Output {
                stdout_base64: BASE64.encode(vec![b'x'; 8192]),
                stderr_base64: String::new(),
            };
            let (bytes, truncated) =
                bounded_output_frame(oversized, limit - PROCESS_RESULT_RESERVE)
                    .unwrap()
                    .unwrap();
            assert!(truncated);
            assert!(metadata + bytes.len() < limit);
            let ProcessFrame::Output { stdout_base64, .. } =
                serde_json::from_slice(&bytes).unwrap()
            else {
                panic!("output expected")
            };
            let prefix = BASE64.decode(stdout_base64).unwrap();
            assert!(!prefix.is_empty());
            assert!(prefix.iter().all(|byte| *byte == b'x'));
        }
    }

    #[test]
    fn network_completion_evidence_fits_after_output_budget_is_filled() {
        let exact_origins = (0..8)
            .map(|i| format!("https://{}{i}.example.test", "a".repeat(40)))
            .collect::<Vec<_>>();
        let wildcard_origins = (0..MAX_OBSERVED_ORIGINS)
            .map(|i| {
                format!(
                    "https://{}.{}.{}.{i:02}.test:65535",
                    "a".repeat(63),
                    "b".repeat(63),
                    "c".repeat(63)
                )
            })
            .collect::<Vec<_>>();
        for backend in ["native", "windows_job", "oci"] {
            for (destinations, observed, limit) in [
                (exact_origins.clone(), exact_origins.clone(), 4096),
                (vec!["*".into()], wildcard_origins.clone(), 65536),
            ] {
                let obligations = PolicyObligations {
                    sandbox_backend: backend.into(),
                    network_destinations: destinations,
                    ..PolicyObligations::default()
                };
                let reserve = process_result_reserve(&obligations);
                let mut result = SandboxJobResult {
                    backend: backend.into(),
                    exit_code: Some(i32::MIN),
                    success: false,
                    timed_out: false,
                    stopped: true,
                    resource_limit_exceeded: Some("process-count".into()),
                    output_truncated: true,
                    stdout_base64: String::new(),
                    stderr_base64: String::new(),
                    observed_origins: observed,
                };
                for origin in &result.observed_origins {
                    validate_observed_origin(origin).unwrap();
                }
                let started = serde_json::to_vec(&ProcessFrame::Started {
                    deadline_ms: u64::MAX,
                    timeout_ms: u64::MAX,
                    max_output_bytes: u64::MAX,
                })
                .unwrap()
                .len()
                    + 1;
                let completed = serde_json::to_vec(&ProcessFrame::Completed {
                    result: result.clone(),
                })
                .unwrap()
                .len()
                    + 1;
                assert!(
                    started + completed > PROCESS_RESULT_RESERVE,
                    "fixture must expose the old fixed reserve"
                );
                assert!(started + completed <= reserve);
                assert!(completed <= MAX_PROCESS_FRAME_BYTES);
                let mut budget = limit - reserve;
                let mut emitted = 0;
                loop {
                    let output = ProcessFrame::Output {
                        stdout_base64: BASE64.encode(vec![b'x'; OUTPUT_CHUNK_BYTES]),
                        stderr_base64: String::new(),
                    };
                    let Some((frame, truncated)) = bounded_output_frame(output, budget).unwrap()
                    else {
                        break;
                    };
                    assert!(frame.len() + 1 < MAX_PROCESS_FRAME_BYTES);
                    emitted += frame.len() + 1;
                    budget -= frame.len() + 1;
                    if truncated {
                        break;
                    }
                }
                assert!(emitted > 64);
                assert!(
                    started + emitted + completed <= limit,
                    "filled streamed output must retain the confirmed completion"
                );
                let captured = process_capture_limit(limit, reserve);
                result.stdout_base64 = BASE64.encode(vec![b'x'; captured / 2]);
                result.stderr_base64 = BASE64.encode(vec![b'y'; captured - captured / 2]);
                assert!(
                    serde_json::to_vec(&result).unwrap().len() <= limit,
                    "sync output must retain every recorded origin too"
                );
            }
        }
    }

    #[test]
    fn wildcard_origin_evidence_is_bounded_before_proxy_forwarding() {
        assert!(validate_observed_origin("https://example.test:8443").is_ok());
        assert!(validate_observed_origin(&format!("https://{}.test", "x".repeat(400))).is_err());
        let maximum_reserve =
            PROCESS_RESULT_RESERVE + MAX_OBSERVED_ORIGINS * (MAX_OBSERVED_ORIGIN_JSON_BYTES + 1);
        assert!(maximum_reserve < MAX_PROCESS_FRAME_BYTES);
    }

    #[test]
    fn proxy_credential_spanning_chunks_is_never_released() {
        let secret = "s".repeat(64);
        let mut bytes = vec![b'x'; OUTPUT_CHUNK_BYTES - 10];
        bytes.extend_from_slice(secret.as_bytes());
        bytes.extend_from_slice(b"tail");
        let mut cursor = 0;
        let first = next_output(&bytes, &mut cursor, Some(&secret), false);
        assert_eq!(first, vec![b'x'; OUTPUT_CHUNK_BYTES - 10]);
        let second = next_output(&bytes, &mut cursor, Some(&secret), true);
        assert!(
            !second
                .windows(secret.len())
                .any(|part| part == secret.as_bytes())
        );
        assert!(second.ends_with(b"tail"));
        assert_eq!(cursor, bytes.len());
    }
}
