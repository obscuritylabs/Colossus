use crate::MAX_ACTIVE_WATCH_STREAMS;
use crate::auth::{
    MAX_CONCURRENT_AUTHENTICATED_DECODES, MAX_CONCURRENT_AUTHENTICATED_DECODES_PER_APPLICATION,
};
use crate::server::{
    MAX_ACCEPTED_CONNECTIONS, MAX_CONCURRENT_REQUESTS_PER_CONNECTION, MAX_CONCURRENT_STREAMS,
    MAX_CONNECTION_AGE, MAX_GLOBAL_CONCURRENT_REQUESTS, MAX_REQUEST_MESSAGE_BYTES,
    MAX_REQUEST_SETUP_DURATION, MAX_RESPONSE_MESSAGE_BYTES, RESERVED_UNARY_REQUEST_HEADROOM,
};
use colossus_api::{CallerContext, PLAN_CONTINUATION_CAPABILITY, scopes};
use colossus_api_proto::v1alpha1::{
    ApiLimit, Capability, DeploymentMode, GetReadinessRequest, GetReadinessResponse,
    GetServerInfoRequest, GetServerInfoResponse, ReadinessCheck, ReadinessStatus, ServerInfo,
    system_service_server::SystemService,
};
use prost_types::Timestamp;
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tonic::{Request, Response, Status};

const API_PACKAGE: &str = "colossus.api.v1alpha1";

/// Credential-free server identity and compatibility metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemMetadata {
    /// Stable UUID-form instance identifier.
    pub instance_id: String,
    /// Colossus semantic version.
    pub server_version: String,
    /// Active runtime placement.
    pub deployment_mode: DeploymentMode,
}

/// One bounded caller-visible readiness result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicReadiness {
    /// Aggregate status.
    pub status: ReadinessStatus,
    /// Bounded checks with no raw adapter errors.
    pub checks: Vec<ReadinessCheck>,
}

/// Supplies safe readiness without exposing diagnostic internals.
pub trait ReadinessProvider: Send + Sync {
    /// Return a caller-authorized readiness snapshot.
    fn readiness(&self, caller: &CallerContext) -> PublicReadiness;
}

/// Fixed readiness provider useful for tests, sidecars, and embedded composition.
#[derive(Clone, Debug)]
pub struct FixedReadiness(PublicReadiness);

impl FixedReadiness {
    /// Construct a fixed snapshot.
    pub fn new(readiness: PublicReadiness) -> Self {
        Self(readiness)
    }

    /// Construct a ready snapshot.
    pub fn ready() -> Self {
        Self(PublicReadiness {
            status: ReadinessStatus::Ready,
            checks: vec![ReadinessCheck {
                name: "runtime".into(),
                status: ReadinessStatus::Ready as i32,
                detail: "the runtime can accept work".into(),
            }],
        })
    }
}

impl ReadinessProvider for FixedReadiness {
    fn readiness(&self, _caller: &CallerContext) -> PublicReadiness {
        self.0.clone()
    }
}

/// Authenticated implementation of the public system service.
#[derive(Clone)]
pub struct SystemServiceAdapter {
    metadata: SystemMetadata,
    readiness: Arc<dyn ReadinessProvider>,
    application_limits: Vec<ApiLimit>,
}

impl SystemServiceAdapter {
    /// Compose metadata and a bounded readiness provider.
    pub fn new(metadata: SystemMetadata, readiness: Arc<dyn ReadinessProvider>) -> Self {
        Self {
            metadata,
            readiness,
            application_limits: default_application_limits(),
        }
    }

    /// Replace application-resource limits with values enforced by the host.
    pub fn with_application_limits(mut self, limits: Vec<ApiLimit>) -> Self {
        self.application_limits = limits;
        self
    }
}

