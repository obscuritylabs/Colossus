//! Explicit real Windows source acceptance. Missing prerequisites fail categorically.
#![cfg(all(windows, debug_assertions))]
#[path = "browser_windows_native/actions.rs"]
mod actions;
#[path = "browser_windows_native/capture.rs"]
mod capture;
#[path = "browser_windows_native/continuity.rs"]
mod continuity;
#[path = "browser_windows_native/exercise.rs"]
mod exercise;
#[path = "browser_windows_native/fixture.rs"]
mod fixture;
#[path = "browser_windows_native/keyboard.rs"]
mod keyboard;
#[path = "browser_windows_native/logging.rs"]
mod logging;
#[path = "browser_windows_native/tls.rs"]
mod tls;
#[path = "browser_windows_native/transfer.rs"]
mod transfer;

use colossus_contracts::{
    BrowserActionKind, BrowserCapabilities, BrowserDocumentId, BrowserLimits, BrowserMode,
    BrowserOpenOptions, BrowserOrigin, BrowserScope, BrowserSessionBinding, BrowserSessionId,
    BrowserTabId, BrowserUrl,
};
use colossus_ports::BrowserDriverOpenRequest;
use colossus_sandbox::{
    BrowserEgressLimits, WindowsBrowserConfig, WindowsBrowserShutdownReceipt,
    WindowsBrowserSupervisor,
};
use colossus_windows_native::PrivateDirectoryCreation;
use sha2::{Digest as _, Sha256};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

#[tokio::test]
#[ignore = "requires native Windows x64 MSVC, pinned source host, AppContainer/WFP privileges and owner-private state"]
async fn owned_windows_chromium_frames_input_viewer_and_full_cleanup() {
    run(false).await;
}

#[cfg(debug_assertions)]
#[tokio::test]
#[ignore = "requires the pinned Windows source host and an owner-private state parent; no administrator token or outer containment"]
async fn user_windows_chromium_automation_frames_input_transfers_and_cleanup() {
    run(true).await;
}

