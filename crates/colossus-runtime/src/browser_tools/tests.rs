use super::*;
use crate::{
    Runtime, RuntimeConfig, RuntimeOpenOptions, prelude::*, test_support::private_tempdir,
};
use colossus_contracts::*;
use colossus_ports::{
    AgentRunLifecycle, BrowserDriver, BrowserDriverCommand, BrowserDriverControl,
    BrowserDriverError, BrowserDriverOpenRequest,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[cfg(all(target_os = "linux", debug_assertions))]
mod native;
#[cfg(all(target_os = "linux", debug_assertions))]
mod native_pki;
#[cfg(all(target_os = "linux", debug_assertions))]
mod native_presentation;
#[cfg(all(target_os = "linux", debug_assertions))]
mod native_profiles;
mod screenshot;
mod transfer;

struct Driver {
    capabilities: BrowserCapabilities,
    opened: AtomicUsize,
    executed: AtomicUsize,
    closed: AtomicUsize,
    close_started: AtomicUsize,
    blocked_close: StdMutex<Option<(BrowserSessionId, Arc<tokio::sync::Notify>)>>,
    unknown: AtomicBool,
    captured: AtomicUsize,
    capture_command: StdMutex<Option<BrowserDriverCommand>>,
    transfers: AtomicUsize,
    transfer: StdMutex<Option<transfer::Pending>>,
}

impl Driver {
    fn verified() -> Self {
        Self {
            capabilities: BrowserCapabilities {
                available: true,
                engine_version: Some("fixture".into()),
                modes: vec![BrowserMode::Headless, BrowserMode::Embedded],
                actions: vec![
                    BrowserActionKind::Navigate,
                    BrowserActionKind::Snapshot,
                    BrowserActionKind::Click,
                    BrowserActionKind::Fill,
                ],
                limits: BrowserLimits::default(),
                private_ca_trust: true,
                client_identities: true,
                restrictive_egress: true,
            },
            opened: AtomicUsize::new(0),
            executed: AtomicUsize::new(0),
            closed: AtomicUsize::new(0),
            close_started: AtomicUsize::new(0),
            blocked_close: StdMutex::new(None),
            unknown: AtomicBool::new(false),
            captured: AtomicUsize::new(0),
            capture_command: StdMutex::new(None),
            transfers: AtomicUsize::new(0),
            transfer: StdMutex::new(None),
        }
    }
}

#[async_trait]
impl BrowserDriver for Driver {
    fn capabilities(&self) -> BrowserCapabilities {
        self.capabilities.clone()
    }
    async fn capture(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<colossus_ports::BrowserScreenshotDescriptor, BrowserDriverError> {
        if control.is_cancelled() {
            return Err(BrowserDriverError::Cancelled);
        }
        self.captured.fetch_add(1, Ordering::SeqCst);
        let bytes = BASE64.decode(screenshot::PNG).unwrap();
        let descriptor = colossus_ports::BrowserScreenshotDescriptor {
            session_id: command.session_id.clone(),
            target: command.target.clone(),
            control_generation: command.control_generation,
            transfer_id: "d".repeat(32),
            size_bytes: bytes.len() as u32,
            sha256: hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&bytes)),
            width: 1,
            height: 1,
        };
        *self.capture_command.lock().unwrap() = Some(command);
        Ok(descriptor)
    }
    async fn read_screenshot_chunk(
        &self,
        request: colossus_ports::BrowserScreenshotReadRequest,
        control: &BrowserDriverControl,
    ) -> Result<colossus_ports::BrowserScreenshotChunk, BrowserDriverError> {
        let command = self
            .capture_command
            .lock()
            .unwrap()
            .take()
            .ok_or(BrowserDriverError::Stale)?;
        if control.is_cancelled()
            || request.offset != 0
            || request.binding != command.binding
            || request.run_id != command.run_id
            || request.session_id != command.session_id
            || request.target != command.target
            || request.control_generation != command.control_generation
        {
            return Err(BrowserDriverError::Stale);
        }
        Ok(colossus_ports::BrowserScreenshotChunk {
            offset: 0,
            data_base64: screenshot::PNG.into(),
        })
    }

    async fn open_session(
        &self,
        request: BrowserDriverOpenRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        if control.is_cancelled() {
            return Err(BrowserDriverError::Cancelled);
        }
        self.opened.fetch_add(1, Ordering::SeqCst);
        Ok(BrowserTabSummary {
            tab_id: request.tab_id,
            document_id: request.document_id,
            origin: request.options.initial_url.map(|url| url.origin()),
            title: "fixture".into(),
        })
    }

    async fn prepare_upload(
        &self,
        request: colossus_ports::BrowserUploadPrepareRequest,
        _: &BrowserDriverControl,
    ) -> Result<colossus_ports::BrowserUploadReceipt, BrowserDriverError> {
        self.transfer_prepare(request)
    }
    async fn write_upload_chunk(
        &self,
        request: colossus_ports::BrowserUploadWriteRequest,
        _: &BrowserDriverControl,
    ) -> Result<colossus_ports::BrowserUploadReceipt, BrowserDriverError> {
        self.transfer_write(request)
    }
    async fn commit_upload(
        &self,
        request: colossus_ports::BrowserUploadCommitRequest,
        _: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        self.transfer_commit(request)
    }
    async fn download(
        &self,
        command: BrowserDriverCommand,
        _: &BrowserDriverControl,
    ) -> Result<colossus_ports::BrowserDownloadDescriptor, BrowserDriverError> {
        self.transfer_download(command)
    }
    async fn read_download_chunk(
        &self,
        request: colossus_ports::BrowserDownloadReadRequest,
        _: &BrowserDriverControl,
    ) -> Result<colossus_ports::BrowserDownloadChunk, BrowserDriverError> {
        self.transfer_read(request)
    }

    async fn execute(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        if control.is_cancelled() {
            return Err(BrowserDriverError::Cancelled);
        }
        self.executed.fetch_add(1, Ordering::SeqCst);
        if self.unknown.load(Ordering::SeqCst) {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let tab = BrowserTabSummary {
            tab_id: command.target.tab_id,
            document_id: if matches!(command.action, BrowserAction::Navigate { .. }) {
                command.next_document_id
            } else {
                command.target.document_id
            },
            origin: Some(BrowserOrigin::parse("https://example.org").unwrap()),
            title: "fixture".into(),
        };
        let snapshot = command.snapshot_id.map(|snapshot_id| BrowserSnapshot {
            snapshot_id: snapshot_id.clone(),
            document_id: tab.document_id.clone(),
            nodes: vec![BrowserSnapshotNode {
                element: BrowserElementRef {
                    document_id: tab.document_id.clone(),
                    snapshot_id,
                    element_id: BrowserElementId::parse(format!("be_{}", "1".repeat(32))).unwrap(),
                },
                role: "textbox".into(),
                name: "ordinary field".into(),
                value: None,
            }],
            truncated: false,
        });
        Ok(BrowserObservation {
            session_id: command.session_id,
            tab,
            snapshot,
            truncated: false,
        })
    }

    async fn cancel_session(&self, _: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        Ok(())
    }

    async fn close_session(&self, session_id: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.close_started.fetch_add(1, Ordering::SeqCst);
        let blocked = self
            .blocked_close
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(id, _)| id == session_id)
            .map(|(_, notify)| Arc::clone(notify));
        if let Some(notify) = blocked {
            notify.notified().await;
        }
        self.closed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct NoFallback;
#[async_trait]
impl ToolExecutor for NoFallback {
    async fn execute(&self, call: ToolCall, _: ExecutionContext) -> Result<ToolResult, ToolError> {
        Err(ToolError::Unknown(call.name))
    }
}

fn config(workspace: &Path) -> RuntimeConfig {
    let mut config = RuntimeConfig::offline_template(workspace.join("state.redb"));
    config.use_ephemeral_storage();
    config.access.profile = AccessProfile::AllowAll;
    config.sandbox.network_destinations = vec!["https://example.org".into()];
    config
}

fn runtime(workspace: &Path, config: &RuntimeConfig, driver: Option<Arc<Driver>>) -> Runtime {
    let mut options = RuntimeOpenOptions::for_workspace(workspace).unwrap();
    if let Some(driver) = driver {
        options = options.with_browser_host(RuntimeBrowserHost::new(driver));
    }
    Runtime::open_with_options(config, Arc::new(DenyApproval), None, options).unwrap()
}

fn executor(runtime: &Runtime) -> BrowserToolExecutor {
    BrowserToolExecutor {
        gateway: Arc::clone(&runtime.gateway),
        registry: Arc::clone(&runtime.tools),
        browser: Arc::clone(runtime.browser.as_ref().unwrap()),
        journal: runtime.journal(),
        inner: Arc::new(NoFallback),
    }
}

fn begin(runtime: &Runtime, run_id: &str) -> ExecutionContext {
    let context = ExecutionContext {
        run_id: Some(run_id.into()),
        session_id: Some("conversation".into()),
        offered_tools: runtime
            .tools
            .list_specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect(),
        ..ExecutionContext::default()
    };
    runtime
        .browser
        .as_ref()
        .unwrap()
        .begin_run(
            &context,
            &Actor {
                actor_type: ActorType::User,
                id: "authenticated-app".into(),
            },
            RunControl::default(),
        )
        .unwrap();
    context
}

async fn call(
    executor: &BrowserToolExecutor,
    context: &ExecutionContext,
    name: &str,
    arguments: Value,
) -> Result<Value, ToolError> {
    let result = executor
        .execute(
            ToolCall {
                call_id: Uuid::now_v7().to_string(),
                name: name.into(),
                arguments,
            },
            context.clone(),
        )
        .await?;
    Ok(serde_json::from_str(&result.output).unwrap())
}

fn open_arguments() -> Value {
    json!({"mode": "headless", "allowed_origins": ["https://example.org"], "initial_url": "https://example.org/start"})
}

fn target(opened: &Value) -> Value {
    json!({"session_id": opened["session"]["session_id"], "control_generation": opened["control_generation"], "tab_id": opened["session"]["tabs"][0]["tab_id"], "document_id": opened["session"]["tabs"][0]["document_id"]})
}

fn snapshot_target(opened: &Value) -> Value {
    let mut target = target(opened);
    target["max_nodes"] = json!(10);
    target
}

#[test]
fn unavailable_backend_preserves_explicit_selection_diagnostics() {
    let workspace = private_tempdir();
    let mut config = config(workspace.path());
    config.access.tools.include = vec!["browser.open".into()];
    let runtime = runtime(workspace.path(), &config, None);
    assert!(!runtime.browser_capabilities().available);
    assert!(
        runtime
            .tools
            .list_specs()
            .iter()
            .all(|spec| !spec.name.starts_with("browser."))
    );
    assert_eq!(
        runtime.state_doctor().unwrap()["browser"]["available"],
        false
    );
}

#[test]
fn partial_backend_advertises_only_proved_operations() {
    let workspace = private_tempdir();
    let driver = Arc::new(Driver::verified());
    let runtime = runtime(workspace.path(), &config(workspace.path()), Some(driver));
    let names = runtime
        .tools
        .list_specs()
        .into_iter()
        .map(|spec| spec.name)
        .collect::<Vec<_>>();
    assert!(names.contains(&"browser.snapshot".into()));
    assert!(!names.contains(&"browser.press".into()));
    assert!(!names.contains(&"browser.screenshot".into()));
}

#[tokio::test]
async fn denied_origin_and_action_never_allocate_a_browser() {
    for deny_action in [false, true] {
        let workspace = private_tempdir();
        let driver = Arc::new(Driver::verified());
        let mut config = config(workspace.path());
        if deny_action {
            config.access.actions.deny = vec!["browser.open".into()];
        }
        let runtime = runtime(workspace.path(), &config, Some(Arc::clone(&driver)));
        let context = begin(&runtime, "run");
        let mut arguments = open_arguments();
        if !deny_action {
            arguments["allowed_origins"] = json!(["https://other.example"]);
            arguments["initial_url"] = json!("https://other.example");
        }
        assert!(
            call(&executor(&runtime), &context, "browser.open", arguments)
                .await
                .is_err()
        );
        assert_eq!(driver.opened.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn unoffered_tool_and_forged_run_provenance_never_allocate() {
    let workspace = private_tempdir();
    let driver = Arc::new(Driver::verified());
    let runtime = runtime(
        workspace.path(),
        &config(workspace.path()),
        Some(Arc::clone(&driver)),
    );
    let context = begin(&runtime, "run");
    let mut unoffered = context.clone();
    unoffered.offered_tools.clear();
    assert!(matches!(
        call(
            &executor(&runtime),
            &unoffered,
            "browser.open",
            open_arguments()
        )
        .await,
        Err(ToolError::Denied(_))
    ));
    let mut forged = context.clone();
    forged.session_id = Some("other-conversation".into());
    assert!(
        call(
            &executor(&runtime),
            &forged,
            "browser.open",
            open_arguments()
        )
        .await
        .is_err()
    );
    let mut forged_lineage = context;
    forged_lineage.step_id = Some("unregistered-step".into());
    assert!(
        call(
            &executor(&runtime),
            &forged_lineage,
            "browser.open",
            open_arguments()
        )
        .await
        .is_err()
    );
    assert_eq!(driver.opened.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn state_survives_calls_and_element_handles_expire_after_mutation() {
    let workspace = private_tempdir();
    let driver = Arc::new(Driver::verified());
    let runtime = runtime(
        workspace.path(),
        &config(workspace.path()),
        Some(Arc::clone(&driver)),
    );
    let context = begin(&runtime, "run");
    let executor = executor(&runtime);
    let opened = call(&executor, &context, "browser.open", open_arguments())
        .await
        .unwrap();
    assert!(runtime.browser_active_work());
    let target = target(&opened);
    let snapshot = call(
        &executor,
        &context,
        "browser.snapshot",
        snapshot_target(&opened),
    )
    .await
    .unwrap();
    let mut click = target.clone();
    click["snapshot_id"] = snapshot["snapshot"]["snapshot_id"].clone();
    click["element_id"] = snapshot["snapshot"]["nodes"][0]["element"]["element_id"].clone();
    call(&executor, &context, "browser.click", click.clone())
        .await
        .unwrap();
    assert!(
        call(&executor, &context, "browser.click", click)
            .await
            .is_err()
    );
    assert_eq!(driver.opened.load(Ordering::SeqCst), 1);
    assert_eq!(driver.executed.load(Ordering::SeqCst), 2);
    runtime.browser.as_ref().unwrap().finish_run("run").await;
    assert_eq!(driver.closed.load(Ordering::SeqCst), 1);
    assert!(!runtime.browser_active_work());
}

#[tokio::test]
async fn other_run_cannot_claim_session_and_cancel_revokes_dispatch() {
    let workspace = private_tempdir();
    let driver = Arc::new(Driver::verified());
    let runtime = runtime(
        workspace.path(),
        &config(workspace.path()),
        Some(Arc::clone(&driver)),
    );
    let first = begin(&runtime, "first");
    let second = begin(&runtime, "second");
    let executor = executor(&runtime);
    let opened = call(&executor, &first, "browser.open", open_arguments())
        .await
        .unwrap();
    assert!(
        call(
            &executor,
            &second,
            "browser.snapshot",
            snapshot_target(&opened)
        )
        .await
        .is_err()
    );
    runtime.browser.as_ref().unwrap().cancel_run("first");
    assert!(
        call(
            &executor,
            &first,
            "browser.snapshot",
            snapshot_target(&opened)
        )
        .await
        .is_err()
    );
    runtime.drain_browser_sessions().await;
    assert_eq!(driver.executed.load(Ordering::SeqCst), 0);
    assert_eq!(driver.closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unknown_mutation_is_not_retried_and_unicode_byte_limits_are_enforced() {
    let workspace = private_tempdir();
    let driver = Arc::new(Driver::verified());
    let runtime = runtime(
        workspace.path(),
        &config(workspace.path()),
        Some(Arc::clone(&driver)),
    );
    let context = begin(&runtime, "run");
    let executor = executor(&runtime);
    let opened = call(&executor, &context, "browser.open", open_arguments())
        .await
        .unwrap();
    let snapshot = call(
        &executor,
        &context,
        "browser.snapshot",
        snapshot_target(&opened),
    )
    .await
    .unwrap();
    let mut fill = target(&opened);
    fill["snapshot_id"] = snapshot["snapshot"]["snapshot_id"].clone();
    fill["element_id"] = snapshot["snapshot"]["nodes"][0]["element"]["element_id"].clone();
    fill["text"] = json!("🔥".repeat(3000));
    assert!(matches!(
        call(&executor, &context, "browser.fill", fill).await,
        Err(ToolError::InvalidArguments { .. })
    ));
    assert_eq!(driver.executed.load(Ordering::SeqCst), 1);
    let mut click = target(&opened);
    click["snapshot_id"] = snapshot["snapshot"]["snapshot_id"].clone();
    click["element_id"] = snapshot["snapshot"]["nodes"][0]["element"]["element_id"].clone();
    driver.unknown.store(true, Ordering::SeqCst);
    assert!(matches!(
        call(&executor, &context, "browser.click", click.clone()).await,
        Err(ToolError::OutcomeUnknown(_))
    ));
    assert!(
        call(&executor, &context, "browser.click", click)
            .await
            .is_err()
    );
    assert_eq!(driver.executed.load(Ordering::SeqCst), 2);
    let status = call(
        &executor,
        &context,
        "browser.status",
        json!({"session_id": opened["session"]["session_id"]}),
    )
    .await
    .unwrap();
    assert!(status["control_generation"] != opened["control_generation"]);
    assert!(
        call(
            &executor,
            &context,
            "browser.close",
            json!({"session_id": opened["session"]["session_id"], "control_generation": opened["control_generation"]}),
        )
        .await
        .is_err()
    );
    call(
        &executor,
        &context,
        "browser.close",
        json!({"session_id": opened["session"]["session_id"], "control_generation": status["control_generation"]}),
    )
    .await
    .unwrap();
    assert_eq!(driver.closed.load(Ordering::SeqCst), 1);
    assert!(!runtime.browser_active_work());
    runtime.drain_browser_sessions().await;
}

#[tokio::test]
async fn human_takeover_prevents_agent_close_even_with_fresh_generation() {
    let workspace = private_tempdir();
    let driver = Arc::new(Driver::verified());
    let runtime = runtime(
        workspace.path(),
        &config(workspace.path()),
        Some(Arc::clone(&driver)),
    );
    let context = begin(&runtime, "run");
    let executor = executor(&runtime);
    let opened = call(&executor, &context, "browser.open", open_arguments())
        .await
        .unwrap();
    let browser = runtime.browser.as_ref().unwrap();
    let run = browser.run(&context).unwrap();
    let session_id: BrowserSessionId =
        serde_json::from_value(opened["session"]["session_id"].clone()).unwrap();
    browser
        .coordinator
        .takeover(&run.actor.binding, &session_id)
        .await
        .unwrap();
    let status = call(
        &executor,
        &context,
        "browser.status",
        json!({"session_id": session_id}),
    )
    .await
    .unwrap();
    assert!(
        call(
            &executor,
            &context,
            "browser.close",
            json!({"session_id": session_id, "control_generation": status["control_generation"]}),
        )
        .await
        .is_err()
    );
    assert_eq!(driver.closed.load(Ordering::SeqCst), 0);
    assert!(runtime.browser_active_work());
    runtime.drain_browser_sessions().await;
    assert_eq!(driver.closed.load(Ordering::SeqCst), 1);
}

struct PostDeny(BuiltInPolicy);

#[async_trait]
impl PolicyDecisionPoint for PostDeny {
    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, PolicyError> {
        let mut decision = self.0.decide(request).await?;
        if request.phase == EffectPhase::PostEffect {
            decision.outcome = DecisionOutcome::Deny;
            decision.reason = "browser release denied".into();
        }
        Ok(decision)
    }
    async fn doctor(&self) -> Result<Value, PolicyError> {
        self.0.doctor().await
    }
}

#[tokio::test]
async fn post_effect_denial_withholds_browser_evidence_and_cleanup_still_runs() {
    let workspace = private_tempdir();
    let driver = Arc::new(Driver::verified());
    let runtime = runtime(
        workspace.path(),
        &config(workspace.path()),
        Some(Arc::clone(&driver)),
    );
    let context = begin(&runtime, "run");
    let mut executor = executor(&runtime);
    executor.gateway = Arc::new(EffectGateway::new(
        Arc::clone(&runtime.journal),
        Arc::new(PostDeny(
            BuiltInPolicy::offline_default()
                .with_action("browser.open", DecisionOutcome::Allow)
                .with_network_destination("https://example.org"),
        )),
        Arc::new(DenyApproval),
        SafetyKernel::new(["browser.open".into()]),
        [7; 32],
    ));
    let error = call(&executor, &context, "browser.open", open_arguments())
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::Denied(_)));
    assert!(!error.to_string().contains("fixture"));
    assert_eq!(driver.opened.load(Ordering::SeqCst), 1);
    runtime.drain_browser_sessions().await;
    assert_eq!(driver.closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn narrow_output_policy_fails_before_allocation() {
    let workspace = private_tempdir();
    let driver = Arc::new(Driver::verified());
    let mut config = config(workspace.path());
    config.sandbox.max_output_bytes = 1024;
    let runtime = runtime(workspace.path(), &config, Some(Arc::clone(&driver)));
    let context = begin(&runtime, "run");
    assert!(
        call(
            &executor(&runtime),
            &context,
            "browser.open",
            open_arguments()
        )
        .await
        .is_err()
    );
    assert_eq!(driver.opened.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancellation_without_finish_schedules_native_cleanup() {
    let workspace = private_tempdir();
    let driver = Arc::new(Driver::verified());
    let runtime = runtime(
        workspace.path(),
        &config(workspace.path()),
        Some(Arc::clone(&driver)),
    );
    let context = begin(&runtime, "run");
    call(
        &executor(&runtime),
        &context,
        "browser.open",
        open_arguments(),
    )
    .await
    .unwrap();
    runtime.browser.as_ref().unwrap().cancel_run("run");
    tokio::time::timeout(Duration::from_secs(1), async {
        while driver.closed.load(Ordering::SeqCst) == 0
            || !runtime
                .browser
                .as_ref()
                .unwrap()
                .runs
                .lock()
                .unwrap()
                .is_empty()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("drop-path cleanup");
    assert_eq!(driver.closed.load(Ordering::SeqCst), 1);
    runtime.drain_browser_sessions().await;
}

#[tokio::test]
async fn stalled_run_teardown_does_not_block_another_runs_cleanup() {
    let workspace = private_tempdir();
    let driver = Arc::new(Driver::verified());
    let runtime = runtime(
        workspace.path(),
        &config(workspace.path()),
        Some(Arc::clone(&driver)),
    );
    let first = begin(&runtime, "first");
    let second = begin(&runtime, "second");
    let executor = executor(&runtime);
    let opened = call(&executor, &first, "browser.open", open_arguments())
        .await
        .unwrap();
    call(&executor, &second, "browser.open", open_arguments())
        .await
        .unwrap();
    let first_id: BrowserSessionId =
        serde_json::from_value(opened["session"]["session_id"].clone()).unwrap();
    let release = Arc::new(tokio::sync::Notify::new());
    *driver.blocked_close.lock().unwrap() = Some((first_id, Arc::clone(&release)));
    runtime.browser.as_ref().unwrap().cancel_run("first");
    tokio::time::timeout(Duration::from_secs(1), async {
        while driver.close_started.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("first close started");
    runtime.browser.as_ref().unwrap().cancel_run("second");
    tokio::time::timeout(Duration::from_secs(1), async {
        while driver.closed.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("second close is independent");
    assert_eq!(driver.closed.load(Ordering::SeqCst), 1);
    release.notify_one();
    runtime.drain_browser_sessions().await;
    assert_eq!(driver.closed.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn plaintext_public_origins_do_not_bypass_transport_policy() {
    let workspace = private_tempdir();
    let driver = Arc::new(Driver::verified());
    let mut config = config(workspace.path());
    config.sandbox.network_destinations = vec!["http://example.org".into()];
    let runtime = runtime(workspace.path(), &config, Some(Arc::clone(&driver)));
    let context = begin(&runtime, "run");
    assert!(call(&executor(&runtime), &context, "browser.open", json!({"mode":"headless", "allowed_origins":["http://example.org"], "initial_url":"http://example.org"})).await.is_err());
    assert_eq!(driver.opened.load(Ordering::SeqCst), 0);
}
