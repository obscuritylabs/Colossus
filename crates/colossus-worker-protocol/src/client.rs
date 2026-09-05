use std::{sync::Arc, time::Duration};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    WorkerApprovalMode, WorkerControlError, WorkerSessionMap, WorkerThreadDelegateInspection,
    wire::{
        ClientHello, ControlFrameContent, ControlOperation, MAX_CLOCK_SKEW_MS, MAX_FRAME_BYTES,
        MAX_REQUEST_BYTES, PROTOCOL_VERSION, ServerHello, UnsignedFrame, UnsignedRequest,
        UnsignedServerHello, WorkerFrame, WorkerRequest, decode_frame_content, now_ms,
        read_message, request_tag, verify_tag, write_message,
    },
};

#[cfg(not(windows))]
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
#[cfg(windows)]
const CONNECT_TIMEOUT: Duration = Duration::from_secs(65);
#[cfg(not(windows))]
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(windows)]
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(not(windows))]
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(windows)]
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Narrow authenticated client for worker readiness and live approval-mode control.
#[derive(Clone)]
pub struct WorkerControlClient {
    pub(crate) endpoint: String,
    pub(crate) authentication_key: Arc<Zeroizing<[u8; 32]>>,
}

impl WorkerControlClient {
    /// Bind a native-delivered worker endpoint to its independent 256-bit key.
    pub fn new(
        endpoint: impl Into<String>,
        authentication_key: Zeroizing<[u8; 32]>,
    ) -> Result<Self, WorkerControlError> {
        let endpoint = endpoint.into();
        if endpoint.is_empty() || endpoint.len() > 4_096 || endpoint.contains('\0') {
            return Err(WorkerControlError::Protocol(
                "worker control endpoint is invalid".into(),
            ));
        }
        Ok(Self {
            endpoint,
            authentication_key: Arc::new(authentication_key),
        })
    }

    /// Return the current worker-wide mode after an authenticated readiness probe.
    pub async fn approval_mode(&self) -> Result<Option<WorkerApprovalMode>, WorkerControlError> {
        let result = self.call(ControlOperation::Ping).await?;
        match result.get("approval_mode") {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(mode) => serde_json::from_value(mode.clone())
                .map(Some)
                .map_err(|error| WorkerControlError::Protocol(error.to_string())),
        }
    }

    /// Flush live host-owned OTLP exporters without releasing configuration or payloads.
    pub async fn observability_doctor(&self) -> Result<serde_json::Value, WorkerControlError> {
        self.call(ControlOperation::ObservabilityDoctor).await
    }

    /// Change the worker-wide mode used outside client-scoped overrides.
    pub async fn set_approval_mode(
        &self,
        approval_mode: WorkerApprovalMode,
    ) -> Result<WorkerApprovalMode, WorkerControlError> {
        let result = self
            .call(ControlOperation::SetApprovalMode { approval_mode })
            .await?;
        serde_json::from_value(result.get("approval_mode").cloned().ok_or_else(|| {
            WorkerControlError::Protocol("worker control response omitted approval mode".into())
        })?)
        .map_err(|error| WorkerControlError::Protocol(error.to_string()))
    }

    /// Return one bounded child-agent inspection after worker-side lineage validation.
    pub async fn inspect_thread_delegate(
        &self,
        parent_run_id: &str,
        job_id: &str,
    ) -> Result<WorkerThreadDelegateInspection, WorkerControlError> {
        if parent_run_id.is_empty()
            || parent_run_id.len() > 128
            || job_id.is_empty()
            || job_id.len() > 128
        {
            return Err(WorkerControlError::Protocol(
                "delegate inspection identifiers are invalid".into(),
            ));
        }
        let result = self
            .call(ControlOperation::InspectThreadDelegate {
                parent_run_id: parent_run_id.into(),
                job_id: job_id.into(),
            })
            .await?;
        serde_json::from_value(result)
            .map_err(|error| WorkerControlError::Protocol(error.to_string()))
    }