#[tonic::async_trait]
impl SystemService for SystemServiceAdapter {
    async fn get_server_info(
        &self,
        request: Request<GetServerInfoRequest>,
    ) -> Result<Response<GetServerInfoResponse>, Status> {
        let caller = caller_context(&request)?;
        let capabilities = [
            ("agent_runs.create", scopes::RUNS_EXECUTE),
            ("agent_runs.read", scopes::RUNS_READ),
            ("agent_runs.cancel", scopes::RUNS_CONTROL),
            ("prompts.respond", scopes::PROMPTS_RESPOND),
            ("approvals.respond", scopes::APPROVALS_RESPOND),
            ("artifacts.read", scopes::ARTIFACTS_READ),
            ("artifacts.upload", scopes::ARTIFACTS_WRITE),
        ]
        .into_iter()
        .map(|(name, scope)| Capability {
            name: name.into(),
            enabled: caller.principal().has_scope(scope),
            detail: String::new(),
        })
        .chain(std::iter::once(Capability {
            name: "research.create".into(),
            enabled: caller.principal().has_scope(scopes::RUNS_EXECUTE),
            detail: String::new(),
        }))
        .chain(std::iter::once(Capability {
            name: "agent_runs.delegation".into(),
            enabled: caller.principal().has_scope(scopes::RUNS_EXECUTE)
                && caller.principal().allows_tool("agent.delegate"),
            detail: String::new(),
        }))
        .chain(std::iter::once(Capability {
            name: PLAN_CONTINUATION_CAPABILITY.into(),
            enabled: caller.principal().has_scope(scopes::RUNS_EXECUTE)
                && caller.principal().has_scope(scopes::RUNS_READ),
            detail: String::new(),
        }))
        .chain(std::iter::once(Capability {
            name: "attachments.run_input".into(),
            enabled: caller.principal().has_scope(scopes::ARTIFACTS_READ)
                && caller.principal().has_scope(scopes::ARTIFACTS_WRITE)
                && caller.principal().has_scope(scopes::RUNS_EXECUTE),
            detail: String::new(),
        }))
        .collect();
        let mut limits = self.application_limits.clone();
        limits.extend([
            ApiLimit {
                name: "transport.grpc_request_message".into(),
                value: MAX_REQUEST_MESSAGE_BYTES as u64,
                unit: "bytes".into(),
            },
            ApiLimit {
                name: "transport.grpc_response_message".into(),
                value: MAX_RESPONSE_MESSAGE_BYTES as u64,
                unit: "bytes".into(),
            },
            ApiLimit {
                name: "transport.concurrent_connections".into(),
                value: MAX_ACCEPTED_CONNECTIONS as u64,
                unit: "connections".into(),
            },
            ApiLimit {
                name: "transport.concurrent_request_setups".into(),
                value: MAX_GLOBAL_CONCURRENT_REQUESTS as u64,
                unit: "requests".into(),
            },
            ApiLimit {
                name: "transport.concurrent_request_setups_per_connection".into(),
                value: MAX_CONCURRENT_REQUESTS_PER_CONNECTION as u64,
                unit: "requests".into(),
            },
            ApiLimit {
                name: "transport.concurrent_streams_per_connection".into(),
                value: MAX_CONCURRENT_STREAMS as u64,
                unit: "streams".into(),
            },
            ApiLimit {
                name: "transport.active_watch_streams".into(),
                value: MAX_ACTIVE_WATCH_STREAMS as u64,
                unit: "streams".into(),
            },
            ApiLimit {
                name: "transport.reserved_unary_request_headroom".into(),
                value: RESERVED_UNARY_REQUEST_HEADROOM as u64,
                unit: "requests".into(),
            },
            ApiLimit {
                name: "transport.concurrent_authenticated_decodes".into(),
                value: MAX_CONCURRENT_AUTHENTICATED_DECODES as u64,
                unit: "requests".into(),
            },
            ApiLimit {
                name: "transport.concurrent_authenticated_decodes_per_application".into(),
                value: MAX_CONCURRENT_AUTHENTICATED_DECODES_PER_APPLICATION as u64,
                unit: "requests".into(),
            },
            ApiLimit {
                name: "transport.connection_age".into(),
                value: MAX_CONNECTION_AGE.as_secs(),
                unit: "seconds".into(),
            },
            ApiLimit {
                name: "transport.request_setup_timeout".into(),
                value: MAX_REQUEST_SETUP_DURATION.as_secs(),
                unit: "seconds".into(),
            },
        ]);
        let info = ServerInfo {
            instance_id: self.metadata.instance_id.clone(),
            server_version: self.metadata.server_version.clone(),
            api_packages: vec![API_PACKAGE.into()],
            deployment_mode: self.metadata.deployment_mode as i32,
            capabilities,
            limits,
            deprecations: Vec::new(),
            server_time: Some(now_timestamp()?),
        };
        Ok(Response::new(GetServerInfoResponse {
            server_info: Some(info),
        }))
    }

