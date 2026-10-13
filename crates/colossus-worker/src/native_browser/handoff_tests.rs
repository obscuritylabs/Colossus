//! Actual public-run and two-hop presentation protocol; no browser/process launch.
use super::*;
use colossus_api::{
    AgentRunApi as _, ApiScope, ApplicationKind, ApplicationPrincipal, CallerContext, ContentPart,
    CreateRunRequest, IdempotencyKey, RequestId, RunMode,
};
use colossus_api_runtime::{PublicInteractionRouter, RuntimeAgentRunApi};
use colossus_browser_presentation::*;
use colossus_contracts::*;
use colossus_ports::*;
use colossus_runtime::{NativeBrowserOpenRequest, RuntimeBrowserHost, RuntimeBrowserPresenter};
use std::sync::atomic::{AtomicU64, AtomicUsize};
#[path = "tabs_tests.rs"]
mod tabs_tests;

#[derive(Default)]
struct Data {
    original: Option<BrowserDriverOpenRequest>,
    tab: Option<BrowserTabSummary>,
    tabs: Vec<BrowserTabSummary>,
    lease: Option<Lease>,
    fenced: bool,
    control: u64,
}
#[derive(Default)]
struct Host {
    data: Mutex<Data>,
    configurations: AtomicUsize,
    sequence: AtomicU64,
    stale_observation: AtomicBool,
    claimed_target: Mutex<Option<BrowserTarget>>,
}
impl Host {
    fn page(data: &Data) -> PageState {
        let tab = data.tab.as_ref().unwrap();
        PageState {
            target: BrowserTarget {
                tab_id: tab.tab_id.clone(),
                document_id: tab.document_id.clone(),
            },
            url: "https://example.org/page".into(),
            title: "fixture".into(),
            loading: false,
            can_go_back: false,
            can_go_forward: false,
        }
    }
}
#[async_trait]
impl PresentationHost for Host {
    async fn dispatch(
        &self,
        command: PresentationCommand,
    ) -> Result<PresentationReply, PresentationError> {
        let mut data = self.data.lock().unwrap();
        match command {
            PresentationCommand::Configure(config) => {
                if config.target != Self::page(&data).target
                    || config.control_generation != data.control
                    || data.fenced && config.control_generation == 0
                    || data
                        .lease
                        .is_some_and(|old| old.viewport_generation >= config.viewport_generation)
                {
                    return Err(PresentationError::Stale);
                }
                let lease = Lease {
                    tab: if data.original.as_ref().unwrap().tab_id == config.target.tab_id {
                        1
                    } else {
                        2
                    },
                    session_generation: 1,
                    control_generation: config.control_generation,
                    viewport_generation: config.viewport_generation,
                    document_generation: if data.original.as_ref().unwrap().tab_id
                        == config.target.tab_id
                    {
                        3
                    } else {
                        1
                    },
                    pixel_width: config.width,
                    pixel_height: config.height,
                };
                data.lease = Some(lease);
                self.configurations.fetch_add(1, Ordering::SeqCst);
                Ok(PresentationReply::Configured(lease))
            }
            PresentationCommand::FenceHuman { lease }
                if data.lease == Some(lease) && !data.fenced =>
            {
                data.fenced = true;
                Ok(PresentationReply::Fenced(HumanFenceReceipt {
                    prior_lease: lease,
                    state: Self::page(&data),
                    native_document_generation: 3,
                }))
            }
            PresentationCommand::Observe { lease } if data.lease == Some(lease) => {
                let mut page = Self::page(&data);
                if self.stale_observation.swap(false, Ordering::SeqCst) {
                    page.target.tab_id =
                        BrowserTabId::parse(format!("bt_{}", "9".repeat(32))).unwrap();
                }
                if let Some(target) = self.claimed_target.lock().unwrap().take() {
                    page.target = target;
                }
                Ok(PresentationReply::State(page))
            }
            PresentationCommand::Human { lease, .. }
                if data.lease == Some(lease) && !data.fenced && data.control == 0 =>
            {
                Ok(PresentationReply::Ack)
            }
            PresentationCommand::Hide { lease } if data.lease == Some(lease) => {
                Ok(PresentationReply::Ack)
            }
            _ => Err(PresentationError::Stale),
        }
    }
    async fn frame(&self, lease: Lease) -> Result<Option<Frame>, PresentationError> {
        let data = self.data.lock().unwrap();
        let native = if data.original.as_ref().unwrap().tab_id == data.tab.as_ref().unwrap().tab_id
        {
            1
        } else {
            2
        };
        if data.lease != Some(lease) || native != lease.tab {
            return Err(PresentationError::Stale);
        }
        Ok(Some(Frame {
            lease,
            sequence: self.sequence.fetch_add(1, Ordering::SeqCst) + 1,
            stride: lease.pixel_width * 4,
            pixels: vec![255; (lease.pixel_width * lease.pixel_height * 4) as usize],
        }))
    }
    fn revoke_now(&self) {}
    async fn revoke(&self) {}
}
#[derive(Default)]
struct Driver(Arc<Host>);
#[async_trait]
impl BrowserDriver for Driver {
    fn capabilities(&self) -> BrowserCapabilities {
        BrowserCapabilities {
            available: true,
            engine_version: Some("protocol-fixture".into()),
            modes: vec![BrowserMode::Embedded],
            actions: vec![
                BrowserActionKind::Snapshot,
                BrowserActionKind::TabOpen,
                BrowserActionKind::TabSelect,
                BrowserActionKind::TabClose,
            ],
            limits: BrowserLimits::default(),
            private_ca_trust: false,
            client_identities: false,
            restrictive_egress: true,
        }
    }
    async fn open_session(
        &self,
        request: BrowserDriverOpenRequest,
        _: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        let tab = BrowserTabSummary {
            tab_id: request.tab_id.clone(),
            document_id: request.document_id.clone(),
            origin: Some(request.options.allowed_origins[0].clone()),
            title: "fixture".into(),
        };
        let mut data = self.0.data.lock().unwrap();
        data.original = Some(request);
        data.tab = Some(tab.clone());
        data.tabs.push(tab.clone());
        Ok(tab)
    }
    async fn execute(
        &self,
        command: BrowserDriverCommand,
        _: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        let mut data = self.0.data.lock().unwrap();
        let target = data
            .tabs
            .iter()
            .find(|tab| {
                tab.tab_id == command.target.tab_id && tab.document_id == command.target.document_id
            })
            .cloned();
        if command.binding != data.original.as_ref().unwrap().binding || target.is_none() {
            return Err(BrowserDriverError::Denied);
        }
        data.control = command.control_generation;
        let tab = match command.action {
            BrowserAction::TabOpen { .. } => {
                let tab = command.new_tab.ok_or(BrowserDriverError::Denied)?;
                data.tabs.push(tab.clone());
                data.tab = Some(tab.clone());
                tab
            }
            BrowserAction::TabSelect { .. } => {
                let tab = target.unwrap();
                data.tab = Some(tab.clone());
                tab
            }
            BrowserAction::TabClose { .. } => {
                let tab = target.unwrap();
                data.tabs.retain(|owned| owned.tab_id != tab.tab_id);
                if data
                    .tab
                    .as_ref()
                    .is_some_and(|selected| selected.tab_id == tab.tab_id)
                {
                    data.tab = data.tabs.first().cloned();
                }
                tab
            }
            _ => target.unwrap(),
        };
        Ok(BrowserObservation {
            inspection: None,
            session_id: command.session_id,
            tab: tab.clone(),
            truncated: false,
            snapshot: command.snapshot_id.map(|snapshot_id| BrowserSnapshot {
                snapshot_id,
                document_id: tab.document_id,
                nodes: vec![],
                truncated: false,
            }),
        })
    }
    async fn confirm_native_handoff(
        &self,
        request: BrowserNativeHandoffRequest,
        _: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        let data = self.0.data.lock().unwrap();
        if !data.fenced
            || request.binding != data.original.as_ref().unwrap().binding
            || request.confirmed_target != Host::page(&data).target
            || request.native_document_generation != 3
        {
            return Err(BrowserDriverError::Denied);
        }
        Ok(data.tab.clone().unwrap())
    }
    async fn cancel_session(&self, _: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        Ok(())
    }
    async fn close_session(&self, _: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        Ok(())
    }
}
#[async_trait]
impl RuntimeBrowserPresenter for Driver {
    async fn acquire(
        &self,
        _: &BrowserDriverOpenRequest,
    ) -> Result<(PresentationClient, [u8; 32]), BrowserDriverError> {
        let (client, server) = tokio::io::duplex(32768);
        let (cr, cw) = tokio::io::split(client);
        let (sr, sw) = tokio::io::split(server);
        tokio::spawn(serve_presentation(
            PresentationChannel::new(sr, sw),
            Zeroizing::new([1; 32]),
            [2; 32],
            self.0.clone(),
        ));
        Ok((
            PresentationClient::connect(
                PresentationChannel::new(cr, cw),
                Zeroizing::new([1; 32]),
                [2; 32],
            )
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?,
            [2; 32],
        ))
    }
}