    /// Return bounded canonical resources after native selected-session validation.
    pub async fn inspect_session_map(
        &self,
        session_id: &str,
    ) -> Result<WorkerSessionMap, WorkerControlError> {
        if session_id.is_empty() || session_id.len() > 128 {
            return Err(WorkerControlError::Protocol(
                "session map identifier is invalid".into(),
            ));
        }
        let result = self
            .call(ControlOperation::InspectSessionMap {
                session_id: session_id.into(),
            })
            .await?;
        serde_json::from_value(result)
            .map_err(|error| WorkerControlError::Protocol(error.to_string()))
    }

    /// Return bounded configured MCP server metadata from the live runtime.
    pub async fn mcp_servers(&self) -> Result<serde_json::Value, WorkerControlError> {
        self.call(ControlOperation::McpServers).await
    }

    /// Discover allowlist-filtered MCP tools through the live runtime boundary.
    pub async fn mcp_tools(
        &self,
        server: Option<&str>,
    ) -> Result<serde_json::Value, WorkerControlError> {
        if server.is_some_and(|server| !valid_mcp_server_name(server)) {
            return Err(WorkerControlError::Protocol(
                "MCP server name is invalid".into(),
            ));
        }
        self.call(ControlOperation::McpTools {
            server: server.map(str::to_owned),
        })
        .await
    }

    /// Begin an MCP OAuth authorization-code flow in the live runtime.
    pub async fn mcp_oauth_begin(
        &self,
        server: &str,
    ) -> Result<serde_json::Value, WorkerControlError> {
        validate_mcp_server_name(server)?;
        self.call(ControlOperation::McpAuthBegin {
            server: server.into(),
        })
        .await
    }

    /// Complete an MCP OAuth flow with the exact loopback callback URL.
    pub async fn mcp_oauth_complete(
        &self,
        server: &str,
        callback_url: &str,
    ) -> Result<serde_json::Value, WorkerControlError> {
        validate_mcp_server_name(server)?;
        if callback_url.is_empty() || callback_url.len() > 8_192 || callback_url.contains('\0') {
            return Err(WorkerControlError::Protocol(
                "MCP OAuth callback URL is invalid".into(),
            ));
        }
        self.call(ControlOperation::McpAuthComplete {
            server: server.into(),
            callback_url: callback_url.into(),
        })
        .await
    }

    /// Return the live MCP OAuth credential status without credential material.
    pub async fn mcp_oauth_status(
        &self,
        server: &str,
    ) -> Result<serde_json::Value, WorkerControlError> {
        validate_mcp_server_name(server)?;
        self.call(ControlOperation::McpAuthStatus {
            server: server.into(),
        })
        .await
    }

    /// Remove the live MCP OAuth credential through the runtime-owned store.
    pub async fn mcp_oauth_logout(
        &self,
        server: &str,
    ) -> Result<serde_json::Value, WorkerControlError> {
        validate_mcp_server_name(server)?;
        self.call(ControlOperation::McpAuthLogout {
            server: server.into(),
        })
        .await
    }

    /// Exercise one provider catalog endpoint without releasing response bodies.
    pub async fn provider_doctor(
        &self,
        profile: &str,
    ) -> Result<serde_json::Value, WorkerControlError> {
        validate_profile_name(profile)?;
        self.call(ControlOperation::ProviderDoctor {
            profile: Some(profile.into()),
            include_provider_response: false,
        })
        .await
    }

    /// Exercise one bounded model generation without releasing response bodies.
    pub async fn model_doctor(
        &self,
        profile: &str,
    ) -> Result<serde_json::Value, WorkerControlError> {
        validate_profile_name(profile)?;
        self.call(ControlOperation::ModelDoctor {
            profile: Some(profile.into()),
            include_provider_response: false,
        })
        .await
    }

    /// Exercise one configured search role with a fixed bounded diagnostic query.
    pub async fn search_doctor(&self, role: &str) -> Result<serde_json::Value, WorkerControlError> {
        if !matches!(role, "agent" | "research") {
            return Err(WorkerControlError::Protocol(
                "search role is invalid".into(),
            ));
        }
        self.call(ControlOperation::SearchQuery {
            role: role.into(),
            query: "Colossus connectivity test".into(),
            limit: 1,
        })
        .await
    }

