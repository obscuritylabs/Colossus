//! Actual publisher-bound creator/keeper and private TLS composition through Core.
mod fixture;

use super::*;
use std::path::Path;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a fresh signed owned CEF stage and compiler-bound public trust; operator preparation in native/browser/README.md"]
async fn real_owned_mac_chromium_controls_https_through_core_and_reaps_known_helpers() {
    let Some(trust) = option_env!("COLOSSUS_MACOS_OWNED_ACCEPTANCE_TRUST") else {
        panic!("prepare the independently compiler-bound public package trust");
    };
    let Some(inputs) = option_env!("COLOSSUS_MACOS_OWNED_ACCEPTANCE_INPUTS") else {
        panic!("prepare the compiler-bound synthetic TLS fixture directory");
    };
    let trust_value: Value = serde_json::from_str(trust).unwrap();
    let allocation = PathBuf::from(trust_value["package_root"].as_str().unwrap());
    let descriptor: Value =
        serde_json::from_slice(&std::fs::read(allocation.join("owned-process-host.json")).unwrap())
            .unwrap();
    let origin = descriptor["fixture_origin"].as_str().unwrap().to_owned();
    assert_eq!(descriptor["production_accepted"], false);
    let owner = crate::browser_package::macos_diagnostic_from_compiler_seal(trust.as_bytes())
        .await
        .unwrap();
    assert!(owner.is_owned_process_diagnostic());
    let server = fixture::Fixture::start(&origin, Path::new(inputs)).await;
    let requests = Arc::clone(&server.requests);
    let foreign = Arc::clone(&server.foreign_connections);
    let workspace = private_tempdir();
    let mut configuration = config(workspace.path());
    configuration.sandbox.network_destinations = vec![origin.clone()];
    let runtime = Runtime::open_with_options(
        &configuration,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(workspace.path())
            .unwrap()
            .with_browser_host(owner.host()),
    )
    .unwrap();
    let publisher = Arc::new(CapturePublisher::default());
    runtime.install_browser_artifact_publisher(publisher.clone());
    let captured_image = Arc::clone(&publisher);
    let mut check = tokio::spawn(async move {
        let executor = executor(&runtime);
        let context = begin(&runtime, "native-browser-macos");
        assert!(
            call(
                &executor,
                &context,
                "browser.open",
                json!({"mode":"embedded", "allowed_origins":["https://unreviewed.invalid"],
                    "initial_url":"https://unreviewed.invalid/"}),
            )
            .await
            .is_err()
        );
        assert_eq!(requests.load(Ordering::Acquire), 0);
        let opened = call(
            &executor,
            &context,
            "browser.open",
            json!({"mode":"embedded", "allowed_origins":[origin],
                "initial_url":format!("{origin}/fixture")}),
        )
        .await
        .unwrap();
        let observed = observe(
            &executor,
            &context,
            snapshot_target(&opened),
            "Denied origin blocked",
        )
        .await;
        assert_eq!(foreign.load(Ordering::Acquire), 0);
        let node = observed
            .snapshot
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.name == "Core click")
            .unwrap();
        let mut click = observed_target(&opened, &observed);
        click["snapshot_id"] = json!(node.element.snapshot_id);
        click["element_id"] = json!(node.element.element_id);
        call(&executor, &context, "browser.click", click)
            .await
            .unwrap();
        let observed = observe(
            &executor,
            &context,
            observed_target(&opened, &observed),
            "Core click received",
        )
        .await;
        call(
            &executor,
            &context,
            "browser.screenshot",
            observed_target(&opened, &observed),
        )
        .await
        .unwrap();
        {
            let screenshot = captured_image.0.lock().unwrap();
            let image = image::load_from_memory_with_format(
                screenshot.as_ref().unwrap().as_slice(),
                image::ImageFormat::Png,
            )
            .unwrap()
            .to_rgba8();
            assert!(image.width() >= 640 && image.height() >= 480);
            assert_eq!(
                image.get_pixel(image.width() / 2, image.height() / 2).0,
                [231, 241, 255, 255]
            );
        }
        let observed = observe(
            &executor,
            &context,
            observed_target(&opened, &observed),
            "Core click received",
        )
        .await;
        let mut navigate = observed_target(&opened, &observed);
        navigate["url"] = json!(format!("{origin}/second"));
        let navigated: BrowserObservation = serde_json::from_value(
            call(&executor, &context, "browser.navigate", navigate)
                .await
                .unwrap(),
        )
        .unwrap();
        observe(
            &executor,
            &context,
            observed_target(&opened, &navigated),
            "Core Mac navigation",
        )
        .await;
        assert!(requests.load(Ordering::Acquire) >= 2);
        assert_eq!(foreign.load(Ordering::Acquire), 0);
        runtime
            .browser
            .as_ref()
            .unwrap()
            .finish_run("native-browser-macos")
            .await;
        assert!(!runtime.browser_active_work());
    });
    let result = tokio::time::timeout(Duration::from_secs(60), &mut check).await;
    if result.is_err() {
        check.abort();
        let _ = check.await;
    }
    // Always settle the retained independent owner before propagating test panic.
    let cleanup =
        tokio::time::timeout(Duration::from_secs(20), owner.shutdown_direct_child()).await;
    server.close().await;
    assert!(matches!(cleanup, Ok(Ok(_))), "owned native cleanup unknown");
    let receipts = cleanup.unwrap().unwrap();
    match result {
        Ok(Ok(())) => {
            assert_eq!(receipts.len(), 1);
            assert_eq!(
                receipts[0].1,
                Some(MacosBrowserShutdownReceipt::GracefulKnownChildrenExit)
            );
            if let Some(path) = std::env::var_os("COLOSSUS_BROWSER_MACOS_ACCEPTANCE_SCREENSHOT") {
                std::fs::write(
                    path,
                    publisher.0.lock().unwrap().as_ref().unwrap().as_slice(),
                )
                .unwrap();
            }
            eprintln!(
                "Owned Core Mac HTTPS diagnostic: real AX click/navigation/PNG, denied foreign page origin, private TLS and graceful known-helper cleanup passed; production and whole-domain acceptance false"
            );
        }
        Ok(Err(error)) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        other => panic!("owned Core HTTPS diagnostic did not finish: {other:?}"),
    }
}
