//! Explicit real Windows CEF system-store TLS and private native-owner review.
#![cfg(all(windows, debug_assertions))]

use colossus_browser_bridge::BrowserHostFactory;
use colossus_browser_presentation::{ClientIdentityKey, ClientIdentityPolicy, PresentationClient};
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
    fingerprints_sha256: BTreeMap<String, String>,
}

async fn settle_review(
    operation: &mut tokio::task::JoinHandle<Result<String, BrowserDriverError>>,
    presentation: &PresentationClient,
    fixture: &Fixture,
    name: &str,
    run: &RunControl,
) -> Result<String, BrowserDriverError> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(55);
    let mut submitted = Vec::<ClientIdentityKey>::new();
    while !operation.is_finished() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "bounded native review"
        );
        // Discover the exact current request first. An earlier policy poll must
        // never become permission to use an identity in a future handshake.
        if let Some(review) = presentation
            .pending_client_identity(ClientIdentityPolicy::Review)
            .await
            .unwrap()
        {
            review.validate().unwrap();
            assert_eq!(
                review.key.control_generation,
                u64::from(name != "unique_initial_identity")
            );
            assert!(
                review
                    .candidates
                    .iter()
                    .all(|candidate| ["client", "alternate_client"]
                        .into_iter()
                        .filter_map(|label| fixture.fingerprints_sha256.get(label))
                        .any(|fingerprint| fingerprint == &candidate.fingerprint_sha256)),
                "only exact generated fixture identities may be considered"
            );
            if !submitted.contains(&review.key) {
                submitted.push(review.key);
                match name {
                    "unique_initial_identity" => {
                        assert_eq!(review.origin.as_str(), fixture.urls["mtls"]);
                        assert_eq!(review.candidates.len(), 1);
                        assert_eq!(
                            review.candidates[0].fingerprint_sha256,
                            fixture.fingerprints_sha256["client"]
                        );
                        assert!(
                            presentation
                                .pending_client_identity(ClientIdentityPolicy::AutomaticIfUnique)
                                .await
                                .unwrap()
                                .is_none()
                        );
                    }
                    "cancelled_request" => run.cancel(),
                    _ => {
                        let fingerprint = if name == "missing_identity"
                            || (name == "changed_origin"
                                && review.origin.as_str() == fixture.urls["redirect_mtls"])
                        {
                            None
                        } else {
                            assert_eq!(review.origin.as_str(), fixture.urls["mtls"]);
                            assert_eq!(
                                review.candidates.len(),
                                2,
                                "ambiguous identities must reach explicit native review"
                            );
                            let label = if name == "alternate_identity" {
                                "alternate_client"
                            } else {
                                "client"
                            };
                            let fingerprint = fixture.fingerprints_sha256[label].clone();
                            assert!(
                                review
                                    .candidates
                                    .iter()
                                    .any(|candidate| candidate.fingerprint_sha256 == fingerprint)
                            );
                            Some(fingerprint)
                        };
                        // ACK is only submission. Never retry an uncertain write;
                        // authentication is checked separately at the TLS server.
                        presentation
                            .select_client_identity(review.key, fingerprint)
                            .await
                            .unwrap();
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    if matches!(
        name,
        "unique_initial_identity"
            | "missing_identity"
            | "alternate_identity"
            | "exact_identity"
            | "changed_origin"
            | "cancelled_request"
    ) {
        assert!(
            !submitted.is_empty(),
            "a real pending native request must be observed"
        );
    }
    operation.await.unwrap()
}

#[tokio::test]
#[ignore = "requires generated fixture-only CurrentUser ROOT/MY+CNG keys, pinned CEF component and private native review channel"]
async fn system_ca_and_reviewed_client_identity_authenticate_through_real_cef() {
    let directory = PathBuf::from(
        std::env::var_os("COLOSSUS_WINDOWS_PKI_FIXTURE").expect("explicit owned fixture"),
    );
    let source = BoundPath::open_directory(&directory).unwrap();
    source.validate_private_owner_dacl().unwrap();
    let mut fixture: Fixture =
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
        ]
        .into_iter()
        .enumerate()
        {
            if index == 1 {
                eprintln!("EVIDENCE windows-pki stage=provision_alternate");
                let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
                let ready = path.join("alternate-client-ready.json");
                while !ready.exists() {
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "bounded fixture-only identity provisioning"
                    );
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                let encoded = std::fs::read(ready).unwrap();
                assert!(encoded.len() <= 64 * 1024);
                let next: Fixture = serde_json::from_slice(&encoded).unwrap();
                assert_eq!(next.urls, fixture.urls);
                assert_eq!(
                    next.fingerprints_sha256["client"],
                    fixture.fingerprints_sha256["client"]
                );
                assert!(next.fingerprints_sha256.contains_key("alternate_client"));
                fixture = next;
            }
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
                    initial_url: (name == "unique_initial_identity").then(|| {
                        BrowserUrl::parse(format!("{}{}", fixture.urls[endpoint], route)).unwrap()
                    }),
                },
            };
            let run = RunControl::default();
            let control = BrowserDriverControl::new(run.clone(), RunControl::default());
            let driver = runner.launch(&request, &control).await.unwrap();
            let (presentation, _) = runner.presentation(&request).await.unwrap();
            let mut navigation = if name == "unique_initial_identity" {
                let driver = Arc::clone(&driver);
                let control = control.clone();
                let request = request.clone();
                // No Configure/lease/view attachment exists during this open.
                tokio::spawn(async move {
                    driver
                        .open_session(request, &control)
                        .await
                        .map(|tab| tab.title)
                })
            } else {
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
                        url: BrowserUrl::parse(format!("{}{}", fixture.urls[endpoint], route))
                            .unwrap(),
                    },
                    next_document_id: BrowserDocumentId::parse(format!("bd_{:032x}", index + 100))
                        .unwrap(),
                    snapshot_id: None,
                    new_tab: None,
                };
                {
                    let driver = Arc::clone(&driver);
                    let control = control.clone();
                    tokio::spawn(async move {
                        driver
                            .execute(command, &control)
                            .await
                            .map(|observation| observation.tab.title)
                    })
                }
            };
            let result = settle_review(&mut navigation, &presentation, &fixture, name, &run).await;
            if let Some(title) = expected {
                assert_eq!(result.unwrap(), title, "exact server identity/login title");
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