    async fn get_readiness(
        &self,
        request: Request<GetReadinessRequest>,
    ) -> Result<Response<GetReadinessResponse>, Status> {
        let caller = caller_context(&request)?;
        let readiness = self.readiness.readiness(caller);
        if readiness.status == ReadinessStatus::Unspecified
            || readiness.checks.iter().any(|check| {
                ReadinessStatus::try_from(check.status)
                    .is_ok_and(|status| status == ReadinessStatus::Unspecified)
            })
        {
            return Err(Status::internal("readiness invariant failed"));
        }
        Ok(Response::new(GetReadinessResponse {
            status: readiness.status as i32,
            checks: readiness.checks,
        }))
    }
}

fn default_application_limits() -> Vec<ApiLimit> {
    [
        ("request.input", 1_048_576, "bytes"),
        ("request.input_parts", 128, "items"),
        ("stream.run_updates_page", 16, "items"),
        ("list.page", 3, "runs"),
        ("list.owner_index_read_batch", 8, "events"),
        ("list.owner_index_events_scanned", 64, "events"),
        ("list.run_stream_events", 4_099, "events/run"),
        ("list.reconstruct_run_events", 16_396, "events/request"),
        ("run.nonterminal_sequence_ceiling", 4_096, "sequence"),
        ("run.stream_events", 4_099, "events/run"),
        ("run.released_bytes", 16 * 1_048_576, "bytes"),
        ("run.max_turns", 100, "turns"),
        ("run.active_global", 32, "runs"),
        ("run.active_per_application", 8, "runs"),
        ("run.create_rate_global", 4, "runs/second"),
        ("run.create_burst_global", 16, "runs"),
        ("run.create_rate_per_application", 1, "runs/second"),
        ("run.create_burst_per_application", 4, "runs"),
        ("watch.active_global", 64, "streams"),
        ("watch.active_per_application", 8, "streams"),
        ("list.concurrent_global", 4, "requests"),
        ("list.concurrent_per_application", 1, "requests"),
        ("list.rate_global", 8, "requests/second"),
        ("list.burst_global", 8, "requests"),
        ("list.rate_per_application", 2, "requests/second"),
        ("list.burst_per_application", 2, "requests"),
    ]
    .into_iter()
    .map(|(name, value, unit)| ApiLimit {
        name: name.into(),
        value,
        unit: unit.into(),
    })
    .collect()
}

pub(crate) fn caller_context<T>(request: &Request<T>) -> Result<&CallerContext, Status> {
    request
        .extensions()
        .get::<CallerContext>()
        .ok_or_else(|| Status::unauthenticated("authenticated caller context is absent"))
}

