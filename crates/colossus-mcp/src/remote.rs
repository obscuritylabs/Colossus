//! One permit owns one bounded HTTP discovery or invocation lifecycle.

use super::*;
use crate::executor::{RemoteOperationResult, failed, remote_call_failure};
use colossus_contracts::McpDiagnosticStage;
use http::{HeaderName, HeaderValue};
use rmcp::{
    model::CallToolResponse,
    service::{ClientLifecycleMode, ClientServiceExt as _},
    transport::{
        common::client_side_sse::NeverRetry,
        streamable_http_client::{
            StreamableHttpClient, StreamableHttpClientTransport,
            StreamableHttpClientTransportConfig,
        },
    },
};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub(super) async fn execute_remote_operation<C>(
    http: C,
    server: &ConfiguredServer,
    operation: &McpOperation,
    headers: HashMap<HeaderName, HeaderValue>,
    call_dispatched: &AtomicBool,
) -> Result<RemoteOperationResult, ExecutionError>
where
    C: StreamableHttpClient + Send + Sync,
{
    let diagnostics = McpDiagnosticCapture::current();
    diagnostics.stage(McpDiagnosticStage::Initialize);
    let endpoint = server
        .url
        .clone()
        .ok_or_else(|| failed("MCP HTTP endpoint is absent"))?;
    let mut config = StreamableHttpClientTransportConfig::with_uri(endpoint);
    config.retry_config = Arc::new(NeverRetry::default());
    config.allow_stateless = server.allow_stateless;
    config.reinit_on_expired_session = false;
    config.max_concurrent_requests = 1;
    config.max_sse_event_size =
        usize::try_from(server.max_output_bytes.unwrap_or(1024 * 1024)).unwrap_or(1024 * 1024);
    config.custom_headers = headers;
    let transport = StreamableHttpClientTransport::with_client(http, config);
    let lifecycle = match server.protocol_version {
        McpProtocolVersion::Auto => ClientLifecycleMode::Auto {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            legacy_version: Some(ProtocolVersion::LATEST_WITH_INITIALIZE),
        },
        McpProtocolVersion::V2026 => ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        },
        McpProtocolVersion::V2025 => ClientLifecycleMode::Initialize,
    };
    let client = InitializeRequestParams::new(
        ClientCapabilities::default(),
        Implementation::new("colossus", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE);
    let mut service = client
        .serve_with_lifecycle(transport, lifecycle)
        .await
        .map_err(|_| failed("MCP HTTP protocol negotiation failed"))?;
    let result = async {
        let info = service
            .peer_info()
            .ok_or_else(|| failed("MCP discovery metadata is absent"))?;
        let valid_mode = match server.protocol_version {
            McpProtocolVersion::V2026 => info.protocol_version == ProtocolVersion::V_2026_07_28,
            McpProtocolVersion::V2025 => info.protocol_version.has_initialize(),
            McpProtocolVersion::Auto => true,
        };
        if !valid_mode
            || info.capabilities.tools.is_none()
            || !ProtocolVersion::KNOWN_VERSIONS.contains(&info.protocol_version)
            || info.protocol_version > ProtocolVersion::LATEST
        {
            return Err(failed(
                "MCP server negotiated an unsupported protocol or omitted tools capability",
            ));
        }
        if let McpOperation::CallTool {
            tool, arguments, ..
        } = operation
        {
            // The request already binds the advertised schema, exact arguments,
            // and output validation to the permit. Re-listing here can mint a
            // different server-owned routing/session default after authorization.
            if info.protocol_version >= ProtocolVersion::STANDARD_HEADERS {
                crate::param_headers::call_headers(operation).map_err(failed)?;
            }
            let arguments = arguments
                .as_object()
                .cloned()
                .ok_or_else(|| failed("MCP tool arguments must be an object"))?;
            call_dispatched.store(true, Ordering::Release);
            return match service
                .call_tool_once(CallToolRequestParams::new(tool.clone()).with_arguments(arguments))
                .await
            {
                Ok(CallToolResponse::Complete(result)) => Ok(RemoteOperationResult::Call(result)),
                Ok(_) => Err(ExecutionError::OutcomeUnknown(
                    "MCP tool returned an unsupported task or client-input continuation".into(),
                )),
                Err(error) => remote_call_failure(error, operation),
            };
        }
        if matches!(
            operation,
            McpOperation::ListTools {
                cursor: Some(_),
                ..
            }
        ) {
            return Err(failed("MCP cursors cannot be resumed across connections"));
        }
        diagnostics.stage(McpDiagnosticStage::ListTools);
        let mut inventory = ListToolsResult::default();
        let mut cursor = None;
        let mut cursors = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut inventory_bytes = 0_usize;
        for page_number in 0..MAX_MCP_PAGES {
            let page = service
                .list_tools(Some(
                    PaginatedRequestParams::default().with_cursor(cursor.clone()),
                ))
                .await
                .map_err(|_| failed("MCP HTTP tool discovery failed"))?;
            inventory_bytes =
                inventory_bytes.saturating_add(serde_json::to_vec(&page).map_err(failed)?.len());
            if inventory_bytes > server.max_output_bytes.unwrap_or(1024 * 1024) as usize {
                return Err(failed("MCP discovery exceeded its output bound"));
            }
            for tool in &page.tools {
                if !names.insert(tool.name.to_string()) || names.len() > MAX_MCP_TOOLS {
                    return Err(failed(
                        "MCP discovery returned duplicate tools or exceeded its tool bound",
                    ));
                }
            }
            let next = page.next_cursor.clone();
            inventory.tools.extend(page.tools);
            let Some(next) = next else {
                break;
            };
            if next.is_empty()
                || next.len() > 8 * 1024
                || !cursors.insert(next.clone())
                || page_number + 1 == MAX_MCP_PAGES
            {
                return Err(failed(
                    "MCP discovery returned an invalid cursor or exceeded its page bound",
                ));
            }
            cursor = Some(next);
        }
        Ok(RemoteOperationResult::Tools(inventory))
    }
    .await;
    let _ = service.close_with_timeout(Duration::from_millis(500)).await;
    result
}
