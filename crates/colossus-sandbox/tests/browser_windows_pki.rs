//! Explicit real Windows CEF system-store TLS and native-consent client-key acceptance.
#![cfg(all(windows, debug_assertions))]

use colossus_browser_bridge::BrowserHostFactory;
use colossus_contracts::{
    BrowserAction, BrowserActionKind, BrowserCapabilities, BrowserDocumentId, BrowserLimits,
    BrowserMode, BrowserOpenOptions, BrowserOrigin, BrowserScope, BrowserSessionBinding,
    BrowserSessionId, BrowserTabId, BrowserTarget, BrowserUrl,
};
use colossus_ports::{
    BrowserDriverCommand, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
    RunControl,
};
use colossus_sandbox::{BrowserEgressLimits, WindowsBrowserConfig, WindowsBrowserSupervisor};
use colossus_windows_native::{BoundPath, PrivateDirectoryCreation};
use sha2::{Digest as _, Sha256};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

#[derive(serde::Deserialize)]
struct Fixture {
    urls: BTreeMap<String, String>,
}

#[tokio::test]
#[ignore = "requires generated fixture-only CurrentUser ROOT/MY+CNG keys, pinned CEF component and exact native dialog driver"]
async fn system_ca_and_reviewed_client_identity_authenticate_through_real_cef() {
    let directory = PathBuf::from(
        std::env::var_os("COLOSSUS_WINDOWS_PKI_FIXTURE").expect("explicit owned fixture"),
    );
    let source = BoundPath::open_directory(&directory).unwrap();
    source.validate_private_owner_dacl().unwrap();
    let fixture: Fixture =
        serde_json::from_slice(&std::fs::read(directory.join("fixture.json")).unwrap()).unwrap();
    let component = PathBuf::from(
        std::env::var_os("COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_COMPONENT")
            .expect("verified CEF stage"),
    );
    let parent = PathBuf::from(
        std::env::var_os("COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_STATE_PARENT")
            .expect("owner-private state parent"),
    );
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).unwrap();
    let mut state =
        PrivateDirectoryCreation::create(&parent.join(format!("pki-{}", hex::encode(nonce))))
            .unwrap();
    let manifest = std::fs::read(component.join("browser-component.json")).unwrap();
    let supervisor = Arc::new(
        WindowsBrowserSupervisor::install_for_development(WindowsBrowserConfig {
            component_root: component,
            manifest_sha256: Sha256::digest(&manifest).into(),
            state_root: state.path().to_owned(),
            capabilities: BrowserCapabilities {
                available: true,
                engine_version: Some("154.0.34+g14c5a08+chromium-154.0.8037.98".into()),
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
    let runner = Arc::clone(&supervisor);
    let path = directory.clone();
    let mut work = tokio::spawn(async move {
        for (index, (name, endpoint, route, expected)) in [
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
        ]
        .into_iter()
        .enumerate()
        {
            eprintln!("EVIDENCE windows-pki case={name} stage=begin");
            let session = BrowserSessionId::parse(format!("bs_{index:032x}")).unwrap();
            let request = BrowserDriverOpenRequest {
                binding: BrowserSessionBinding {
                    runtime_id: "pki-runtime".into(),
                    workspace_id: "pki-workspace".into(),
                    application_id: "pki-app".into(),
                    scope: BrowserScope::Conversation {
                        id: format!("pki-{index}"),
                    },
                },
                run_id: Some("pki-run".into()),
                session_id: session.clone(),
                tab_id: BrowserTabId::parse(format!("bt_{index:032x}")).unwrap(),
                document_id: BrowserDocumentId::parse(format!("bd_{index:032x}")).unwrap(),
                options: BrowserOpenOptions {
                    mode: BrowserMode::Embedded,
                    profile: Default::default(),
                    public_web: false,
                    allowed_origins: fixture
                        .urls
                        .values()
                        .map(|origin| BrowserOrigin::parse(origin).unwrap())
                        .collect(),
                    initial_url: None,
                },
            };
            let run = RunControl::default();
            let control = BrowserDriverControl::new(run.clone(), RunControl::default());
            let driver = runner.launch(&request, &control).await.unwrap();
            let tab = driver
                .open_session(request.clone(), &control)
                .await
                .unwrap();
            let command = BrowserDriverCommand {
                binding: request.binding.clone(),
                run_id: "pki-run".into(),
                session_id: session.clone(),
                target: BrowserTarget {
                    tab_id: tab.tab_id,
                    document_id: tab.document_id,
                },
                control_generation: 1,
                action: BrowserAction::Navigate {
                    url: BrowserUrl::parse(format!("{}{}", fixture.urls[endpoint], route)).unwrap(),
                },
                next_document_id: BrowserDocumentId::parse(format!("bd_{:032x}", index + 100))
                    .unwrap(),
                snapshot_id: None,
                new_tab: None,
            };
            let mut navigation = {
                let driver = Arc::clone(&driver);
                let control = control.clone();
                tokio::spawn(async move { driver.execute(command, &control).await })
            };
            if name == "cancelled_request" {
                let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
                while !path.join("cancelled-request-dialog-seen").exists() {
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "real native prompt required before cancellation"
                    );
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                run.cancel();
            }
            let result = tokio::time::timeout(Duration::from_secs(55), &mut navigation)
                .await
                .unwrap()
                .unwrap();
            if let Some(title) = expected {
                assert_eq!(
                    result.unwrap().tab.title,
                    title,
                    "exact server identity/login title"
                );
            } else {
                assert!(
                    matches!(result, Err(BrowserDriverError::OutcomeUnknown)),
                    "rejected TLS/native identity cannot become success"
                );
            }
            driver.close_session(&session).await.unwrap();
            eprintln!("PASS windows-pki case={name}");
        }
    });
    let result = tokio::time::timeout(Duration::from_secs(300), &mut work).await;
    if result.is_err() {
        work.abort();
        let _ = work.await;
    }
    supervisor
        .shutdown()
        .await
        .expect("independent whole native cleanup");
    let receipts = supervisor.shutdown_receipts().await.len();
    drop(supervisor);
    state.remove_empty().expect("all owned profiles removed");
    match result {
        // Closed allocation metadata is pruned by the next launch. Each close
        // above was acknowledged; the final allocation retains its receipt.
        Ok(Ok(())) => assert_eq!(receipts, 1),
        Ok(Err(error)) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        _ => panic!("Windows PKI native fixture did not finish"),
    }
}