fn now_timestamp() -> Result<Timestamp, Status> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Status::internal("server time is unavailable"))?;
    Ok(Timestamp {
        seconds: i64::try_from(duration.as_secs())
            .map_err(|_| Status::internal("server time is out of range"))?,
        nanos: i32::try_from(duration.subsec_nanos())
            .map_err(|_| Status::internal("server time is out of range"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_api::{
        ApiScope, ApplicationKind, ApplicationPrincipal, RequestId, scopes::RUNS_READ,
    };

    fn service() -> SystemServiceAdapter {
        SystemServiceAdapter::new(
            SystemMetadata {
                instance_id: "019f7d38-649a-7580-a30f-01157b719c2a".into(),
                server_version: "0.9.0".into(),
                deployment_mode: DeploymentMode::SharedDaemon,
            },
            Arc::new(FixedReadiness::ready()),
        )
    }

    fn request<T>(body: T) -> Request<T> {
        request_with(body, [RUNS_READ], Vec::new())
    }

    fn request_with<T>(
        body: T,
        scopes: impl IntoIterator<Item = &'static str>,
        tools: Vec<String>,
    ) -> Request<T> {
        let principal = ApplicationPrincipal::authenticated(
            "app:test-ui",
            "credential-1",
            ApplicationKind::Enrolled,
            scopes
                .into_iter()
                .map(|scope| ApiScope::new(scope).expect("scope")),
            ["primary".into()],
            tools,
        )
        .expect("principal");
        let mut request = Request::new(body);
        request
            .extensions_mut()
            .insert(CallerContext::authenticated(
                principal,
                RequestId::new("request-1").expect("request id"),
            ));
        request
    }

    #[tokio::test]
    async fn system_metadata_is_scope_filtered() {
        let response = service()
            .get_server_info(request(GetServerInfoRequest {}))
            .await
            .expect("server info")
            .into_inner()
            .server_info
            .expect("server info");
        assert_eq!(response.api_packages, [API_PACKAGE]);
        assert!(
            response
                .capabilities
                .iter()
                .any(|capability| capability.name == "agent_runs.read" && capability.enabled)
        );
        assert!(
            response
                .capabilities
                .iter()
                .any(|capability| capability.name == "plans.continue" && !capability.enabled)
        );
        assert!(
            response
                .capabilities
                .iter()
                .any(|capability| capability.name == "agent_runs.create" && !capability.enabled)
        );
        assert!(response.limits.iter().any(|limit| {
            limit.name == "run.active_global" && limit.value == 32 && limit.unit == "runs"
        }));
        assert!(response.limits.iter().any(|limit| {
            limit.name == "run.active_per_application" && limit.value == 8 && limit.unit == "runs"
        }));
        assert!(response.limits.iter().any(|limit| {
            limit.name == "list.owner_index_events_scanned"
                && limit.value == 64
                && limit.unit == "events"
        }));
        assert!(
            !response
                .limits
                .iter()
                .any(|limit| limit.name == "list.scan_global_events")
        );
        assert!(response.limits.iter().any(|limit| {
            limit.name == "transport.concurrent_request_setups"
                && limit.value == 80
                && limit.unit == "requests"
        }));
        assert!(response.limits.iter().any(|limit| {
            limit.name == "transport.active_watch_streams"
                && limit.value == 64
                && limit.unit == "streams"
        }));
        assert!(response.limits.iter().any(|limit| {
            limit.name == "transport.reserved_unary_request_headroom"
                && limit.value == 16
                && limit.unit == "requests"
        }));
    }

    #[tokio::test]
    async fn optional_capabilities_follow_authenticated_scopes_and_tool_ceiling() {
        let response = service()
            .get_server_info(request_with(
                GetServerInfoRequest {},
                [
                    scopes::RUNS_EXECUTE,
                    scopes::RUNS_READ,
                    scopes::ARTIFACTS_READ,
                    scopes::ARTIFACTS_WRITE,
                ],
                vec!["agent.delegate".into()],
            ))
            .await
            .expect("server info")
            .into_inner()
            .server_info
            .expect("server info");
        let enabled = response
            .capabilities
            .into_iter()
            .filter(|capability| capability.enabled)
            .map(|capability| capability.name)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(enabled.contains("agent_runs.delegation"));
        assert!(enabled.contains("plans.continue"));
        assert!(enabled.contains("artifacts.read"));
        assert!(enabled.contains("artifacts.upload"));
        assert!(enabled.contains("attachments.run_input"));

        let without_run_scope = service()
            .get_server_info(request_with(
                GetServerInfoRequest {},
                [scopes::ARTIFACTS_READ],
                vec!["agent.delegate".into()],
            ))
            .await
            .expect("server info without run scope")
            .into_inner()
            .server_info
            .expect("server info");
        let disabled = without_run_scope
            .capabilities
            .into_iter()
            .filter(|capability| !capability.enabled)
            .map(|capability| capability.name)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(disabled.contains("agent_runs.delegation"));
        assert!(disabled.contains(PLAN_CONTINUATION_CAPABILITY));
    }

    #[tokio::test]
    async fn direct_call_without_transport_identity_fails_closed() {
        let error = service()
            .get_readiness(Request::new(GetReadinessRequest {}))
            .await
            .expect_err("missing caller must fail");
        assert_eq!(error.code(), tonic::Code::Unauthenticated);
    }
}
