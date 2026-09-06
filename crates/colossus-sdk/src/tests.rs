use super::*;
use async_trait::async_trait;
use futures::stream;
use std::{
    collections::VecDeque,
    num::NonZeroU32,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
#[cfg(feature = "daemon")]
use url::Url;
use uuid::Uuid;

struct TestCredential;

#[async_trait]
impl CredentialProvider for TestCredential {
    async fn load(&self) -> SdkResult<Secret> {
        Secret::new(b"credential-value-never-log".to_vec())
    }
}

struct UnusedAgentRuns;

#[async_trait]
impl AgentRunClient for UnusedAgentRuns {
    async fn create_run(&self, _request: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        unreachable!("run service is not exercised by lifecycle tests")
    }

    async fn get_run(&self, _request: GetRunRequest) -> ApiResult<GetRunResponse> {
        unreachable!("run service is not exercised by lifecycle tests")
    }

    async fn list_runs(&self, _request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        unreachable!("run service is not exercised by lifecycle tests")
    }

    async fn watch_run(&self, _request: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        unreachable!("run service is not exercised by lifecycle tests")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> ApiResult<CancelRunResponse> {
        unreachable!("run service is not exercised by lifecycle tests")
    }

    async fn respond_interaction(
        &self,
        _request: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        unreachable!("run service is not exercised by lifecycle tests")
    }
}

struct TestBackend {
    kind: BackendKind,
    closed: AtomicBool,
}

impl TestBackend {
    fn new(kind: BackendKind) -> Self {
        Self {
            kind,
            closed: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl Backend for TestBackend {
    fn kind(&self) -> BackendKind {
        self.kind
    }

    fn agent_runs(&self) -> Arc<dyn AgentRunClient> {
        Arc::new(UnusedAgentRuns)
    }

    async fn close(&self) -> SdkResult<()> {
        self.closed.store(true, Ordering::Release);
        Ok(())
    }
}

#[test]
fn secret_debug_output_is_redacted() {
    let secret = Secret::new(b"credential-value-never-log".to_vec()).expect("valid secret");
    let debug = format!("{secret:?}");
    assert_eq!(debug, "Secret([REDACTED])");
    assert!(!debug.contains("credential-value-never-log"));
}

#[tokio::test]
async fn plan_continuation_requires_an_advertised_runtime_capability() {
    let client = Colossus::from_backend(TestBackend::new(BackendKind::Daemon));
    let error = client
        .create_run(CreateRunRequest {
            plugin_skill_ids: Vec::new(),
            input: vec![InputContentPart::Text("Run the approved Plan".into())],
            session_id: Some("session-1".into()),
            end_user_id: None,
            role: "primary".into(),
            mode: RunMode::Execute,
            research_depth: None,
            research_sources: Vec::new(),
            plan_action: Some(PlanRunAction::Execute {
                source_run_id: "run-plan-source".into(),
                expected_revision: 3,
                strategy: PlanExecutionStrategy::Direct,
            }),
            branch: None,
            max_turns: 10,
            idempotency_key: IdempotencyKey::new("plan-continuation-capability")
                .expect("idempotency key"),
        })
        .await
        .expect_err("older runtimes must not receive unknown Plan actions");

    assert_eq!(error.code, ApiErrorCode::FailedPrecondition);
    assert_eq!(error.reason, ApiErrorReason::InvalidRunTransition);
}

#[test]
fn empty_secret_is_rejected() {
    assert!(matches!(
        Secret::new(Vec::new()),
        Err(SdkError::InvalidConfiguration(_))
    ));
    assert!(matches!(
        Secret::new(vec![b'x'; 762]),
        Err(SdkError::InvalidConfiguration(_))
    ));
    assert!(matches!(
        Secret::new(b"token with whitespace".to_vec()),
        Err(SdkError::InvalidConfiguration(_))
    ));
    assert!(matches!(
        Secret::new(vec![0xff]),
        Err(SdkError::InvalidConfiguration(_))
    ));
}

#[test]
fn executable_and_private_directory_require_absolute_non_root_paths() {
    let digest = Sha256Digest::from_bytes([7; 32]);
    assert!(matches!(
        VerifiedExecutable::new("colossus", digest),
        Err(SdkError::PathNotAbsolute(_))
    ));
    assert!(matches!(
        AppPrivateInstanceDir::new("state"),
        Err(SdkError::PathNotAbsolute(_))
    ));
    assert!(matches!(
        AppPrivateInstanceDir::new(std::env::temp_dir().join("../other-state")),
        Err(SdkError::InvalidConfiguration(_))
    ));

    #[cfg(unix)]
    {
        assert!(matches!(
            AppPrivateInstanceDir::new("/"),
            Err(SdkError::InvalidConfiguration(_))
        ));
    }
}

#[test]
fn trusted_identity_inputs_are_canonical_and_non_nil() {
    assert!(TlsFingerprint::from_hex(&"a".repeat(64)).is_ok());
    for invalid in ["A".repeat(64), "a".repeat(63), "g".repeat(64)] {
        assert!(matches!(
            TlsFingerprint::from_hex(&invalid),
            Err(SdkError::InvalidConfiguration(_))
        ));
    }

    #[cfg(feature = "daemon")]
    assert!(matches!(
        DaemonConnectOptions::new(
            InstanceId::from_uuid(Uuid::nil()),
            std::env::temp_dir().join("colossus-nil-instance.descriptor"),
            TlsFingerprint::from_bytes([1; 32]),
            ApiMajor::new(1).expect("major"),
            Arc::new(TestCredential),
        ),
        Err(SdkError::InvalidConfiguration(_))
    ));

    #[cfg(feature = "sidecar")]
    assert!(matches!(
        SidecarOptions::new(
            InstanceId::from_uuid(Uuid::nil()),
            AppPrivateInstanceDir::new(std::env::temp_dir().join("colossus-nil-sidecar-state"))
                .expect("absolute state"),
            VerifiedExecutable::new(
                std::env::temp_dir().join("colossus-nil-sidecar-bin"),
                Sha256Digest::from_bytes([4; 32]),
            )
            .expect("absolute executable"),
            ApiMajor::new(1).expect("major"),
        ),
        Err(SdkError::InvalidConfiguration(_))
    ));

    #[cfg(feature = "embedded")]
    assert!(matches!(
        EmbeddedOptions::new(
            InstanceId::from_uuid(Uuid::nil()),
            AppPrivateInstanceDir::new(std::env::temp_dir().join("colossus-nil-embedded-state"))
                .expect("absolute state"),
            "app:example/ui",
        ),
        Err(SdkError::InvalidConfiguration(_))
    ));
}

#[cfg(feature = "daemon")]
fn test_instance_id() -> InstanceId {
    InstanceId::from_uuid(Uuid::from_u128(0x018f_1068_d264_7d6c_8f52_0123_4567_89ab))
}

#[cfg(feature = "daemon")]
fn test_descriptor(endpoint: &str) -> DaemonDescriptor {
    DaemonDescriptor::new(
        test_instance_id(),
        NonZeroU32::new(42).expect("non-zero"),
        Url::parse(endpoint).expect("URL"),
        ApiMajor::new(1).expect("major"),
        TlsFingerprint::from_bytes([9; 32]),
    )
    .expect("valid descriptor")
}

#[cfg(feature = "daemon")]
fn test_connect_options() -> DaemonConnectOptions {
    DaemonConnectOptions::new(
        test_instance_id(),
        std::env::temp_dir().join("colossus-test.descriptor"),
        TlsFingerprint::from_bytes([9; 32]),
        ApiMajor::new(1).expect("major"),
        Arc::new(TestCredential),
    )
    .expect("valid connect options")
}

#[cfg(feature = "daemon")]
fn test_launch_options() -> DaemonLaunchOptions {
    DaemonLaunchOptions::new(
        test_connect_options(),
        VerifiedExecutable::new(
            std::env::temp_dir().join("colossus-test-bin"),
            Sha256Digest::from_bytes([7; 32]),
        )
        .expect("absolute executable"),
        AppPrivateInstanceDir::new(std::env::temp_dir().join("colossus-test-state"))
            .expect("absolute instance directory"),
    )
}

#[cfg(feature = "daemon")]
#[test]
fn daemon_descriptor_accepts_only_bare_ip_literal_https_loopback() {
    assert_eq!(
        test_descriptor("https://127.0.0.1:43123/")
            .endpoint()
            .host_str(),
        Some("127.0.0.1")
    );
    assert!(
        DaemonDescriptor::new(
            test_instance_id(),
            NonZeroU32::new(1).expect("non-zero"),
            Url::parse("https://[::1]:43123/").expect("URL"),
            ApiMajor::new(1).expect("major"),
            TlsFingerprint::from_bytes([1; 32]),
        )
        .is_ok()
    );

    for endpoint in [
        "http://127.0.0.1:43123/",
        "https://localhost:43123/",
        "https://127.0.0.2:43123/",
        "https://192.0.2.10:43123/",
        "https://user@127.0.0.1:43123/",
        "https://127.0.0.1:43123/rpc",
        "https://127.0.0.1/",
    ] {
        let result = DaemonDescriptor::new(
            test_instance_id(),
            NonZeroU32::new(1).expect("non-zero"),
            Url::parse(endpoint).expect("URL"),
            ApiMajor::new(1).expect("major"),
            TlsFingerprint::from_bytes([1; 32]),
        );
        assert!(result.is_err(), "unexpected accepted endpoint: {endpoint}");
    }
}

#[cfg(feature = "daemon")]
#[test]
fn daemon_options_debug_never_loads_or_prints_credentials() {
    let debug = format!("{:?}", test_connect_options());
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("credential-value-never-log"));
}

#[cfg(feature = "daemon")]
struct TestDaemonLifecycle {
    discoveries: Mutex<VecDeque<DaemonDiscovery>>,
    ready: DaemonDescriptor,
    connects: AtomicUsize,
    locks: AtomicUsize,
    launches: AtomicUsize,
    fail_authentication: bool,
}

#[cfg(feature = "daemon")]
impl TestDaemonLifecycle {
    fn new(discoveries: impl IntoIterator<Item = DaemonDiscovery>) -> Self {
        Self {
            discoveries: Mutex::new(discoveries.into_iter().collect()),
            ready: test_descriptor("https://127.0.0.1:43123/"),
            connects: AtomicUsize::new(0),
            locks: AtomicUsize::new(0),
            launches: AtomicUsize::new(0),
            fail_authentication: false,
        }
    }

    fn authentication_failure(discoveries: impl IntoIterator<Item = DaemonDiscovery>) -> Self {
        Self {
            fail_authentication: true,
            ..Self::new(discoveries)
        }
    }
}

#[cfg(feature = "daemon")]
#[async_trait]
impl DaemonLifecycle for TestDaemonLifecycle {
    async fn discover(&self, _options: &DaemonConnectOptions) -> SdkResult<DaemonDiscovery> {
        self.discoveries
            .lock()
            .expect("discovery lock")
            .pop_front()
            .ok_or(SdkError::Unavailable)
    }

    async fn connect_verified(
        &self,
        _options: &DaemonConnectOptions,
        _descriptor: &DaemonDescriptor,
    ) -> SdkResult<Arc<dyn Backend>> {
        self.connects.fetch_add(1, Ordering::AcqRel);
        if self.fail_authentication {
            Err(SdkError::Authentication)
        } else {
            Ok(Arc::new(TestBackend::new(BackendKind::Daemon)))
        }
    }

    async fn acquire_launch_guard(
        &self,
        _options: &DaemonLaunchOptions,
    ) -> SdkResult<DaemonLaunchGuard> {
        self.locks.fetch_add(1, Ordering::AcqRel);
        Ok(DaemonLaunchGuard::new(()))
    }

    async fn launch_verified(
        &self,
        _options: &DaemonLaunchOptions,
        _guard: &DaemonLaunchGuard,
    ) -> SdkResult<()> {
        self.launches.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    async fn wait_until_ready(
        &self,
        _options: &DaemonConnectOptions,
        _guard: &DaemonLaunchGuard,
    ) -> SdkResult<DaemonDescriptor> {
        Ok(self.ready.clone())
    }
}

#[cfg(feature = "daemon")]
#[tokio::test]
async fn connect_or_start_connects_without_launch_when_present() {
    let lifecycle = TestDaemonLifecycle::new([DaemonDiscovery::Present(test_descriptor(
        "https://127.0.0.1:43123/",
    ))]);
    let client = Colossus::connect_or_start(&lifecycle, test_launch_options())
        .await
        .expect("connect");

    assert_eq!(client.backend_kind(), BackendKind::Daemon);
    assert_eq!(lifecycle.connects.load(Ordering::Acquire), 1);
    assert_eq!(lifecycle.locks.load(Ordering::Acquire), 0);
    assert_eq!(lifecycle.launches.load(Ordering::Acquire), 0);
}

#[cfg(feature = "daemon")]
#[tokio::test]
async fn connect_or_start_rechecks_after_launch_lock() {
    let lifecycle = TestDaemonLifecycle::new([
        DaemonDiscovery::Absent,
        DaemonDiscovery::Present(test_descriptor("https://127.0.0.1:43123/")),
    ]);
    Colossus::connect_or_start(&lifecycle, test_launch_options())
        .await
        .expect("connect after competing launch");

    assert_eq!(lifecycle.locks.load(Ordering::Acquire), 1);
    assert_eq!(lifecycle.launches.load(Ordering::Acquire), 0);
    assert_eq!(lifecycle.connects.load(Ordering::Acquire), 1);
}

#[cfg(feature = "daemon")]
#[tokio::test]
async fn connect_or_start_launches_only_after_two_verified_absences() {
    let lifecycle = TestDaemonLifecycle::new([DaemonDiscovery::Absent, DaemonDiscovery::Absent]);
    Colossus::connect_or_start(&lifecycle, test_launch_options())
        .await
        .expect("launch and connect");

    assert_eq!(lifecycle.locks.load(Ordering::Acquire), 1);
    assert_eq!(lifecycle.launches.load(Ordering::Acquire), 1);
    assert_eq!(lifecycle.connects.load(Ordering::Acquire), 1);
}

#[cfg(feature = "daemon")]
#[tokio::test]
async fn authentication_failure_never_falls_back_to_launch() {
    let lifecycle = TestDaemonLifecycle::authentication_failure([DaemonDiscovery::Present(
        test_descriptor("https://127.0.0.1:43123/"),
    )]);
    let error = Colossus::connect_or_start(&lifecycle, test_launch_options())
        .await
        .expect_err("authentication must fail closed");

    assert!(matches!(error, SdkError::Authentication));
    assert_eq!(lifecycle.locks.load(Ordering::Acquire), 0);
    assert_eq!(lifecycle.launches.load(Ordering::Acquire), 0);
}

#[cfg(feature = "daemon")]
#[tokio::test]
async fn mismatched_instance_is_rejected_before_transport_authentication() {
    let mismatched = DaemonDescriptor::new(
        InstanceId::from_uuid(Uuid::from_u128(0x018f_1068_d264_7d6c_8f52_ffff_ffff_ffff)),
        NonZeroU32::new(1).expect("non-zero"),
        Url::parse("https://127.0.0.1:43123/").expect("URL"),
        ApiMajor::new(1).expect("major"),
        TlsFingerprint::from_bytes([1; 32]),
    )
    .expect("descriptor shape");
    let lifecycle = TestDaemonLifecycle::new([DaemonDiscovery::Present(mismatched)]);

    let error = Colossus::connect(&lifecycle, test_connect_options())
        .await
        .expect_err("identity mismatch");
    assert!(matches!(error, SdkError::IdentityMismatch));
    assert_eq!(lifecycle.connects.load(Ordering::Acquire), 0);
}

#[cfg(feature = "daemon")]
#[tokio::test]
async fn descriptor_pin_must_match_independently_provisioned_expected_pin() {
    let mismatched = DaemonDescriptor::new(
        test_instance_id(),
        NonZeroU32::new(1).expect("non-zero"),
        Url::parse("https://127.0.0.1:43123/").expect("URL"),
        ApiMajor::new(1).expect("major"),
        TlsFingerprint::from_bytes([8; 32]),
    )
    .expect("descriptor shape");
    let lifecycle = TestDaemonLifecycle::new([DaemonDiscovery::Present(mismatched)]);

    let error = Colossus::connect(&lifecycle, test_connect_options())
        .await
        .expect_err("descriptor cannot establish its own TLS trust");
    assert!(matches!(error, SdkError::IdentityMismatch));
    assert_eq!(lifecycle.connects.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn owned_run_update_stream_preserves_order() {
    let stream: RunUpdateStream = Box::pin(stream::iter([
        Ok(RunUpdate {
            run_id: "run-1".into(),
            sequence: 1,
            created_at: "2026-07-19T00:00:00Z".into(),
            update: RunUpdateKind::State(RunStatus::Running),
        }),
        Ok(RunUpdate {
            run_id: "run-1".into(),
            sequence: 2,
            created_at: "2026-07-19T00:00:01Z".into(),
            update: RunUpdateKind::Cancellation(RunCancellation {
                turn: 0,
                message: "cancelled".into(),
                plan_id: None,
                plan_revision: None,
                plan_status: None,
                goal_id: None,
            }),
        }),
    ]));
    let mut updates = RunUpdates::checked(
        Arc::new(UnusedAgentRuns),
        stream,
        WatchRunRequest {
            run_id: "run-1".into(),
            after_sequence: 0,
        },
    );

    assert_eq!(
        updates
            .next_update()
            .await
            .expect("first")
            .expect("valid")
            .sequence,
        1
    );
    assert_eq!(
        updates
            .next_update()
            .await
            .expect("second")
            .expect("valid")
            .sequence,
        2
    );
    assert!(updates.next_update().await.is_none());
}

struct CheckedSnapshotClient {
    run_id: String,
    last_sequence: u64,
    terminal: bool,
    gets: AtomicUsize,
}

#[async_trait]
impl AgentRunClient for CheckedSnapshotClient {
    async fn create_run(&self, _request: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        unreachable!("only GetRun is exercised")
    }

    async fn get_run(&self, _request: GetRunRequest) -> ApiResult<GetRunResponse> {
        self.gets.fetch_add(1, Ordering::AcqRel);
        Ok(GetRunResponse {
            run: Run {
                plugin_skill_ids: Vec::new(),
                run_id: self.run_id.clone(),
                session_id: "session-1".into(),
                title: "Checked snapshot".into(),
                role: "primary".into(),
                mode: RunMode::Execute,
                status: if self.terminal {
                    RunStatus::Cancelled
                } else {
                    RunStatus::Running
                },
                created_at: "2026-07-19T00:00:00Z".into(),
                updated_at: "2026-07-19T00:00:01Z".into(),
                started_at: Some("2026-07-19T00:00:00Z".into()),
                finished_at: self.terminal.then(|| "2026-07-19T00:00:01Z".to_string()),
                last_sequence: self.last_sequence,
                pending_interaction_count: 0,
                terminal: self.terminal.then(|| {
                    RunTerminal::Cancellation(RunCancellation {
                        turn: 0,
                        message: "cancelled".into(),
                        plan_id: None,
                        plan_revision: None,
                        plan_status: None,
                        goal_id: None,
                    })
                }),
                etag: "snapshot-etag".into(),
                archived: false,
            },
            pending_interactions: Vec::new(),
        })
    }

    async fn list_runs(&self, _request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        unreachable!("only GetRun is exercised")
    }

    async fn watch_run(&self, _request: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        unreachable!("only GetRun is exercised")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> ApiResult<CancelRunResponse> {
        unreachable!("only GetRun is exercised")
    }

    async fn respond_interaction(
        &self,
        _request: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        unreachable!("only GetRun is exercised")
    }
}

#[tokio::test]
async fn checked_run_update_stream_reconciles_exact_terminal_cursor() {
    let stream: RunUpdateStream = Box::pin(stream::iter([Ok(run_update(
        "run-1",
        1,
        RunUpdateKind::State(RunStatus::Running),
    ))]));
    let client = Arc::new(CheckedSnapshotClient {
        run_id: "run-1".into(),
        last_sequence: 1,
        terminal: true,
        gets: AtomicUsize::new(0),
    });
    let mut updates = RunUpdates::checked(
        client.clone(),
        stream,
        WatchRunRequest {
            run_id: "run-1".into(),
            after_sequence: 0,
        },
    );

    assert_eq!(
        updates
            .next_update()
            .await
            .expect("first")
            .expect("valid")
            .sequence,
        1
    );
    assert!(updates.next_update().await.is_none());
    assert_eq!(client.gets.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn checked_run_update_stream_fails_closed_on_nonterminal_summary() {
    let stream: RunUpdateStream = Box::pin(stream::iter([Ok(run_update(
        "run-1",
        1,
        RunUpdateKind::State(RunStatus::Running),
    ))]));
    let client = Arc::new(CheckedSnapshotClient {
        run_id: "run-1".into(),
        last_sequence: 1,
        terminal: false,
        gets: AtomicUsize::new(0),
    });
    let mut updates = RunUpdates::checked(
        client.clone(),
        stream,
        WatchRunRequest {
            run_id: "run-1".into(),
            after_sequence: 0,
        },
    );

    assert_eq!(
        updates
            .next_update()
            .await
            .expect("first")
            .expect("valid")
            .sequence,
        1
    );
    let error = updates
        .next_update()
        .await
        .expect("clean EOF protocol error")
        .expect_err("a non-terminal summary must fail closed");
    assert_eq!(error.code, ApiErrorCode::Internal);
    assert_eq!(error.reason, ApiErrorReason::InternalInvariant);
    assert!(!error.retryable);
    assert_eq!(
        error.message,
        "the run watch closed without an exact terminal run summary"
    );
    assert!(updates.next_update().await.is_none());
    assert_eq!(client.gets.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn checked_run_update_stream_fails_closed_on_cursor_mismatch() {
    let stream: RunUpdateStream = Box::pin(stream::iter([Ok(run_update(
        "run-1",
        1,
        RunUpdateKind::State(RunStatus::Running),
    ))]));
    let client = Arc::new(CheckedSnapshotClient {
        run_id: "run-1".into(),
        last_sequence: 2,
        terminal: true,
        gets: AtomicUsize::new(0),
    });
    let mut updates = RunUpdates::checked(
        client.clone(),
        stream,
        WatchRunRequest {
            run_id: "run-1".into(),
            after_sequence: 0,
        },
    );

    assert_eq!(
        updates
            .next_update()
            .await
            .expect("first")
            .expect("valid")
            .sequence,
        1
    );
    let error = updates
        .next_update()
        .await
        .expect("clean EOF protocol error")
        .expect_err("a mismatched terminal cursor must fail closed");
    assert_eq!(error.code, ApiErrorCode::Internal);
    assert_eq!(error.reason, ApiErrorReason::InternalInvariant);
    assert!(!error.retryable);
    assert_eq!(
        error.message,
        "the run summary did not match the verified watch cursor"
    );
    assert!(updates.next_update().await.is_none());
    assert_eq!(client.gets.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn checked_run_update_stream_fails_closed_on_run_mismatch() {
    let stream: RunUpdateStream = Box::pin(stream::empty());
    let client = Arc::new(CheckedSnapshotClient {
        run_id: "run-other".into(),
        last_sequence: 7,
        terminal: true,
        gets: AtomicUsize::new(0),
    });
    let mut updates = RunUpdates::checked(
        client.clone(),
        stream,
        WatchRunRequest {
            run_id: "run-1".into(),
            after_sequence: 7,
        },
    );

    let error = updates
        .next_update()
        .await
        .expect("clean EOF protocol error")
        .expect_err("a different run summary must fail closed");
    assert_eq!(error.code, ApiErrorCode::Internal);
    assert_eq!(error.reason, ApiErrorReason::InternalInvariant);
    assert!(!error.retryable);
    assert_eq!(
        error.message,
        "the run summary returned a different run during watch reconciliation"
    );
    assert!(updates.next_update().await.is_none());
    assert_eq!(client.gets.load(Ordering::Acquire), 1);
}

fn run_update(run_id: &str, sequence: u64, update: RunUpdateKind) -> RunUpdate {
    RunUpdate {
        run_id: run_id.into(),
        sequence,
        created_at: format!("2026-07-19T00:00:{sequence:02}Z"),
        update,
    }
}

fn unavailable_watch_error() -> ApiError {
    ApiError {
        code: ApiErrorCode::Unavailable,
        reason: ApiErrorReason::InternalInvariant,
        message: "the Colossus API is unavailable".into(),
        correlation_id: None,
        retryable: true,
        outcome: colossus_api::OutcomeCertainty::Known,
        violations: Vec::new(),
    }
}

#[tokio::test]
async fn checked_run_update_stream_drops_duplicates_and_fails_closed_on_gaps() {
    let stream: RunUpdateStream = Box::pin(stream::iter([
        Ok(run_update(
            "run-1",
            1,
            RunUpdateKind::State(RunStatus::Running),
        )),
        Ok(run_update(
            "run-1",
            1,
            RunUpdateKind::State(RunStatus::Running),
        )),
        Ok(run_update(
            "run-1",
            3,
            RunUpdateKind::State(RunStatus::Completed),
        )),
    ]));
    let mut updates = RunUpdates::checked(
        Arc::new(UnusedAgentRuns),
        stream,
        WatchRunRequest {
            run_id: "run-1".into(),
            after_sequence: 0,
        },
    );

    assert_eq!(
        updates
            .next_update()
            .await
            .expect("first")
            .expect("valid")
            .sequence,
        1
    );
    let gap = updates
        .next_update()
        .await
        .expect("gap")
        .expect_err("gap must fail closed");
    assert_eq!(gap.code, ApiErrorCode::Internal);
    assert_eq!(gap.reason, ApiErrorReason::InternalInvariant);
    assert!(updates.next_update().await.is_none());
}

#[cfg(feature = "daemon")]
struct ScriptedWatchClient {
    streams: Mutex<VecDeque<ApiResult<RunUpdateStream>>>,
    cursors: Mutex<Vec<u64>>,
}

#[cfg(feature = "daemon")]
#[async_trait]
impl AgentRunClient for ScriptedWatchClient {
    async fn create_run(&self, _request: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        unreachable!("only watch is exercised")
    }

    async fn get_run(&self, _request: GetRunRequest) -> ApiResult<GetRunResponse> {
        Err(unavailable_watch_error())
    }

    async fn list_runs(&self, _request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        unreachable!("only watch is exercised")
    }

    async fn watch_run(&self, request: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        self.cursors
            .lock()
            .expect("cursor lock")
            .push(request.after_sequence);
        self.streams
            .lock()
            .expect("stream lock")
            .pop_front()
            .unwrap_or_else(|| Err(unavailable_watch_error()))
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> ApiResult<CancelRunResponse> {
        unreachable!("only watch is exercised")
    }

    async fn respond_interaction(
        &self,
        _request: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        unreachable!("only watch is exercised")
    }
}

#[cfg(feature = "daemon")]
struct TerminalSnapshotWatchClient {
    watches: AtomicUsize,
    gets: AtomicUsize,
    run_id: String,
    last_sequence: u64,
}

#[cfg(feature = "daemon")]
#[async_trait]
impl AgentRunClient for TerminalSnapshotWatchClient {
    async fn create_run(&self, _request: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        unreachable!("only watch is exercised")
    }

    async fn get_run(&self, _request: GetRunRequest) -> ApiResult<GetRunResponse> {
        self.gets.fetch_add(1, Ordering::AcqRel);
        Ok(GetRunResponse {
            run: Run {
                plugin_skill_ids: Vec::new(),
                run_id: self.run_id.clone(),
                session_id: "session-1".into(),
                title: "Terminal snapshot".into(),
                role: "primary".into(),
                mode: RunMode::Execute,
                status: RunStatus::Cancelled,
                created_at: "2026-07-19T00:00:00Z".into(),
                updated_at: "2026-07-19T00:00:01Z".into(),
                started_at: None,
                finished_at: Some("2026-07-19T00:00:01Z".into()),
                last_sequence: self.last_sequence,
                pending_interaction_count: 0,
                terminal: Some(RunTerminal::Cancellation(RunCancellation {
                    turn: 0,
                    message: "cancelled".into(),
                    plan_id: None,
                    plan_revision: None,
                    plan_status: None,
                    goal_id: None,
                })),
                etag: "terminal-etag".into(),
                archived: false,
            },
            pending_interactions: Vec::new(),
        })
    }

    async fn list_runs(&self, _request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        unreachable!("only watch is exercised")
    }

    async fn watch_run(&self, _request: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        self.watches.fetch_add(1, Ordering::AcqRel);
        Ok(Box::pin(stream::empty()))
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> ApiResult<CancelRunResponse> {
        unreachable!("only watch is exercised")
    }

    async fn respond_interaction(
        &self,
        _request: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        unreachable!("only watch is exercised")
    }
}

#[cfg(feature = "daemon")]
#[tokio::test]
async fn clean_watch_close_at_terminal_cursor_completes_without_reconnect() {
    let client = Arc::new(TerminalSnapshotWatchClient {
        watches: AtomicUsize::new(0),
        gets: AtomicUsize::new(0),
        run_id: "run-terminal".into(),
        last_sequence: 7,
    });
    let mut updates = RunUpdates::resilient(
        client.clone(),
        WatchRunRequest {
            run_id: "run-terminal".into(),
            after_sequence: 7,
        },
        None,
    );

    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), updates.next_update())
            .await
            .expect("terminal state resolves promptly")
            .is_none()
    );
    assert_eq!(client.watches.load(Ordering::Acquire), 1);
    assert_eq!(client.gets.load(Ordering::Acquire), 1);
}

#[cfg(feature = "daemon")]
#[tokio::test]
async fn resilient_watch_fails_closed_on_reconciled_run_mismatch() {
    let client = Arc::new(TerminalSnapshotWatchClient {
        watches: AtomicUsize::new(0),
        gets: AtomicUsize::new(0),
        run_id: "run-other".into(),
        last_sequence: 7,
    });
    let mut updates = RunUpdates::resilient(
        client.clone(),
        WatchRunRequest {
            run_id: "run-terminal".into(),
            after_sequence: 7,
        },
        None,
    );

    let error = tokio::time::timeout(std::time::Duration::from_millis(100), updates.next_update())
        .await
        .expect("run mismatch resolves promptly")
        .expect("protocol error")
        .expect_err("another run must fail closed");
    assert_eq!(error.code, ApiErrorCode::Internal);
    assert_eq!(error.reason, ApiErrorReason::InternalInvariant);
    assert_eq!(
        error.message,
        "the run summary returned a different run during watch reconciliation"
    );
    assert!(updates.next_update().await.is_none());
    assert_eq!(client.watches.load(Ordering::Acquire), 1);
    assert_eq!(client.gets.load(Ordering::Acquire), 1);
}

#[cfg(feature = "daemon")]
#[tokio::test]
async fn resilient_run_updates_reconnect_from_cursor_and_deduplicate_replay() {
    let resumed: RunUpdateStream = Box::pin(stream::iter([
        Ok(run_update(
            "run-1",
            1,
            RunUpdateKind::State(RunStatus::Running),
        )),
        Ok(run_update(
            "run-1",
            2,
            RunUpdateKind::Cancellation(RunCancellation {
                turn: 1,
                message: "cancelled".into(),
                plan_id: None,
                plan_revision: None,
                plan_status: None,
                goal_id: None,
            }),
        )),
    ]));
    let client = Arc::new(ScriptedWatchClient {
        streams: Mutex::new(VecDeque::from([Ok(resumed)])),
        cursors: Mutex::new(Vec::new()),
    });
    let initial: RunUpdateStream = Box::pin(stream::iter([Ok(run_update(
        "run-1",
        1,
        RunUpdateKind::State(RunStatus::Running),
    ))]));
    let mut updates = RunUpdates::resilient(
        client.clone(),
        WatchRunRequest {
            run_id: "run-1".into(),
            after_sequence: 0,
        },
        Some(initial),
    );

    assert_eq!(
        updates
            .next_update()
            .await
            .expect("first")
            .expect("valid")
            .sequence,
        1
    );
    assert_eq!(
        updates
            .next_update()
            .await
            .expect("terminal")
            .expect("valid")
            .sequence,
        2
    );
    assert!(updates.next_update().await.is_none());
    assert_eq!(*client.cursors.lock().expect("cursor lock"), [1]);
}

#[cfg(feature = "daemon")]
#[tokio::test]
async fn resilient_run_updates_retries_only_unavailable_open_failures() {
    let terminal: RunUpdateStream = Box::pin(stream::iter([Ok(run_update(
        "run-1",
        1,
        RunUpdateKind::Cancellation(RunCancellation {
            turn: 0,
            message: "cancelled".into(),
            plan_id: None,
            plan_revision: None,
            plan_status: None,
            goal_id: None,
        }),
    ))]));
    let client = Arc::new(ScriptedWatchClient {
        streams: Mutex::new(VecDeque::from([
            Err(unavailable_watch_error()),
            Ok(terminal),
        ])),
        cursors: Mutex::new(Vec::new()),
    });
    let mut updates = RunUpdates::resilient(
        client.clone(),
        WatchRunRequest {
            run_id: "run-1".into(),
            after_sequence: 0,
        },
        None,
    );

    assert_eq!(
        updates
            .next_update()
            .await
            .expect("terminal")
            .expect("valid")
            .sequence,
        1
    );
    assert!(updates.next_update().await.is_none());
    assert_eq!(*client.cursors.lock().expect("cursor lock"), [0, 0]);
}

#[cfg(feature = "daemon")]
#[tokio::test]
async fn resilient_run_updates_do_not_retry_nonretryable_unavailable() {
    let mut nonretryable = unavailable_watch_error();
    nonretryable.retryable = false;
    let client = Arc::new(ScriptedWatchClient {
        streams: Mutex::new(VecDeque::from([Err(nonretryable)])),
        cursors: Mutex::new(Vec::new()),
    });
    let mut updates = RunUpdates::resilient(
        client.clone(),
        WatchRunRequest {
            run_id: "run-1".into(),
            after_sequence: 0,
        },
        None,
    );

    let error = updates
        .next_update()
        .await
        .expect("error")
        .expect_err("nonretryable unavailable must terminate");
    assert_eq!(error.code, ApiErrorCode::Unavailable);
    assert!(!error.retryable);
    assert!(updates.next_update().await.is_none());
    assert_eq!(*client.cursors.lock().expect("cursor lock"), [0]);
}

#[cfg(feature = "daemon")]
struct CloseAwareWatchClient {
    closed: tokio::sync::watch::Sender<bool>,
    attempts: AtomicUsize,
    attempted: tokio::sync::Notify,
}

#[cfg(feature = "daemon")]
#[async_trait]
impl AgentRunClient for CloseAwareWatchClient {
    async fn create_run(&self, _request: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        unreachable!("only watch is exercised")
    }

    async fn get_run(&self, _request: GetRunRequest) -> ApiResult<GetRunResponse> {
        unreachable!("only watch is exercised")
    }

    async fn list_runs(&self, _request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        unreachable!("only watch is exercised")
    }

    async fn watch_run(&self, _request: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        self.attempts.fetch_add(1, Ordering::AcqRel);
        self.attempted.notify_one();
        Err(unavailable_watch_error())
    }

    fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }

    async fn wait_closed(&self) {
        let mut closed = self.closed.subscribe();
        if *closed.borrow() {
            return;
        }
        while closed.changed().await.is_ok() {
            if *closed.borrow() {
                return;
            }
        }
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> ApiResult<CancelRunResponse> {
        unreachable!("only watch is exercised")
    }

    async fn respond_interaction(
        &self,
        _request: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        unreachable!("only watch is exercised")
    }
}

#[cfg(feature = "daemon")]
#[tokio::test]
async fn closing_a_client_interrupts_watch_reconnect_backoff() {
    let (closed, _) = tokio::sync::watch::channel(false);
    let client = Arc::new(CloseAwareWatchClient {
        closed,
        attempts: AtomicUsize::new(0),
        attempted: tokio::sync::Notify::new(),
    });
    let mut updates = RunUpdates::resilient(
        client.clone(),
        WatchRunRequest {
            run_id: "run-1".into(),
            after_sequence: 0,
        },
        None,
    );

    let error = {
        let next = updates.next_update();
        tokio::pin!(next);
        tokio::select! {
            () = client.attempted.notified() => {}
            result = &mut next => panic!("watch terminated before close: {result:?}"),
        }
        client.closed.send_replace(true);
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut next)
            .await
            .expect("close interrupts backoff")
            .expect("close error")
            .expect_err("closed watch must fail nonretryably")
    };
    assert_eq!(error.code, ApiErrorCode::Unavailable);
    assert!(!error.retryable);
    assert_eq!(client.attempts.load(Ordering::Acquire), 1);
    assert!(updates.next_update().await.is_none());
}

#[cfg(feature = "sidecar")]
struct TestSidecarLifecycle {
    backend: Arc<TestBackend>,
}

#[cfg(feature = "sidecar")]
#[async_trait]
impl SidecarLifecycle for TestSidecarLifecycle {
    async fn start_verified(&self, _options: &SidecarOptions) -> SdkResult<Arc<dyn Backend>> {
        Ok(self.backend.clone())
    }
}

#[cfg(feature = "sidecar")]
fn test_sidecar_options() -> SidecarOptions {
    SidecarOptions::new(
        test_instance_id(),
        AppPrivateInstanceDir::new(std::env::temp_dir().join("colossus-sidecar-state"))
            .expect("absolute state"),
        VerifiedExecutable::new(
            std::env::temp_dir().join("colossus-sidecar-bin"),
            Sha256Digest::from_bytes([4; 32]),
        )
        .expect("absolute executable"),
        ApiMajor::new(1).expect("major"),
    )
    .expect("valid sidecar options")
}

#[cfg(feature = "sidecar")]
#[tokio::test]
async fn sidecar_rejects_a_backend_with_wrong_lifecycle_semantics() {
    let backend = Arc::new(TestBackend::new(BackendKind::Daemon));
    let lifecycle = TestSidecarLifecycle {
        backend: backend.clone(),
    };

    let error = Colossus::start_sidecar(&lifecycle, test_sidecar_options())
        .await
        .expect_err("wrong backend kind");
    assert!(matches!(error, SdkError::IdentityMismatch));
    assert!(backend.closed.load(Ordering::Acquire));
}

#[cfg(feature = "sidecar")]
#[test]
fn sidecar_options_have_no_bootstrap_secret_surface() {
    let debug = format!("{:?}", test_sidecar_options());
    assert!(!debug.contains("credential"));
    assert!(!debug.contains("bootstrap"));
    assert!(!debug.contains("secret"));
}

#[cfg(feature = "embedded")]
struct TestEmbeddedLifecycle {
    backend: Arc<TestBackend>,
}

#[cfg(feature = "embedded")]
#[async_trait]
impl EmbeddedLifecycle for TestEmbeddedLifecycle {
    async fn open_isolated(&self, _options: &EmbeddedOptions) -> SdkResult<Arc<dyn Backend>> {
        Ok(self.backend.clone())
    }
}

#[cfg(feature = "embedded")]
fn test_embedded_options() -> EmbeddedOptions {
    EmbeddedOptions::new(
        InstanceId::from_uuid(Uuid::from_u128(0x018f_1068_d264_7d6c_8f52_0123_4567_89ab)),
        AppPrivateInstanceDir::new(std::env::temp_dir().join("colossus-embedded-state"))
            .expect("absolute state"),
        "app:com.obscuritylabs.desktop",
    )
    .expect("valid embedded options")
}

#[cfg(feature = "embedded")]
#[test]
fn embedded_application_identity_is_strictly_bounded() {
    let state = AppPrivateInstanceDir::new(std::env::temp_dir().join("colossus-embedded-state"))
        .expect("absolute state");
    let invalid = EmbeddedOptions::new(
        InstanceId::from_uuid(Uuid::from_u128(0x018f_1068_d264_7d6c_8f52_0123_4567_89ab)),
        state,
        "application id with spaces",
    );
    assert!(matches!(invalid, Err(SdkError::InvalidConfiguration(_))));
    assert!(
        EmbeddedOptions::new(
            InstanceId::from_uuid(Uuid::from_u128(0x018f_1068_d264_7d6c_8f52_0123_4567_89ab)),
            AppPrivateInstanceDir::new(
                std::env::temp_dir().join("colossus-embedded-state-unprefixed")
            )
            .expect("absolute state"),
            "example/ui",
        )
        .is_err()
    );
    assert!(
        EmbeddedOptions::new(
            InstanceId::from_uuid(Uuid::from_u128(0x018f_1068_d264_7d6c_8f52_0123_4567_89ab)),
            AppPrivateInstanceDir::new(
                std::env::temp_dir().join("colossus-embedded-state-canonical")
            )
            .expect("absolute state"),
            "app:example/ui",
        )
        .is_ok()
    );
}

#[cfg(feature = "embedded")]
#[test]
fn embedded_execution_boundary_defaults_to_full_access_with_explicit_safe_builders() {
    let options = test_embedded_options();
    assert_eq!(
        options.execution_boundary(),
        ManagedExecutionBoundary::FullAccess
    );
    assert_eq!(
        options
            .clone()
            .with_execution_boundary(ManagedExecutionBoundary::WorkspaceIsolated)
            .execution_boundary(),
        ManagedExecutionBoundary::WorkspaceIsolated
    );
    assert_eq!(
        options
            .with_execution_boundary(ManagedExecutionBoundary::OfflineIsolated)
            .execution_boundary(),
        ManagedExecutionBoundary::OfflineIsolated
    );
}

#[cfg(feature = "embedded")]
#[tokio::test]
async fn embedded_rejects_a_backend_with_wrong_lifecycle_semantics() {
    let backend = Arc::new(TestBackend::new(BackendKind::Daemon));
    let lifecycle = TestEmbeddedLifecycle {
        backend: backend.clone(),
    };

    let error = Colossus::open_embedded(&lifecycle, test_embedded_options())
        .await
        .expect_err("wrong backend kind");
    assert!(matches!(error, SdkError::IdentityMismatch));
    assert!(backend.closed.load(Ordering::Acquire));
}
