//! Parent side of the authenticated, bounded process stream.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::AsyncBufReadExt as _;

/// Stop signal for one managed invocation; it never accepts an operating-system PID.
#[derive(Clone, Default)]
pub struct ProcessControl(Arc<AtomicBool>);

impl ProcessControl {
    /// Ask the supervisor to terminate and reap the invocation.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Whether stop has been requested (not proof of termination).
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

struct InputGuard(tokio::task::JoinHandle<()>);
impl Drop for InputGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl SandboxProcessExecutor {
    pub(super) async fn read_stream(
        &self,
        mut child: tokio::process::Child,
        stdin: tokio::process::ChildStdin,
        observer: &mut dyn QuarantinedEffectObserver,
        proxy: Option<&AllowlistProxy>,
        output_limit: u64,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let control = self.control.clone();
        // This task only owns the cancellation pipe. Dropping the adapter future
        // aborts it and closes the pipe, so the supervisor still reaps its child.
        let input = InputGuard(tokio::spawn(async move {
            let _stdin = stdin;
            while !control.is_cancelled() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }));
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| adapter_failure("process stream is absent"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| adapter_failure("process diagnostics are absent"))?;
        // Drain diagnostics without retaining unbounded data or exposing raw logs.
        let diagnostics = tokio::spawn(async move {
            let mut stderr = stderr;
            let mut buffer = [0; 4096];
            while stderr.read(&mut buffer).await.is_ok_and(|count| count != 0) {}
        });
        let mut reader = tokio::io::BufReader::new(stdout);
        let frames = async {
            let mut started = false;
            let mut terminal = None;
            let mut total = 0_u64;
            loop {
                let mut bytes = Vec::new();
                let count = (&mut reader)
                    .take((MAX_PROCESS_FRAME_BYTES + 1) as u64)
                    .read_until(b'\n', &mut bytes)
                    .await
                    .map_err(adapter_failure)?;
                if count == 0 {
                    break;
                }
                if count > MAX_PROCESS_FRAME_BYTES
                    || bytes.last() != Some(&b'\n')
                    || terminal.is_some()
                {
                    return Err(ExecutionError::OutcomeUnknown(
                        "invalid process stream framing".into(),
                    ));
                }
                total = total.saturating_add(count as u64);
                if total > output_limit {
                    return Err(ExecutionError::OutcomeUnknown(
                        "process stream exceeds authorized output limit".into(),
                    ));
                }
                let frame: ProcessFrame = serde_json::from_slice(&bytes).map_err(|_| {
                    ExecutionError::OutcomeUnknown("invalid process stream data".into())
                })?;
                match &frame {
                    ProcessFrame::Started { .. } if !started => started = true,
                    ProcessFrame::Output { .. } if started => {}
                    ProcessFrame::Completed { result }
                        if started || result.stopped || result.timed_out =>
                    {
                        terminal = Some(result.clone());
                        continue;
                    }
                    _ => {
                        return Err(ExecutionError::OutcomeUnknown(
                            "invalid process stream order".into(),
                        ));
                    }
                }
                let chunk = bounded_json(
                    serde_json::to_value(frame).map_err(adapter_failure)?,
                    MAX_PROCESS_FRAME_BYTES,
                )?;
                observer.observe(chunk).await?;
            }
            terminal.ok_or_else(|| {
                ExecutionError::OutcomeUnknown(
                    "process stream ended without confirmed termination".into(),
                )
            })
        }
        .await;
        drop(reader);
        drop(input);
        // Closing both pipes lets a blocked helper writer fail and stop supervision.
        // Never report a confirmed terminal state until the helper has exited.
        let status = child.wait().await.map_err(|_| {
            ExecutionError::OutcomeUnknown(
                "process helper termination could not be confirmed".into(),
            )
        })?;
        let _ = diagnostics.await;
        let mut result = frames?;
        if !status.success() {
            return Err(ExecutionError::OutcomeUnknown(
                "process helper failed before cleanup was confirmed".into(),
            ));
        }
        if let Some(proxy) = proxy {
            result.observed_origins = proxy.observed_origins();
        }
        let terminal = bounded_json(
            serde_json::to_value(ProcessFrame::Completed { result }).map_err(adapter_failure)?,
            usize::try_from(output_limit).map_err(adapter_failure)?,
        )?;
        observer.observe(terminal.clone()).await?;
        Ok(terminal)
    }
}
