//! Explicit terminal-user commands share shell preparation and the effect gateway.

use super::*;
use colossus_policy::{QuarantinedEffectObserver, StreamingEffectExecutor};
use colossus_sandbox::ProcessControl;

const MAX_COMMAND_BYTES: usize = 64 * 1024;
const MAX_VISIBLE_OUTPUT_BYTES: usize = 64 * 1024;

pub(super) struct UserShellExecutor {
    pub(super) tools: Arc<GatewayToolExecutor>,
    pub(super) process: Arc<SandboxProcessExecutor>,
    pub(super) identity: workspace_lease::WorkspaceIdentity,
}

impl Runtime {
    /// Run shell source entered by a local operator, without invoking a model.
    ///
    /// The session's execution boundary, policy, approvals, sandbox, audit and output
    /// release remain mandatory. Each invocation starts in the workspace with the
    /// same environment rules as `shell.run`; shell state does not persist between
    /// invocations. Cancellation awaits authenticated process cleanup. Only bounded,
    /// released stdout/stderr and confirmed terminal status are returned as JSON.
    pub async fn run_user_shell(
        &self,
        session_id: &str,
        command: &str,
        control: &RunControl,
    ) -> Result<Value, RuntimeError> {
        self.native_session_lifecycle.during(session_id, Box::pin(async {
        if command.trim().is_empty() || command.contains('\0') || command.len() > MAX_COMMAND_BYTES
        {
            return Err(RuntimeError::Config(
                "shell commands must be nonempty, at most 64 KiB and contain no NUL bytes".into(),
            ));
        }
        if self.get_session(session_id)?.is_none() {
            return Err(StoreError::NotFound("shell command session was not found".into()).into());
        }
        self.user_shell.identity.revalidate()?;
        if control.is_cancelled() {
            return Err(RuntimeError::Config(
                "shell command cancelled before launch".into(),
            ));
        }
        let context = ExecutionContext {
            correlation_id: Uuid::now_v7().to_string(),
            session_id: Some(session_id.into()),
            ..ExecutionContext::default()
        };
        let (request, _isolated) = self
            .user_shell
            .tools
            .prepare_user_shell(command, context)
            .map_err(|error| RuntimeError::Agent(AgentError::Tool(error)))?;
        let cancellation = ShellCancellation::new(control.clone());
        let executor = ControlledShell {
            identity: self.user_shell.identity.clone(),
            process: self
                .user_shell
                .process
                .controlled(cancellation.control.clone()),
        };
        let mut output = ShellOutput::default();
        let released = self
            .gateway
            .execute_stream(request, &executor, &mut output)
            .await?;
        let terminal: Value = serde_json::from_slice(&released.bytes)
            .map_err(|_| RuntimeError::Config("invalid released shell completion".into()))?;
        if terminal["kind"] != "completed" || !terminal["result"].is_object() {
            return Err(RuntimeError::Config("shell completion is absent".into()));
        }
        let result = &terminal["result"];
        Ok(json!({
            "stdout": String::from_utf8_lossy(&output.stdout),
            "stderr": String::from_utf8_lossy(&output.stderr),
            "exit_code": result["exit_code"],
            "stopped": result["stopped"].as_bool().unwrap_or(false),
            "timed_out": result["timed_out"].as_bool().unwrap_or(false),
            "truncated": output.truncated || result["output_truncated"].as_bool().unwrap_or(false),
        }))
        })).await?
    }
}

/// Dropping an invocation fences its supervisor; the helper owns process reaping.
struct ShellCancellation {
    control: ProcessControl,
    monitor: tokio::task::JoinHandle<()>,
}

impl ShellCancellation {
    fn new(run: RunControl) -> Self {
        let control = ProcessControl::default();
        let monitor_control = control.clone();
        let monitor = tokio::spawn(async move {
            while !run.is_cancelled() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            monitor_control.cancel();
        });
        Self { control, monitor }
    }
}

impl Drop for ShellCancellation {
    fn drop(&mut self) {
        self.control.cancel();
        self.monitor.abort();
    }
}

struct ControlledShell {
    identity: workspace_lease::WorkspaceIdentity,
    process: SandboxProcessExecutor,
}

#[async_trait]
impl StreamingEffectExecutor for ControlledShell {
    async fn execute_stream(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
        observer: &mut dyn QuarantinedEffectObserver,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        self.identity
            .revalidate()
            .map_err(|_| ExecutionError::Failed("workspace identity changed".into()))?;
        self.process.execute_stream(request, permit, observer).await
    }
}

#[derive(Default)]
struct ShellOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    truncated: bool,
}

#[async_trait]
impl ReleasedEffectObserver for ShellOutput {
    async fn observe(&mut self, released: ReleasedEffectResult) -> Result<(), ExecutionError> {
        let frame: Value = serde_json::from_slice(&released.bytes)
            .map_err(|_| ExecutionError::OutcomeUnknown("invalid released shell frame".into()))?;
        if frame["kind"] != "output" {
            return Ok(());
        }
        for field in ["stdout_base64", "stderr_base64"] {
            let encoded = frame[field].as_str().ok_or_else(|| {
                ExecutionError::OutcomeUnknown("released shell output is absent".into())
            })?;
            let bytes = BASE64.decode(encoded).map_err(|_| {
                ExecutionError::OutcomeUnknown("invalid released shell output".into())
            })?;
            let available =
                MAX_VISIBLE_OUTPUT_BYTES.saturating_sub(self.stdout.len() + self.stderr.len());
            let retained = bytes.len().min(available);
            self.truncated |= retained < bytes.len();
            let target = if field == "stdout_base64" {
                &mut self.stdout
            } else {
                &mut self.stderr
            };
            target.extend_from_slice(&bytes[..retained]);
        }
        Ok(())
    }
}
