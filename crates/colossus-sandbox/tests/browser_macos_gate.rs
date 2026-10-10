//! macOS owned-browser composition remains fail-closed until native acceptance.

#![cfg(target_os = "macos")]

use colossus_browser_bridge::BrowserHostFactory as _;
use colossus_contracts::{
    BrowserDocumentId, BrowserMode, BrowserOpenOptions, BrowserScope, BrowserSessionBinding,
    BrowserSessionId, BrowserTabId,
};
use colossus_ports::{
    BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest, RunControl,
};
use colossus_sandbox::{MacosBrowserRequirement, MacosBrowserSupervisor};

#[test]
fn current_macos_supervisor_cannot_advertise_browser_authority() {
    let supervisor = MacosBrowserSupervisor::unavailable();

    assert!(!supervisor.readiness().is_available());
    assert_eq!(
        supervisor.readiness().blockers(),
        &[
            MacosBrowserRequirement::WholeProcessTreeOwner,
            MacosBrowserRequirement::InheritedNetworkContainment,
            MacosBrowserRequirement::RequestOriginEnforcement,
            MacosBrowserRequirement::OwnedCertificateCustody,
            MacosBrowserRequirement::NativePresentationAcceptance,
            MacosBrowserRequirement::SignedDistributionAcceptance,
        ]
    );
    assert_eq!(
        supervisor.capabilities(),
        colossus_contracts::BrowserCapabilities::unavailable()
    );
}

#[tokio::test]
async fn current_macos_supervisor_rejects_launch_without_allocating_cleanup_work() {
    let supervisor = MacosBrowserSupervisor::unavailable();
    let request = BrowserDriverOpenRequest {
        binding: BrowserSessionBinding {
            runtime_id: "macos-gate-runtime".into(),
            workspace_id: "macos-gate-workspace".into(),
            application_id: "macos-gate-application".into(),
            scope: BrowserScope::Conversation {
                id: "macos-gate-conversation".into(),
            },
        },
        run_id: Some("macos-gate-run".into()),
        session_id: BrowserSessionId::parse(format!("bs_{}", "1".repeat(32))).unwrap(),
        tab_id: BrowserTabId::parse(format!("bt_{}", "2".repeat(32))).unwrap(),
        document_id: BrowserDocumentId::parse(format!("bd_{}", "3".repeat(32))).unwrap(),
        options: BrowserOpenOptions {
            profile: Default::default(),
            mode: BrowserMode::Embedded,
            allowed_origins: Vec::new(),
            initial_url: None,
        },
    };
    let control = BrowserDriverControl::new(RunControl::default(), RunControl::default());

    assert!(matches!(
        supervisor.launch(&request, &control).await,
        Err(BrowserDriverError::Unavailable)
    ));
    supervisor
        .reap_failed_launch(&request)
        .await
        .expect("unavailable gate never allocated a native obligation");
}