    /// Return the active Agent Plugins inventory from the live runtime.
    pub async fn plugins(&self) -> Result<serde_json::Value, WorkerControlError> {
        self.call(ControlOperation::PluginsInventory).await
    }

    /// Request one typed operator plugin operation; no field constitutes approval proof.
    pub async fn manage_plugin(
        &self,
        request: colossus_contracts::PluginManagementRequest,
    ) -> Result<serde_json::Value, WorkerControlError> {
        self.call(ControlOperation::PluginManage { request }).await
    }

    /// Return bounded registered-workflow event metadata from the live runtime.
    pub async fn workflows(&self) -> Result<serde_json::Value, WorkerControlError> {
        self.call(ControlOperation::WorkflowList).await
    }

    async fn call(
        &self,
        operation: ControlOperation,
    ) -> Result<serde_json::Value, WorkerControlError> {
        let mut stream = self.connect().await?;
        let connection_nonce = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            client_handshake(&mut stream, self.authentication_key.as_ref()),
        )
        .await
        .map_err(|_| WorkerControlError::Busy)??;
        let request = signed_request(
            self.authentication_key.as_ref(),
            operation,
            &connection_nonce,
        )?;
        let frame: WorkerFrame = tokio::time::timeout(REQUEST_TIMEOUT, async {
            write_message(&mut stream, &request, MAX_REQUEST_BYTES).await?;
            read_message(&mut stream, MAX_FRAME_BYTES).await
        })
        .await
        .map_err(|_| WorkerControlError::Busy)??;
        validate_frame(
            self.authentication_key.as_ref(),
            &request.request_id,
            &frame,
        )?;
        match decode_frame_content(&frame)? {
            ControlFrameContent::Complete { result } => Ok(result),
            ControlFrameContent::Error { message } => Err(WorkerControlError::Remote(message)),
        }
    }

    pub(crate) async fn connect(&self) -> Result<platform::ClientStream, WorkerControlError> {
        platform::validate_endpoint(&self.endpoint)?;
        match tokio::time::timeout(CONNECT_TIMEOUT, platform::connect(&self.endpoint)).await {
            Err(_) => Err(WorkerControlError::Busy),
            Ok(Ok(stream)) => Ok(stream),
            Ok(Err(error)) if platform::connection_is_busy(&error) => Err(WorkerControlError::Busy),
            Ok(Err(error)) if platform::connection_is_absent(&error) => {
                Err(WorkerControlError::Unavailable)
            }
            Ok(Err(error)) => Err(WorkerControlError::Io(error)),
        }
    }
}

fn validate_mcp_server_name(server: &str) -> Result<(), WorkerControlError> {
    if valid_mcp_server_name(server) {
        Ok(())
    } else {
        Err(WorkerControlError::Protocol(
            "MCP server name is invalid".into(),
        ))
    }
}

fn valid_mcp_server_name(server: &str) -> bool {
    if let Some((plugin, component)) = server.split_once('/') {
        return !component.contains('/')
            && valid_mcp_server_name(plugin)
            && valid_mcp_server_name(component);
    }
    !server.is_empty()
        && server.len() <= 128
        && server
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_profile_name(profile: &str) -> Result<(), WorkerControlError> {
    if !profile.is_empty()
        && profile.len() <= 128
        && profile
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        Ok(())
    } else {
        Err(WorkerControlError::Protocol(
            "managed profile name is invalid".into(),
        ))
    }
}