#[tokio::test]
async fn public_application_handoff_before_first_tool_configures_each_native_epoch_once() {
    tokio::time::timeout(Duration::from_secs(15), exercise(false))
        .await
        .unwrap();
}

async fn exercise(tab_retarget: bool) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (seen, requested) = tokio::sync::oneshot::channel();
    let (release, proceed) = tokio::sync::oneshot::channel();
    let (completed, mut completions) = tokio::sync::mpsc::channel(1);
    let (permit, permissions) = tokio::sync::mpsc::channel(1);
    let provider = tokio::spawn(async move {
        if tab_retarget {
            tabs_tests::provider(listener, seen, proceed, completed, permissions).await;
            return;
        }
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut chunk = [0; 4096];
            let count = stream.read(&mut chunk).await.unwrap();
            assert!(count > 0);
            bytes.extend_from_slice(&chunk[..count]);
            assert!(bytes.len() < 1_048_576);
            if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                let size: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                if bytes.len() >= end + 4 + size {
                    break;
                }
            }
        }
        seen.send(()).unwrap();
        proceed.await.unwrap();
        let body = "data: {\"id\":\"fixture\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"done\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    });
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let driver = Arc::new(Driver::default());
    let mut config = RuntimeConfig::offline_template(root.join("state.redb"));
    config.use_ephemeral_storage();
    config.access = serde_json::from_value(json!({"profile":"allow_all"})).unwrap();
    config.sandbox.network_destinations = vec![origin.clone(), "https://example.org".into()];
    config.providers = serde_json::from_value(json!({"profiles":{"fixture":{"kind":"open_ai_compatible","baseUrl":format!("{origin}/v1"),"timeoutMs":10000}}})).unwrap();
    config.models = serde_json::from_value(json!({"profiles":{"fixture":{"providerProfile":"fixture","model":"fixture","contextWindowTokens":32768,"maxOutputTokens":4096,"capabilities":{"toolCalls":true,"streaming":true}}},"roles":{"primary":"fixture"}})).unwrap();
    let server = WorkerServer::open_at_workspace(
        &config,
        Arc::new(AllowApproval {
            approved_by: "fixture".into(),
        }),
        RuntimeOpenOptions::for_workspace(&root)
            .unwrap()
            .with_browser_host(
                RuntimeBrowserHost::new(driver.clone()).with_presenter(driver.clone()),
            ),
    )
    .unwrap();
    let credentials =
        Arc::new(server.public_api_credential_manager(PublicApiAuthenticationKey::new([3; 32])));
    let grant = ApplicationGrant::new(
        "app:handoff",
        ApplicationKind::Sidecar,
        [ApiScope::new("sessions:read").unwrap()],
        Vec::<String>::new(),
        Vec::<String>::new(),
    )
    .unwrap();
    let issued = credentials.issue_pending(&grant).unwrap();
    let credential_id = issued.credential_id().to_owned();
    credentials.activate(&credential_id).unwrap();
    let enrollment = NativeBrowserEnrollment {
        generation: *Uuid::now_v7().as_bytes(),
        instance: *Uuid::now_v7().as_bytes(),
        application_id: "app:handoff".into(),
        workspace_version: 1,
        workspace_digest: [5; 32],
        parent_process_id: std::process::id(),
    };
    let mut server = server
        .with_native_browser_api(NativeBrowserServerConfig {
            instance: root,
            enrollment,
            authentication: Zeroizing::new([7; 32]),
            primary_credential_id: credential_id,
            credentials,
        })
        .await
        .unwrap();
    let mut prepared = server.native_browser.take().unwrap();
    let service = prepared.service.clone();
    let authority = service.authority.as_ref().unwrap();
    let session = Arc::new(
        service
            .runtime
            .open_native_browser(
                authority,
                NativeBrowserOpenRequest {
                    conversation_id: None,
                    url: BrowserUrl::parse("https://example.org/page").unwrap(),
                    width: 2,
                    height: 2,
                    scale_milli: 1000,
                    viewport_generation: 1,
                    lease_ms: 1500,
                },
            )
            .await
            .unwrap(),
    );
    service
        .sessions
        .lock()
        .unwrap()
        .insert(session.session_id().clone(), session.clone());
    let relay = Arc::new(relay::NativeRelay::new(service.clone(), session.clone()));
    let (client, outer) = tokio::io::duplex(32768);
    let (cr, cw) = tokio::io::split(client);
    let (sr, sw) = tokio::io::split(outer);
    let outer_task = tokio::spawn(serve_presentation(
        PresentationChannel::new(sr, sw),
        Zeroizing::new([8; 32]),
        session.enrollment_digest(),
        relay.clone(),
    ));
    let client = PresentationClient::connect(
        PresentationChannel::new(cr, cw),
        Zeroizing::new([8; 32]),
        session.enrollment_digest(),
    )
    .await
    .unwrap();
    let initial = session.configure().unwrap();
    let human = client.configure(initial).await.unwrap();
    let old_frame = client.next_frame(human).await.unwrap().unwrap();
    let api = RuntimeAgentRunApi::new(
        service.runtime.clone(),
        Arc::new(PublicInteractionRouter::new(
            Arc::new(AllowApproval {
                approved_by: "fixture".into(),
            }),
            None,
        )),
        "primary",
        "fixture",
    );
    let caller = CallerContext::authenticated(
        ApplicationPrincipal::authenticated(
            "app:handoff",
            "credential-public",
            ApplicationKind::Sidecar,
            ["runs:execute", "runs:read", "runs:control"]
                .map(|scope| ApiScope::new(scope).unwrap()),
            ["primary".into()],
            [
                "browser.open",
                "browser.snapshot",
                "browser.tab.open",
                "browser.tab.select",
                "browser.tab.close",
            ]
            .map(str::to_owned),
        )
        .unwrap(),
        RequestId::new("handoff-public").unwrap(),
    );
    let created = api
        .create_run(
            &caller,
            CreateRunRequest {
                input: vec![ContentPart::Text {
                    text: "pause before any tool".into(),
                }],
                session_id: Some(session.conversation_id().into()),
                end_user_id: Some("terminal-user".into()),
                role: Some("primary".into()),
                mode: RunMode::Execute,
                goal_max_iterations: 0,
                research_depth: None,
                research_sources: vec![],
                skill_ids: vec![],
                plan_action: None,
                branch: None,
                max_turns: 8,
                idempotency_key: IdempotencyKey::new("handoff-public").unwrap(),
            },
        )
        .await
        .unwrap();
    requested.await.unwrap();
    let foreign = service
        .runtime
        .bind_native_browser_authority("app:other", Uuid::from_bytes(service.enrollment.instance))
        .unwrap();
    assert_eq!(
        service
            .runtime
            .handoff_native_browser(&foreign, session.session_id(), &created.run.id, 3000)
            .await
            .err(),
        Some(colossus_runtime::RuntimeNativeBrowserError::Denied)
    );
    let granted = service
        .runtime
        .handoff_native_browser(authority, session.session_id(), &created.run.id, 3000)
        .await
        .unwrap();
    assert_eq!(
        driver.0.configurations.load(Ordering::SeqCst),
        2,
        "one human and one Core handoff native configure"
    );
    let agent = client.configure(granted.configure().clone()).await.unwrap();
    driver.0.stale_observation.store(true, Ordering::SeqCst);
    assert_eq!(
        client.observe(agent).await.err(),
        Some(PresentationError::Stale),
        "known changed document metadata retires observation without losing channel"
    );
    assert!(
        client.observe(agent).await.is_ok(),
        "same authenticated outer channel remains usable"
    );
    assert_eq!(
        driver.0.configurations.load(Ordering::SeqCst),
        2,
        "outer binding consumes Core ACK without reconfigure"
    );
    assert_eq!(
        client.configure(granted.configure().clone()).await,
        Err(PresentationError::Stale)
    );
    assert_eq!(driver.0.configurations.load(Ordering::SeqCst), 2);
    let human_input = PresentationCommand::Human {
        lease: human,
        command: HumanCommand::Navigate {
            url: "https://example.org/page".into(),
        },
    };
    assert!(client.command(human_input).await.is_err());
    let mut codec =
        FrameCodec::new(client.surface_key(), session.enrollment_digest(), agent).unwrap();
    assert!(
        codec.decode(old_frame.clone()).is_err(),
        "old human ciphertext cannot enter the agent epoch"
    );
    assert!(client.next_frame(agent).await.unwrap().is_some());
    let prepared_request = service
        .runtime
        .journal()
        .read_stream(&format!("run:{}", created.run.id))
        .unwrap()
        .into_iter()
        .find(|event| event.event_type == "model.request.prepared.v1")
        .unwrap();
    assert_eq!(prepared_request.actor.actor_type, ActorType::Application);
    assert_eq!(prepared_request.actor.id, "app:handoff");
    release
        .send(tabs_tests::Plan {
            session: session.session_id().clone(),
            control: granted.lease().control_generation,
            original: granted.configure().target.tab_id.clone(),
        })
        .unwrap();
    if tab_retarget {
        tabs_tests::retarget(
            &client,
            &session,
            &driver,
            &mut completions,
            permit,
            human,
            old_frame,
        )
        .await;
    }
    provider.await.unwrap();
    client.disconnect();
    let _ = outer_task.await;
    prepared.listener.cleanup().unwrap();
}