async fn run(user_development: bool) {
    let component = PathBuf::from(
        std::env::var_os("COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_COMPONENT")
            .expect("select the explicit false-mode Windows native host stage"),
    );
    let parent = PathBuf::from(
        std::env::var_os("COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_STATE_PARENT")
            .expect("select an existing owner-private native state parent"),
    );
    let manifest =
        std::fs::read(component.join("browser-component.json")).expect("source inventory required");
    assert!(manifest.len() <= 8 * 1024 * 1024, "inventory byte bound");
    let metadata: serde_json::Value =
        serde_json::from_slice(&manifest).expect("source inventory shape");
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).expect("native fixture nonce");
    let mut state =
        PrivateDirectoryCreation::create(&parent.join(format!("win-{}", hex::encode(nonce))))
            .expect("owner-private exact state allocation");
    let state_path = state.path().to_owned();
    let long_profile = std::env::var_os("COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_LONG_PROFILE")
        .is_some_and(|value| value == "1");
    let state_binding = colossus_windows_native::BoundPath::open_directory(&state_path)
        .expect("native state root binding");
    state_binding
        .validate_private_owner_dacl()
        .expect("native state root DACL");
    state_binding
        .validate_ancestor_namespace_authority()
        .expect("native state root protected ancestry");
    let component_binding = colossus_windows_native::BoundPath::open_directory(&component)
        .expect("native component root binding");
    component_binding
        .validate_immutable_directory_dacl()
        .expect("native component payload and ancestry DACL");
    drop(component_binding);
    let mut engine_state = long_profile.then(|| {
        PrivateDirectoryCreation::create_in_current_user_profile(std::ffi::OsStr::new(&format!(
            "ColossusBrowserPreview-{}",
            hex::encode(nonce)
        )))
        .expect("same exclusive native cache allocation as the functional preview")
    });
    let engine_state_path = engine_state
        .as_ref()
        .map_or_else(|| state_path.clone(), |state| state.path().to_owned());
    let fixture = fixture::Fixture::start().await;
    let tls_fixture = tls::Fixture::start().await;
    let request = BrowserDriverOpenRequest {
        binding: BrowserSessionBinding {
            runtime_id: "windows-native-runtime".into(),
            workspace_id: "windows-native-workspace".into(),
            application_id: "windows-native-application".into(),
            scope: BrowserScope::Conversation {
                id: "windows-native-conversation".into(),
            },
        },
        run_id: None,
        session_id: BrowserSessionId::parse(format!("bs_{}", hex::encode(nonce))).unwrap(),
        tab_id: BrowserTabId::parse(format!("bt_{}", hex::encode(nonce))).unwrap(),
        document_id: BrowserDocumentId::parse(format!("bd_{}", hex::encode(nonce))).unwrap(),
        options: BrowserOpenOptions {
            public_web: false,
            profile: Default::default(),
            mode: BrowserMode::Embedded,
            allowed_origins: vec![
                BrowserOrigin::parse(&fixture.origin).unwrap(),
                BrowserOrigin::parse(&tls_fixture.target.origin).unwrap(),
            ],
            initial_url: Some(BrowserUrl::parse(format!("{}/fixture", fixture.origin)).unwrap()),
        },
    };
    let config = WindowsBrowserConfig {
        component_root: component,
        manifest_sha256: Sha256::digest(&manifest).into(),
        state_root: engine_state_path.clone(),
        capabilities: BrowserCapabilities {
            available: true,
            engine_version: Some(
                metadata["cef_version"]
                    .as_str()
                    .expect("exact CEF version")
                    .into(),
            ),
            modes: vec![BrowserMode::Embedded],
            actions: vec![
                BrowserActionKind::Navigate,
                BrowserActionKind::Snapshot,
                BrowserActionKind::Screenshot,
                BrowserActionKind::Click,
                BrowserActionKind::Upload,
                BrowserActionKind::Download,
                BrowserActionKind::Fill,
                BrowserActionKind::Select,
                BrowserActionKind::Press,
                BrowserActionKind::Scroll,
                BrowserActionKind::Wait,
                BrowserActionKind::Back,
                BrowserActionKind::Forward,
                BrowserActionKind::Reload,
                BrowserActionKind::Stop,
                BrowserActionKind::TabOpen,
                BrowserActionKind::TabSelect,
                BrowserActionKind::TabClose,
            ],
            limits: BrowserLimits::default(),
            private_ca_trust: false,
            client_identities: false,
            restrictive_egress: !user_development,
        },
        memory_bytes: 1024 * 1024 * 1024,
        max_processes: 64,
        egress: BrowserEgressLimits {
            max_connections: 16,
            lifetime: Duration::from_secs(120),
            connection_timeout: Duration::from_secs(10),
            max_connection_bytes: 1024 * 1024,
        },
    };
    let installed = if user_development {
        #[cfg(debug_assertions)]
        {
            WindowsBrowserSupervisor::install_for_development(config).await
        }
        #[cfg(not(debug_assertions))]
        {
            panic!("standard-user browser development is Debug-only");
        }
    } else {
        WindowsBrowserSupervisor::install_for_acceptance(config).await
    };
    let supervisor = Arc::new(
        installed.expect("native diagnostic installation prerequisite; no production acceptance"),
    );
    let runner = Arc::clone(&supervisor);
    let denied = fixture.denied.clone();
    let mut check = tokio::spawn(exercise::run(
        runner,
        request,
        exercise::Fixtures {
            denied,
            origin: fixture.origin.clone(),
            uploaded: Arc::clone(&fixture.uploaded),
            requests: Arc::clone(&fixture.allowed_requests),
            pulse_requests: Arc::clone(&fixture.pulse_requests),
            tls: tls_fixture.target.clone(),
            long_profile_layout: long_profile.then_some(logging::Layout {
                managed_root: state_path.clone(),
                engine_root: engine_state_path.clone(),
            }),
        },
    ));
    let result = tokio::time::timeout(Duration::from_secs(90), &mut check).await;
    if result.is_err() {
        check.abort();
        let _ = check.await;
    }
    // The independent supervisor survives every fixture panic or cancelled task.
    let cleanup = tokio::time::timeout(Duration::from_secs(90), supervisor.shutdown()).await;
    let receipts = supervisor.shutdown_receipts().await;
    let allowed = fixture.allowed_requests.load(Ordering::SeqCst);
    let denied = fixture.denied_requests.load(Ordering::SeqCst);
    let uploaded = fixture.uploaded.load(Ordering::SeqCst);
    fixture.finish().await;
    tls_fixture.finish().await;
    assert!(
        matches!(cleanup, Ok(Ok(()))),
        "full native cleanup unknown; preserve exact private state obligation"
    );
    assert_eq!(
        supervisor.shutdown().await,
        Ok(()),
        "shutdown retry idempotent"
    );
    drop(supervisor);
    if let Some(engine_state) = &mut engine_state {
        engine_state
            .remove_empty()
            .expect("positive short native cache removal after full cleanup");
        assert!(!engine_state_path.exists(), "short native cache gone");
    }
    drop(state_binding);
    state
        .remove_empty()
        .expect("positive exact state-root removal after process/filter/pipe/profile retirement");
    assert!(!state_path.exists(), "owned native state gone");
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        other => panic!("native Windows fixture did not finish: {other:?}"),
    }
    assert!(
        allowed > 0,
        "localhost DNS destination must reach fixture through authenticated proxy"
    );
    assert_eq!(
        denied, 0,
        "foreign loopback subresource must never reach fixture"
    );
    assert_eq!(
        uploaded, 1,
        "actual multipart upload must prove exact native staged bytes"
    );
    assert_eq!(
        receipts.len(),
        1,
        "exactly one independently owned native host"
    );
    assert_eq!(
        receipts[0].1,
        Some(WindowsBrowserShutdownReceipt::GracefulCefShutdown),
        "forced Job cleanup cannot masquerade as graceful CEF shutdown"
    );
    eprintln!(
        "{}",
        serde_json::json!({"native_windows_factory":true,
        "standard_user_development":user_development,
        "low_appcontainer_token":!user_development,
        "low_package_profile_label":!user_development,
        "authenticated_bgra":true,"native_input":true,"read_only_viewer_detach":true,
        "exact_origin_denial":true,"native_upload_actual_http_bytes":true,
        "native_download_actual_http_bytes":true,"full_job_and_io_cleanup":true,
        "native_typed_automation":true,"native_tabs":true,
        "suspended_session_continuity":true,
        "native_untrusted_tls_rejected_with_safe_diagnostic":true,
        "long_managed_path_short_private_profile_without_file_logging":long_profile,
        "cef_shutdown":true,"whole_host_network_acceptance":false,"production_containment":false})
    );
}
