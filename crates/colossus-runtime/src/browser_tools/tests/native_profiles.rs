//! Actual cookie restart conformance through the owned OCI host and Runtime gateway.
//! This debug fixture proves private cache behavior, not encryption or release acceptance.
use super::*;
use colossus_browser_bridge::BrowserHostPool;
use colossus_home::ConfinedRoot;
use colossus_sandbox::{
    BrowserProfileEngine, BrowserProfileStore, OciBrowserConfig, OciBrowserLimits,
    OciBrowserShutdownReceipt, OciBrowserSupervisor,
};
use sha2::{Digest as _, Sha256};
use std::time::Duration;

mod exercise;
mod fixture;
use exercise::{finish, page};

#[tokio::test]
#[ignore = "requires fresh profile-capable CEF stage, pinned OCI image and native Docker"]
async fn persistent_profile_restarts_cookie_state_and_reset_clears_it_while_temporary_does_not() {
    let component = PathBuf::from(
        std::env::var_os("COLOSSUS_BROWSER_OCI_ACCEPTANCE_COMPONENT")
            .expect("select the explicit fresh inventoried native stage"),
    );
    let image = std::env::var("COLOSSUS_BROWSER_OCI_ACCEPTANCE_IMAGE_ID")
        .expect("select the locally installed immutable OCI image");
    let manifest = std::fs::read(component.join("browser-component.json")).unwrap();
    assert!(manifest.len() <= 8 * 1024 * 1024);
    let metadata: Value = serde_json::from_slice(&manifest).unwrap();
    let digest: [u8; 32] = Sha256::digest(&manifest).into();
    let workspace = private_tempdir();
    // Retain these owned paths on unknown native cleanup. Never delete an active cache.
    let state = private_tempdir().keep();
    let profile_root = private_tempdir().keep();
    let store = Arc::new(
        BrowserProfileStore::bind(
            ConfinedRoot::bind(&profile_root).unwrap(),
            BrowserProfileEngine {
                cef_version: metadata["cef_version"].as_str().unwrap().into(),
                chromium_version: metadata["chromium_version"].as_str().unwrap().into(),
                protocol_version: metadata["protocol_version"]
                    .as_u64()
                    .unwrap()
                    .try_into()
                    .unwrap(),
            },
        )
        .unwrap(),
    );
    let (origin, observed, serving) = fixture::start().await;
    let capabilities = BrowserCapabilities {
        available: true,
        engine_version: Some(metadata["cef_version"].as_str().unwrap().into()),
        modes: vec![BrowserMode::Headless],
        actions: vec![BrowserActionKind::Snapshot],
        limits: BrowserLimits::default(),
        private_ca_trust: false,
        client_identities: false,
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
            pki_enrollment: None,
            presentation: false,
            profile_store: Some(store.clone()),
        })
        .await
        .unwrap(),
    );
    let pool = Arc::new(BrowserHostPool::new(supervisor.clone()).unwrap());
    let owned_store = store.clone();
    let owned_supervisor = supervisor.clone();
    let completed = Arc::new(AtomicUsize::new(0));
    let acknowledged = completed.clone();
    let mut check = tokio::spawn(async move {
        let mut configuration = config(workspace.path());
        configuration.sandbox.network_destinations = vec![origin.clone()];
        let runtime = Runtime::open_with_options(
            &configuration,
            Arc::new(DenyApproval),
            None,
            RuntimeOpenOptions::for_workspace(workspace.path())
                .unwrap()
                .with_browser_host(
                    RuntimeBrowserHost::new(pool.clone())
                        .with_native_profile_protection(vec![owned_store.protected_root()]),
                ),
        )
        .unwrap();
        let first = begin(&runtime, "profile-persistent-seed");
        let owner = runtime
            .browser
            .as_ref()
            .unwrap()
            .run(&first)
            .unwrap()
            .actor
            .binding;
        let profile = owned_store
            .create(&owner, "Native restart fixture")
            .unwrap();
        let choice = BrowserProfileSelection::Workspace {
            id: profile.id.clone(),
        };
        page(&runtime, &first, &origin, "/seed", &choice, "Cookie absent").await;
        // A second context may not take the same native cache while the first exists.
        let concurrent = begin(&runtime, "profile-concurrent-denied");
        assert!(call(&executor(&runtime), &concurrent, "browser.open", json!({
            "mode":"headless", "allowed_origins":[origin], "initial_url":format!("{origin}/observe"),
            "profile":choice,
        })).await.is_err());
        runtime
            .browser
            .as_ref()
            .unwrap()
            .finish_run("profile-concurrent-denied")
            .await;
        finish(
            &runtime,
            &owned_supervisor,
            &acknowledged,
            "profile-persistent-seed",
        )
        .await;
        assert_eq!(pool.owned_session_count().unwrap(), 0);

        let restarted = begin(&runtime, "profile-persistent-restarted");
        page(
            &runtime,
            &restarted,
            &origin,
            "/observe",
            &choice,
            "Cookie present",
        )
        .await;
        finish(
            &runtime,
            &owned_supervisor,
            &acknowledged,
            "profile-persistent-restarted",
        )
        .await;
        owned_store.reset(&owner, &profile.id).unwrap();
        let cleared = begin(&runtime, "profile-reset-empty");
        page(
            &runtime,
            &cleared,
            &origin,
            "/observe",
            &choice,
            "Cookie absent",
        )
        .await;
        finish(
            &runtime,
            &owned_supervisor,
            &acknowledged,
            "profile-reset-empty",
        )
        .await;

        let temporary = BrowserProfileSelection::Temporary;
        let first = begin(&runtime, "profile-temporary-seed");
        page(
            &runtime,
            &first,
            &origin,
            "/seed",
            &temporary,
            "Cookie absent",
        )
        .await;
        finish(
            &runtime,
            &owned_supervisor,
            &acknowledged,
            "profile-temporary-seed",
        )
        .await;
        let next = begin(&runtime, "profile-temporary-fresh");
        page(
            &runtime,
            &next,
            &origin,
            "/observe",
            &temporary,
            "Cookie absent",
        )
        .await;
        finish(
            &runtime,
            &owned_supervisor,
            &acknowledged,
            "profile-temporary-fresh",
        )
        .await;
        assert_eq!(pool.owned_session_count().unwrap(), 0);
        let receipts = observed.lock().unwrap();
        assert_eq!(
            receipts
                .iter()
                .filter(|(path, cookie)| path == "/observe" && *cookie)
                .count(),
            1
        );
        assert_eq!(
            receipts
                .iter()
                .filter(|(path, cookie)| path == "/observe" && !cookie)
                .count(),
            2
        );
    });
    let result = tokio::time::timeout(Duration::from_secs(300), &mut check).await;
    if result.is_err() {
        check.abort();
        let _ = check.await;
    }
    let cleanup = tokio::time::timeout(Duration::from_secs(90), supervisor.shutdown()).await;
    let receipts = supervisor.acceptance_launch_stages().await;
    serving.abort();
    let _ = serving.await;
    assert!(
        matches!(cleanup, Ok(Ok(()))),
        "native profile/process cleanup unknown; owned state retained"
    );
    assert_eq!(
        std::fs::read(component.join("browser-component.json")).unwrap(),
        manifest
    );
    drop(supervisor);
    drop(store);
    std::fs::remove_dir_all(state).unwrap();
    std::fs::remove_dir_all(profile_root).unwrap();
    match result {
        Ok(Ok(())) => {
            let actual: Vec<_> = receipts
                .iter()
                .filter(|(_, _, receipt)| *receipt != OciBrowserShutdownReceipt::NoProcess)
                .collect();
            assert_eq!(completed.load(Ordering::SeqCst), 5);
            assert_eq!(actual.len(), 1);
            assert!(actual.iter().all(|(_, retired, receipt)| *retired
                && *receipt == OciBrowserShutdownReceipt::GracefulCefShutdown));
            eprintln!(
                "native profile conformance: cookie_restart=true reset=true temporary_isolation=true exclusive_lease=true manifest_sha256={} encrypted_storage=false production_acceptance=false",
                hex::encode(digest)
            );
        }
        Ok(Err(error)) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        other => panic!("native profile conformance failed: {other:?}"),
    }
}
