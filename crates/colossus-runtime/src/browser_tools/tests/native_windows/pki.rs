//! Real system-store PKI through Core's authenticated owner and effect gateway.
use super::*;
use colossus_windows_native::{BoundPath, PrivateDirectoryCreation};
use std::collections::BTreeMap;

#[derive(serde::Deserialize)]
struct Fixture {
    urls: BTreeMap<String, String>,
}

async fn titled_page(
    executor: &BrowserToolExecutor,
    context: &ExecutionContext,
    opened: &Value,
    mut observed: BrowserObservation,
    expected: &str,
) -> BrowserObservation {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let mut arguments = observed_target(opened, &observed);
        arguments["max_nodes"] = 128.into();
        let result = tokio::time::timeout_at(
            deadline,
            call(executor, context, "browser.snapshot", arguments),
        )
        .await
        .unwrap();
        match result {
            Ok(value) => observed = serde_json::from_value(value).unwrap(),
            Err(ToolError::Failed(message))
                if message == "effect failed: browser document or element is stale" => {}
            Err(error) => panic!("real Core PKI read-only title observation failed: {error}"),
        }
        if observed.tab.title == expected {
            return observed;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "real server title did not reach Core"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires generated fixture-only CurrentUser ROOT/MY+CNG keys, pinned CEF component and exact native dialog driver"]
async fn core_authenticates_system_ca_and_reviewed_client_identity_after_policy() {
    let directory = PathBuf::from(
        std::env::var_os("COLOSSUS_WINDOWS_PKI_FIXTURE").expect("explicit owned fixture"),
    );
    let source = BoundPath::open_directory(&directory).unwrap();
    source.validate_private_owner_dacl().unwrap();
    let encoded = std::fs::read(directory.join("fixture.json")).unwrap();
    assert!(encoded.len() <= 64 * 1024);
    let fixture: Fixture = serde_json::from_slice(&encoded).unwrap();
    let component = PathBuf::from(
        std::env::var_os("COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_COMPONENT")
            .expect("verified CEF stage"),
    );
    let parent = PathBuf::from(
        std::env::var_os("COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_STATE_PARENT")
            .expect("owner-private state parent"),
    );
    let manifest = std::fs::read(component.join("browser-component.json")).unwrap();
    let metadata: Value = serde_json::from_slice(&manifest).unwrap();
    assert_eq!(metadata["modes"]["desktop"], false);
    assert_eq!(metadata["modes"]["headless"], false);
    let manifest_digest: [u8; 32] = sha2::Sha256::digest(&manifest).into();
    let executable = component.join("colossus-native-browser-host.exe");
    let executable_digest = sha2::Sha256::digest(std::fs::read(&executable).unwrap());
    let mut state = PrivateDirectoryCreation::create(
        &parent.join(format!("windows-core-pki-{}", Uuid::now_v7().simple())),
    )
    .unwrap();
    let allocation = state.path().to_owned();
    let supervisor = Arc::new(
        WindowsBrowserSupervisor::install_for_development(WindowsBrowserConfig {
            component_root: component.clone(),
            manifest_sha256: manifest_digest,
            state_root: allocation.clone(),
            capabilities: BrowserCapabilities {
                available: true,
                engine_version: Some(metadata["cef_version"].as_str().unwrap().into()),
                modes: vec![BrowserMode::Embedded],
                actions: vec![BrowserActionKind::Navigate, BrowserActionKind::Snapshot],
                limits: BrowserLimits::default(),
                private_ca_trust: false,
                client_identities: true,
                restrictive_egress: false,
            },
            memory_bytes: 1024 * 1024 * 1024,
            max_processes: 64,
            egress: BrowserEgressLimits {
                max_connections: 16,
                lifetime: Duration::from_secs(120),
                connection_timeout: Duration::from_secs(10),
                max_connection_bytes: 1024 * 1024,
            },
        })
        .await
        .unwrap(),
    );
    let captured = Arc::new(CapturedHost {
        supervisor: Arc::clone(&supervisor),
        request: StdMutex::new(None),
    });
    let workspace = private_tempdir();
    let mut configuration = config(workspace.path());
    configuration.sandbox.network_destinations = fixture.urls.values().cloned().collect();
    let runtime = Arc::new(
        Runtime::open_with_options(
            &configuration,
            Arc::new(DenyApproval),
            None,
            RuntimeOpenOptions::for_workspace(workspace.path())
                .unwrap()
                .with_browser_host(
                    RuntimeBrowserHost::from_supervised_factory(captured.clone()).unwrap(),
                ),
        )
        .unwrap(),
    );
    let task_runtime = Arc::clone(&runtime);
    let task_captured = Arc::clone(&captured);
    let acknowledged = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let task_acknowledged = Arc::clone(&acknowledged);
    let mut work = tokio::spawn(async move {
        let runtime = &*task_runtime;
        let executor = executor(runtime);
        let context = begin(runtime, "core-pki-policy-denied");
        assert!(call(&executor,&context,"browser.open",json!({"mode":"embedded",
            "allowed_origins":["https://unreviewed.invalid"],"initial_url":"https://unreviewed.invalid/"})).await.is_err());
        for key in ["client_identity", "certificate", "pki"] {
            let mut arguments = json!({"mode":"embedded","allowed_origins":fixture.urls.values().collect::<Vec<_>>()});
            arguments[key] = "model-cannot-select-native-credentials".into();
            assert!(matches!(
                call(&executor, &context, "browser.open", arguments).await,
                Err(ToolError::InvalidArguments { .. })
            ));
        }
        assert!(
            task_captured.request.lock().unwrap().is_none(),
            "Core policy/schema denial must precede native allocation"
        );
        runtime
            .browser
            .as_ref()
            .unwrap()
            .finish_run("core-pki-policy-denied")
            .await;
        for (name, endpoint, route, expected) in [
            (
                "trusted_ca",
                "public",
                "/public",
                Some("Colossus private CA verified"),
            ),
            ("missing_identity", "mtls", "/client-check", None),
            (
                "alternate_identity",
                "mtls",
                "/client-check",
                Some("Colossus identity rejected"),
            ),
            (
                "exact_identity",
                "mtls",
                "/client-check",
                Some("Colossus mTLS identity verified"),
            ),
            ("changed_origin", "mtls", "/redirect-denied", None),
            ("untrusted_root", "untrusted", "/public", None),
            ("wrong_hostname", "wrong_hostname", "/public", None),
            ("expired_server", "expired", "/public", None),
            ("cancelled_request", "mtls", "/client-check", None),
        ] {
            eprintln!("EVIDENCE windows-pki case={name} stage=begin");
            let run_id = format!("core-pki-{name}");
            let control = RunControl::default();
            let context = ExecutionContext {
                run_id: Some(run_id.clone()),
                session_id: Some("pki-owned-conversation".into()),
                offered_tools: runtime
                    .tools
                    .list_specs()
                    .into_iter()
                    .map(|tool| tool.name)
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
                        id: "authenticated-pki-app".into(),
                    },
                    control.clone(),
                )
                .unwrap();
            let opened = call(
                &executor,
                &context,
                "browser.open",
                json!({"mode":"embedded",
                "allowed_origins":fixture.urls.values().collect::<Vec<_>>()}),
            )
            .await
            .unwrap();
            let request = task_captured.request.lock().unwrap().clone().unwrap();
            assert_eq!(request.binding.application_id, "authenticated-pki-app");
            assert_eq!(
                request.binding.runtime_id,
                runtime.browser.as_ref().unwrap().runtime_id
            );
            assert_eq!(
                request.binding.scope,
                BrowserScope::Conversation {
                    id: "pki-owned-conversation".into()
                }
            );
            assert_eq!(request.run_id.as_deref(), Some(run_id.as_str()));
            // Core requires observed document state before any navigation. The
            // direct-driver fixture does not exercise this authority precondition.
            let blank: BrowserObservation = serde_json::from_value(
                call(
                    &executor,
                    &context,
                    "browser.snapshot",
                    snapshot_target(&opened),
                )
                .await
                .unwrap(),
            )
            .unwrap();
            let mut arguments = observed_target(&opened, &blank);
            arguments["url"] = format!("{}{}", fixture.urls[endpoint], route).into();
            let cancellation_directory = directory.clone();
            let cancellation = if name == "cancelled_request" {
                Some(tokio::spawn(async move {
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
                    while !cancellation_directory
                        .join("cancelled-request-dialog-seen")
                        .exists()
                    {
                        assert!(
                            tokio::time::Instant::now() < deadline,
                            "real native prompt required before Core cancellation"
                        );
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    control.cancel();
                }))
            } else {
                None
            };
            let result = tokio::time::timeout(
                Duration::from_secs(55),
                call(&executor, &context, "browser.navigate", arguments),
            )
            .await
            .unwrap();
            if let Some(cancellation) = cancellation {
                cancellation.await.unwrap();
            }
            if let Some(expected) = expected {
                let observation: BrowserObservation =
                    serde_json::from_value(result.unwrap()).unwrap();
                let observation =
                    titled_page(&executor, &context, &opened, observation, expected).await;
                assert_eq!(
                    observation.tab.title, expected,
                    "actual server login/title through released Core observation"
                );
                call(
                    &executor,
                    &context,
                    "browser.close",
                    json!({"session_id":request.session_id,
                    "control_generation":opened["control_generation"]}),
                )
                .await
                .unwrap();
            } else {
                assert!(
                    matches!(result, Err(ToolError::OutcomeUnknown(_))),
                    "TLS/native review rejection cannot be released as Core success"
                );
            }
            runtime.browser.as_ref().unwrap().finish_run(&run_id).await;
            assert!(!runtime.browser_active_work());
            let receipts = task_captured.supervisor.shutdown_receipts().await;
            assert!(
                receipts
                    .iter()
                    .any(|receipt| receipt.0 == request.session_id
                        && receipt.1 == Some(WindowsBrowserShutdownReceipt::GracefulCefShutdown)),
                "each Core case must acknowledge exact native shutdown before the next allocation"
            );
            task_acknowledged.fetch_add(1, Ordering::SeqCst);
            eprintln!("PASS windows-pki case={name} core_effect_gateway=true");
        }
    });
    let result = tokio::time::timeout(Duration::from_secs(300), &mut work).await;
    if result.is_err() {
        work.abort();
        let _ = work.await;
    }
    let cleanup = tokio::time::timeout(Duration::from_secs(30), supervisor.shutdown()).await;
    let receipts = supervisor.shutdown_receipts().await;
    assert!(
        matches!(cleanup, Ok(Ok(()))),
        "native cleanup unknown; preserve private allocation"
    );
    drop(captured);
    drop(supervisor);
    state
        .remove_empty()
        .expect("all Core-owned Windows PKI native profiles removed");
    assert!(!allocation.exists());
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        _ => panic!("real Windows Core PKI acceptance did not finish"),
    }
    assert_eq!(acknowledged.load(Ordering::SeqCst), 9);
    assert_eq!(
        receipts.len(),
        1,
        "supervisor bounds retained closed-session metadata"
    );
    assert!(
        receipts
            .iter()
            .all(|receipt| receipt.1 == Some(WindowsBrowserShutdownReceipt::GracefulCefShutdown))
    );
    let journal = runtime.journal();
    journal.verify().unwrap();
    assert!(
        journal.head().unwrap().0 < 1024,
        "bounded fixture journal must fit its audit page"
    );
    let events = journal.read_global(1, 1024).unwrap();
    for case in [
        "missing_identity",
        "changed_origin",
        "untrusted_root",
        "wrong_hostname",
        "expired_server",
        "cancelled_request",
    ] {
        let run_id = format!("core-pki-{case}");
        assert!(events.iter().any(|event| event.context.run_id.as_deref()
            == Some(run_id.as_str())
            && event.event_type == "effect.outcome_unknown.v1"));
    }
    assert_eq!(
        <[u8; 32]>::from(sha2::Sha256::digest(
            std::fs::read(component.join("browser-component.json")).unwrap()
        )),
        manifest_digest
    );
    assert_eq!(
        sha2::Sha256::digest(std::fs::read(executable).unwrap()),
        executable_digest
    );
    eprintln!(
        "PASS Core Windows PKI owner/policy, native reviewed OS-held identity, rejection certainty, journal and whole native cleanup; production acceptance false"
    );
}