pub(crate) async fn client_handshake<S>(
    stream: &mut S,
    key: &[u8; 32],
) -> Result<String, WorkerControlError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut challenge = [0_u8; 32];
    getrandom::fill(&mut challenge)
        .map_err(|error| WorkerControlError::Protocol(error.to_string()))?;
    let challenge = hex::encode(challenge);
    write_message(
        stream,
        &ClientHello {
            version: PROTOCOL_VERSION,
            challenge: challenge.clone(),
        },
        1024,
    )
    .await?;
    let hello: ServerHello = read_message(stream, 1024).await?;
    if hello.version != PROTOCOL_VERSION
        || hello.challenge != challenge
        || hello.server_nonce.len() != 64
        || hex::decode(&hello.server_nonce).map_or(true, |bytes| bytes.len() != 32)
        || (now_ms() - hello.timestamp_ms).abs() > MAX_CLOCK_SKEW_MS
    {
        return Err(WorkerControlError::Protocol(
            "worker server protocol is incompatible or its handshake is invalid".into(),
        ));
    }
    verify_tag(
        key,
        &UnsignedServerHello {
            version: hello.version,
            challenge: &hello.challenge,
            server_nonce: &hello.server_nonce,
            timestamp_ms: hello.timestamp_ms,
        },
        &hello.authentication_tag,
        "worker server handshake",
    )?;
    Ok(hello.server_nonce)
}

pub(crate) fn signed_request(
    key: &[u8; 32],
    operation: ControlOperation,
    connection_nonce: &str,
) -> Result<WorkerRequest, WorkerControlError> {
    let request_id = Uuid::now_v7().to_string();
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce).map_err(|error| WorkerControlError::Protocol(error.to_string()))?;
    let nonce = hex::encode(nonce);
    let timestamp_ms = now_ms();
    let authentication_tag = request_tag(
        key,
        &UnsignedRequest {
            version: PROTOCOL_VERSION,
            request_id: &request_id,
            timestamp_ms,
            nonce: &nonce,
            connection_nonce,
            operation: &operation,
        },
    )?;
    Ok(WorkerRequest {
        version: PROTOCOL_VERSION,
        request_id,
        timestamp_ms,
        nonce,
        connection_nonce: connection_nonce.into(),
        operation,
        authentication_tag,
    })
}

fn validate_frame(
    key: &[u8; 32],
    request_id: &str,
    frame: &WorkerFrame,
) -> Result<(), WorkerControlError> {
    if frame.version != PROTOCOL_VERSION
        || frame.request_id != request_id
        || frame.sequence != 1
        || (now_ms() - frame.timestamp_ms).abs() > MAX_CLOCK_SKEW_MS
    {
        return Err(WorkerControlError::Protocol(
            "worker control response metadata is invalid".into(),
        ));
    }
    verify_tag(
        key,
        &UnsignedFrame {
            version: frame.version,
            request_id: &frame.request_id,
            sequence: frame.sequence,
            timestamp_ms: frame.timestamp_ms,
            content_base64: &frame.content_base64,
        },
        &frame.authentication_tag,
        "worker response",
    )
}

#[cfg(all(test, unix))]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt as _};
    use tokio::net::UnixListener;

    use super::{REQUEST_TIMEOUT, WorkerControlClient};
    use crate::{
        WorkerControlError,
        wire::{
            ClientHello, PROTOCOL_VERSION, ServerHello, UnsignedServerHello, now_ms, read_message,
            request_tag, write_message,
        },
    };

    #[tokio::test]
    async fn a_worker_that_stalls_after_the_handshake_does_not_block_forever() {
        let key = zeroize::Zeroizing::new([7_u8; 32]);
        let directory = tempfile::tempdir().expect("temporary directory");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
            .expect("owner-only directory");
        let endpoint = directory.path().join("control.sock");
        let listener = UnixListener::bind(&endpoint).expect("listener");
        fs::set_permissions(&endpoint, fs::Permissions::from_mode(0o600))
            .expect("owner-only socket");
        let server_key = key.clone();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accepted connection");
            let hello: ClientHello = read_message(&mut stream, 1024).await.expect("client hello");
            let server_nonce = hex::encode([9_u8; 32]);
            let timestamp_ms = now_ms();
            let authentication_tag = request_tag(
                &server_key,
                &UnsignedServerHello {
                    version: PROTOCOL_VERSION,
                    challenge: &hello.challenge,
                    server_nonce: &server_nonce,
                    timestamp_ms,
                },
            )
            .expect("server hello tag");
            write_message(
                &mut stream,
                &ServerHello {
                    version: PROTOCOL_VERSION,
                    challenge: hello.challenge,
                    server_nonce,
                    timestamp_ms,
                    authentication_tag,
                },
                1024,
            )
            .await
            .expect("server hello");
            // Never answer the request, holding the connection open.
            std::future::pending::<()>().await;
        });

        let client =
            WorkerControlClient::new(endpoint.to_string_lossy().into_owned(), key).expect("client");
        let started = std::time::Instant::now();
        let error = client
            .approval_mode()
            .await
            .expect_err("stalled worker response");
        assert!(matches!(error, WorkerControlError::Busy), "{error:?}");
        assert!(started.elapsed() < REQUEST_TIMEOUT * 4);
        server.abort();
    }
}

