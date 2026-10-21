//! Real system-store PKI through Core's authenticated owner and effect gateway.
use super::*;
use colossus_browser_presentation::{ClientIdentityKey, ClientIdentityPolicy};
use colossus_windows_native::{BoundPath, PrivateDirectoryCreation};
use std::collections::BTreeMap;

#[derive(serde::Deserialize)]
struct Fixture {
    urls: BTreeMap<String, String>,
    fingerprints_sha256: BTreeMap<String, String>,
}

/// Act only through Runtime's enrolled native admission authority, independently
/// of an in-flight browser effect or a not-yet-created viewer/Opened result.
async fn reviewed<T>(
    runtime: &Runtime,
    authority: &crate::RuntimeNativeBrowserAuthority,
    fixture: &Fixture,
    case: &str,
    control: &RunControl,
    future: impl std::future::Future<Output = T>,
) -> T {
    tokio::pin!(future);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(55);
    let mut submitted: Vec<ClientIdentityKey> = Vec::new();
    loop {
        tokio::select! {
            result = &mut future => return result,
            () = tokio::time::sleep_until(deadline) => panic!("Core native PKI operation exceeded its original bound"),
            () = tokio::time::sleep(Duration::from_millis(20)) => {
                let policy = if case == "unique_initial_identity" {
                    ClientIdentityPolicy::AutomaticIfUnique
                } else { ClientIdentityPolicy::Review };
                let Some(review) = runtime.pending_native_browser_client_identity(authority, policy)
                    .await.expect("Core native review discovery/automatic submission acknowledged") else { continue; };
                assert_ne!(case, "unique_initial_identity", "the sole initial identity must require no picker");
                assert!(submitted.len() < 8 && !submitted.contains(&review.challenge.key),
                    "one-use native choice must never be replayed");
                assert!(review.challenge.candidates.len() >= 2,
                    "multiple actual eligible OS-held identities must reach review");
                let origin = review.challenge.origin.as_str();
                let fingerprint = match case {
                    "missing_identity" => None,
                    "alternate_identity" => Some(fixture.fingerprints_sha256["alternate_client"].clone()),
                    "exact_identity" => Some(fixture.fingerprints_sha256["client"].clone()),
                    "changed_origin" if origin == fixture.urls["mtls"] =>
                        Some(fixture.fingerprints_sha256["client"].clone()),
                    "changed_origin" if origin == fixture.urls["redirect_mtls"] => None,
                    "cancelled_request" => {
                        control.cancel();
                        submitted.push(review.challenge.key);
                        continue;
                    }
                    _ => panic!("unexpected Core client-key challenge in fixture case"),
                };
                assert!(origin == fixture.urls["mtls"] || origin == fixture.urls["redirect_mtls"]);
                runtime.select_native_browser_client_identity(authority, &review.session,
                    review.challenge.key, fingerprint).await.expect("one exact native choice acknowledged");
                submitted.push(review.challenge.key);
            }
        }
    }
}

async fn alternate_fixture(directory: &std::path::Path) -> Fixture {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let ready = directory.join("alternate-client-ready.json");
    while !ready.exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "alternate fixture identity was not provisioned"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(std::fs::metadata(&ready).unwrap().len() <= 64 * 1024);
    let bytes = std::fs::read(ready).unwrap();
    assert!(bytes.len() <= 64 * 1024);
    let fixture: Fixture = serde_json::from_slice(&bytes).unwrap();
    assert!(fixture.fingerprints_sha256.contains_key("alternate_client"));
    fixture
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
#[ignore = "requires generated fixture-only CurrentUser ROOT/MY+CNG keys, pinned CEF component and native Runtime review bridge"]
async fn core_authenticates_system_ca_and_reviewed_client_identity_after_policy() {
    let directory = PathBuf::from(
        std::env::var_os("COLOSSUS_WINDOWS_PKI_FIXTURE").expect("explicit owned fixture"),
    );
    let source = BoundPath::open_directory(&directory).unwrap();
    source.validate_private_owner_dacl().unwrap();
    let encoded = std::fs::read(directory.join("fixture.json")).unwrap();
    assert!(encoded.len() <= 64 * 1024);
    let mut fixture: Fixture = serde_json::from_slice(&encoded).unwrap();
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
                    RuntimeBrowserHost::from_supervised_factory(captured.clone())
                        .unwrap()
                        .with_presenter(captured.clone()),
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
        let authority = runtime
            .bind_native_browser_authority("authenticated-pki-app", Uuid::now_v7())
            .unwrap();
        assert!(
            runtime.native_browser_available(&authority),
            "live fixture must compose the private presenter before opening TLS"
        );
        for (name, endpoint, route, expected) in [
            (
                "unique_initial_identity",
                "mtls",
                "/client-check",
                Some("Colossus mTLS identity verified"),
            ),
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
            let mut open_arguments = json!({"mode":"embedded",
                "allowed_origins":fixture.urls.values().collect::<Vec<_>>()});
            if name == "unique_initial_identity" {
                open_arguments["initial_url"] =
                    format!("{}{}", fixture.urls[endpoint], route).into();
            }
            let opened = reviewed(
                runtime,
                &authority,
                &fixture,
                name,
                &control,
                call(&executor, &context, "browser.open", open_arguments),
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
            let result = if name == "unique_initial_identity" {
                assert_eq!(
                    request.options.initial_url.as_ref().unwrap().origin(),
                    BrowserUrl::parse(format!("{}{}", fixture.urls[endpoint], route))
                        .unwrap()
                        .origin()
                );
                assert_eq!(
                    runtime
                        .browser
                        .as_ref()
                        .unwrap()
                        .native
                        .identity_attempts
                        .lock()
                        .unwrap()
                        .get(&request.session_id)
                        .unwrap()
                        .control_generation,
                    0,
                    "automatic exact choice must precede first writer grant and Opened"
                );
                Ok(serde_json::to_value(blank).unwrap())
            } else {
                let mut arguments = observed_target(&opened, &blank);
                arguments["url"] = format!("{}{}", fixture.urls[endpoint], route).into();
                reviewed(
                    runtime,
                    &authority,
                    &fixture,
                    name,
                    &control,
                    call(&executor, &context, "browser.navigate", arguments),
                )
                .await
            };
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
            if name == "unique_initial_identity" {
                eprintln!("EVIDENCE windows-pki stage=provision_alternate");
                let next = alternate_fixture(&directory).await;
                assert_eq!(
                    next.urls, fixture.urls,
                    "alternate provisioning cannot change the admitted origins"
                );
                assert_eq!(
                    next.fingerprints_sha256["client"],
                    fixture.fingerprints_sha256["client"]
                );
                fixture = next;
            }
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
    assert_eq!(acknowledged.load(Ordering::SeqCst), 10);
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
