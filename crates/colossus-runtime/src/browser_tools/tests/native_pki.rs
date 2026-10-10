//! Actual native per-request PKI through Runtime's effect gateway, never default OS stores.
use super::*;
use colossus_browser_bridge::BrowserHostPool;
use colossus_sandbox::{
    OciBrowserConfig, OciBrowserIdentity, OciBrowserLimits, OciBrowserPki,
    OciBrowserPkiAuthorization, OciBrowserPkiEnrollmentProvider, OciBrowserPkiRegistration,
    OciBrowserPkiRegistry, OciBrowserPkiScopeAuthorization, OciBrowserShutdownReceipt,
    OciBrowserSupervisor, OciClientIdentityBinding,
};
use sha2::{Digest as _, Sha256};
use std::time::Duration;
mod exercise;
mod fixture;

struct Provider {
    registry: OciBrowserPkiRegistry,
    observed: Arc<StdMutex<Vec<BrowserSessionBinding>>>,
}
impl OciBrowserPkiEnrollmentProvider for Provider {
    fn has_private_ca(&self) -> bool {
        self.registry.has_private_ca()
    }
    fn has_client_identities(&self) -> bool {
        self.registry.has_client_identities()
    }
    fn policy_digest(&self) -> [u8; 32] {
        self.registry.policy_digest()
    }
    fn enroll(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<Option<OciBrowserPki>, BrowserDriverError> {
        self.observed.lock().unwrap().push(request.binding.clone());
        self.registry.enroll(request)
    }
}
fn registration(
    fixture: &fixture::Fixture,
    workspace: &std::path::Path,
    scope: OciBrowserPkiScopeAuthorization,
    identity: &str,
) -> OciBrowserPkiRegistration {
    let password = fixture.bytes("passphrase.txt", 128);
    OciBrowserPkiRegistration::new(
        OciBrowserPkiAuthorization {
            workspace_id: crate::runtime_helpers::repository_identity(workspace),
            application_id: "authenticated-app".into(),
            scope,
            origins: fixture.metadata["urls"]
                .as_object()
                .unwrap()
                .values()
                .map(|value| BrowserOrigin::parse(value.as_str().unwrap()).unwrap())
                .collect(),
        },
        vec![fixture.bytes("ca.der", 256 * 1024).to_vec()],
        vec![
            OciBrowserIdentity::new(
                fixture.bytes(&format!("{identity}.pfx"), 4 * 1024 * 1024),
                HostSecret::new(std::str::from_utf8(&password).unwrap()).unwrap(),
            )
            .unwrap(),
        ],
        vec![OciClientIdentityBinding {
            origin: BrowserOrigin::parse(fixture.url("mtls")).unwrap(),
            fingerprint_sha256: fixture.fingerprint(identity),
        }],
    )
    .unwrap()
}

#[tokio::test]
#[ignore = "requires fresh linked CEF host, exact pinned NSS OCI image and native Docker"]
async fn contained_chromium_enrolls_native_pki_for_the_actual_core_owner_after_policy() {
    let component = PathBuf::from(
        std::env::var_os("COLOSSUS_BROWSER_OCI_ACCEPTANCE_COMPONENT")
            .expect("select the explicit fresh inventoried native stage"),
    );
    let image = std::env::var("COLOSSUS_BROWSER_OCI_ACCEPTANCE_IMAGE_ID")
        .expect("select the immutable image with all nine exact NSS tool/library pins");
    let manifest = std::fs::read(component.join("browser-component.json")).unwrap();
    assert!(manifest.len() <= 8 * 1024 * 1024);
    let metadata: Value = serde_json::from_slice(&manifest).unwrap();
    let digest: [u8; 32] = Sha256::digest(&manifest).into();
    let host_digest =
        Sha256::digest(std::fs::read(component.join("colossus-native-browser-host")).unwrap());
    let selected_image = image.clone();
    let workspace = private_tempdir();
    let state = private_tempdir().keep();
    let mut fixture = fixture::Fixture::start().await;
    let observed = Arc::new(StdMutex::new(Vec::new()));
    let provider = Arc::new(Provider {
        registry: OciBrowserPkiRegistry::new(vec![
            registration(
                &fixture,
                workspace.path(),
                OciBrowserPkiScopeAuthorization::FutureConversations,
                "client",
            ),
            registration(
                &fixture,
                workspace.path(),
                OciBrowserPkiScopeAuthorization::FutureWorkflows,
                "alternate_client",
            ),
        ])
        .unwrap(),
        observed: observed.clone(),
    });
    assert!(observed.lock().unwrap().is_empty());
    let capabilities = BrowserCapabilities {
        available: true,
        engine_version: Some(metadata["cef_version"].as_str().unwrap().into()),
        modes: vec![BrowserMode::Headless],
        actions: vec![BrowserActionKind::Snapshot],
        limits: BrowserLimits::default(),
        private_ca_trust: true,
        client_identities: true,
        restrictive_egress: true,
    };
    let supervisor = Arc::new(
        OciBrowserSupervisor::install_for_acceptance(OciBrowserConfig {
            docker: std::env::var_os("COLOSSUS_BROWSER_OCI_ACCEPTANCE_DOCKER")
                .map_or_else(|| PathBuf::from("/usr/bin/docker"), PathBuf::from),
            image,
            component_root: component.clone(),
            manifest_sha256: digest,
            state_root: state.clone(),
            limits: OciBrowserLimits::default(),
            capabilities,
            pki: None,
            pki_enrollment: Some(provider),
            profile_store: None,
            presentation: false,
        })
        .await
        .unwrap(),
    );
    let pool = Arc::new(BrowserHostPool::new(supervisor.clone()).unwrap());
    let mut configuration = config(workspace.path());
    configuration.sandbox.network_destinations = fixture.metadata["urls"]
        .as_object()
        .unwrap()
        .values()
        .map(|value| value.as_str().unwrap().into())
        .collect();
    // Native PKI must reject this mixed envelope even when ordinary runtime
    // network policy permits it. It must fail before native process allocation.
    configuration
        .sandbox
        .network_destinations
        .push("https://unapproved.invalid".into());
    let runtime = Arc::new(
        Runtime::open_with_options(
            &configuration,
            Arc::new(DenyApproval),
            None,
            RuntimeOpenOptions::for_workspace(workspace.path())
                .unwrap()
                .with_browser_host(RuntimeBrowserHost::new(pool.clone())),
        )
        .unwrap(),
    );
    assert!(
        observed.lock().unwrap().is_empty(),
        "no runtime binding was nominated before Core minted it"
    );
    let task_runtime = runtime.clone();
    let task_pool = pool.clone();
    let task_metadata = fixture.metadata.clone();
    let task_observed = observed.clone();
    let acknowledged = Arc::new(AtomicUsize::new(0));
    let task_acknowledged = acknowledged.clone();
    let task_supervisor = supervisor.clone();
    let mut task = tokio::spawn(async move {
        exercise::run(
            &task_runtime,
            &task_pool,
            &task_metadata,
            &task_observed,
            &task_supervisor,
            &task_acknowledged,
        )
        .await
    });
    let check = tokio::time::timeout(Duration::from_secs(600), &mut task).await;
    if check.is_err() {
        task.abort();
        let _ = task.await;
    }
    // Keep the supervisor outside the policy task: its retained cleanup owner
    // must survive a failed operation, panic or test timeout.
    let cleanup = tokio::time::timeout(Duration::from_secs(90), supervisor.shutdown()).await;
    let stages = supervisor.acceptance_launch_stages().await;
    let report = fixture.report().await;
    let fixture_metadata = fixture.metadata.clone();
    fixture.close(matches!(cleanup, Ok(Ok(())))).await;
    assert!(
        matches!(cleanup, Ok(Ok(()))),
        "native cleanup unknown; exact private state retained"
    );
    assert!(
        stages
            .iter()
            .all(|stage| stage.1 && stage.2 == OciBrowserShutdownReceipt::GracefulCefShutdown)
    );
    match check {
        Ok(Ok(())) => {}
        Ok(Err(error)) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        _ => panic!("native Runtime PKI acceptance did not finish"),
    }
    exercise::server_receipts(&report, &fixture_metadata);
    assert_eq!(acknowledged.load(Ordering::SeqCst), 6);
    let bindings = observed.lock().unwrap();
    assert_eq!(bindings.len(), 7);
    assert!(bindings.iter().all(|binding| binding.runtime_id
        == runtime.browser.as_ref().unwrap().runtime_id
        && binding.application_id == "authenticated-app"));
    assert_eq!(
        <[u8; 32]>::from(Sha256::digest(
            std::fs::read(component.join("browser-component.json")).unwrap(),
        )),
        digest,
        "component inventory changed during native acceptance"
    );
    assert_eq!(
        Sha256::digest(std::fs::read(component.join("colossus-native-browser-host")).unwrap()),
        host_digest,
        "native executable changed during acceptance"
    );
    if let Some(path) = std::env::var_os("COLOSSUS_BROWSER_OCI_PKI_ACCEPTANCE_RECEIPT") {
        use std::{fs::OpenOptions, io::Write as _, os::unix::fs::OpenOptionsExt as _};
        let receipt = json!({"schema_version":1,"runtime_native_pki_conformance":true,
            "host_sha256":hex::encode(host_digest), "selected_oci_image":selected_image,
            "component_manifest_sha256":hex::encode(digest), "cases":6,"graceful_shutdown_acknowledgments":6,
            "exact_enrollments":&*bindings,
            "server_receipts":report, "owned_native_cleanup_verified":true,"production_containment":false,
            "broker_private_key_custody_verified":false,"signed_installed_artifact_verified":false});
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        output
            .write_all(&serde_json::to_vec_pretty(&receipt).unwrap())
            .unwrap();
        output.sync_all().unwrap();
    }
    drop(bindings);
    drop(runtime);
    drop(supervisor);
    std::fs::remove_dir_all(state).unwrap();
}