#[cfg(unix)]
mod platform {
    use std::{
        fs,
        os::unix::fs::{FileTypeExt as _, MetadataExt as _},
        path::Path,
    };
    use tokio::net::UnixStream;

    use crate::{WorkerControlError, endpoint::shortened_endpoint_root};

    pub(crate) type ClientStream = UnixStream;

    pub(crate) async fn connect(endpoint: &str) -> Result<ClientStream, std::io::Error> {
        UnixStream::connect(endpoint).await
    }

    pub(crate) fn connection_is_busy(_error: &std::io::Error) -> bool {
        false
    }

    pub(crate) fn connection_is_absent(error: &std::io::Error) -> bool {
        matches!(
            error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
        )
    }

    pub(crate) fn validate_endpoint(endpoint: &str) -> Result<(), WorkerControlError> {
        let path = Path::new(endpoint);
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(WorkerControlError::Unavailable);
            }
            Err(error) => return Err(error.into()),
        };
        let parent = path.parent().ok_or_else(|| {
            WorkerControlError::Protocol("worker endpoint has no parent directory".into())
        })?;
        let parent_metadata = fs::metadata(parent)?;
        if !metadata.file_type().is_socket()
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != parent_metadata.uid()
        {
            return Err(WorkerControlError::Protocol(
                "worker endpoint is not an owner-only socket in its owning directory".into(),
            ));
        }
        if parent == shortened_endpoint_root() {
            let owner = rustix::process::geteuid().as_raw();
            if !parent_metadata.is_dir()
                || parent_metadata.uid() != owner
                || parent_metadata.mode() & 0o077 != 0
            {
                return Err(WorkerControlError::Protocol(
                    "worker endpoint private directory is not owner-only".into(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
mod platform {
    use std::{io::ErrorKind, time::Duration};
    use tokio::{
        net::windows::named_pipe::{ClientOptions, NamedPipeClient},
        time::{Instant, sleep},
    };

    use crate::WorkerControlError;

    pub(crate) type ClientStream = NamedPipeClient;
    const ERROR_PIPE_BUSY: i32 = 231;

    pub(crate) async fn connect(endpoint: &str) -> Result<ClientStream, std::io::Error> {
        let missing_deadline = Instant::now() + Duration::from_secs(2);
        let busy_deadline = Instant::now() + Duration::from_secs(60);
        loop {
            match ClientOptions::new().open(endpoint) {
                Ok(client) => return Ok(client),
                Err(error) if connection_is_busy(&error) && Instant::now() < busy_deadline => {
                    sleep(Duration::from_millis(10)).await;
                }
                Err(error)
                    if error.kind() == ErrorKind::NotFound && Instant::now() < missing_deadline =>
                {
                    sleep(Duration::from_millis(10)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub(crate) fn connection_is_busy(error: &std::io::Error) -> bool {
        error.kind() == ErrorKind::WouldBlock || error.raw_os_error() == Some(ERROR_PIPE_BUSY)
    }

    pub(crate) fn connection_is_absent(error: &std::io::Error) -> bool {
        error.kind() == ErrorKind::NotFound
    }

    pub(crate) fn validate_endpoint(endpoint: &str) -> Result<(), WorkerControlError> {
        if endpoint.starts_with(r"\\.\pipe\colossus-") {
            Ok(())
        } else {
            Err(WorkerControlError::Protocol(
                "worker named-pipe endpoint is invalid".into(),
            ))
        }
    }
}
