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
    pub(super) fn new(mut input: impl Read + Send + 'static, output_limit: u64) -> Self {
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
                usize::try_from(output_limit)
                    .unwrap_or(usize::MAX)
                    .saturating_sub(4096),
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
            if self.is_truncated() || bytes.len() > *budget {
                self.truncated.store(true, Ordering::Release);
                return Ok(());
            }
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
