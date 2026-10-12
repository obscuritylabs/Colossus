//! Real Core scoped native identity enrollment; operator-only fresh signed stage.
mod fixture;
mod journal;
use super::*;
use sha2::{Digest as _, Sha256};
use std::{os::unix::fs::PermissionsExt as _, path::Path};
use tokio::net::TcpStream;

fn scoped_context(
    runtime: &Runtime,
    run: &str,
    application: &str,
    conversation: &str,
) -> ExecutionContext {
    let context = ExecutionContext {
        run_id: Some(run.into()),
        session_id: Some(conversation.into()),
        offered_tools: runtime
            .tools
            .list_specs()
            .into_iter()
            .map(|spec| spec.name)
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
                id: application.into(),
            },
            RunControl::default(),
        )
        .unwrap();
    context
}
fn unchanged_preallocation(allocation: &Path, custody: &Path) {
    for path in [
        allocation.join("profile"),
        custody.join("colossus-tls-identity"),
        custody.join("colossus-custody-generation.jsonl"),
    ] {
        assert!(
            matches!(std::fs::symlink_metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Root-reviewed real same-component denial proof and fresh signed native-custody Core PKI stage; prepare only then Root launch"]
async fn real_owned_mac_chromium_enrolls_scoped_native_client_identity_and_retires_custody() {
    let Some(trust) = option_env!("COLOSSUS_MACOS_OWNED_ACCEPTANCE_TRUST") else {
        panic!("prepare compiler-bound public trust");
    };
    let Some(inputs) = option_env!("COLOSSUS_MACOS_OWNED_ACCEPTANCE_INPUTS") else {
        panic!("prepare fixed synthetic server inputs");
    };
    let Some(workspace) = option_env!("COLOSSUS_MACOS_OWNED_ACCEPTANCE_WORKSPACE") else {
        panic!("prepare fixed reviewed fixture workspace");
    };
    let inputs = Path::new(inputs);
    let workspace = Path::new(workspace);
    assert_eq!(workspace.canonicalize().unwrap(), workspace);
    assert_eq!(
        std::fs::metadata(workspace).unwrap().permissions().mode() & 0o7777,
        0o700
    );
    assert_eq!(
        std::fs::read_dir(workspace).unwrap().count(),
        0,
        "do not adopt prior runtime state"
    );
    let trust_value: Value = serde_json::from_str(trust).unwrap();
    let allocation = PathBuf::from(trust_value["package_root"].as_str().unwrap());
    let descriptor: Value =
        serde_json::from_slice(&std::fs::read(allocation.join("owned-process-host.json")).unwrap())
            .unwrap();
    assert_eq!(descriptor["production_accepted"], false);
    let origin = descriptor["fixture_origin"].as_str().unwrap().to_owned();
    let app = PathBuf::from(descriptor["app_bundle"].as_str().unwrap());
    let policy =
        std::fs::read(app.join("Contents/Resources/colossus-network-envelope.policy")).unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(&policy)),
        descriptor["policy_sha256"]
    );
    let fields: Vec<_> = std::str::from_utf8(&policy).unwrap().split('\n').collect();
    assert_eq!(fields.len(), 10);
    assert_eq!(fields[0], "COLOSSUS_MAC_NETWORK_ENVELOPE_V1");
    let proxy_port: u16 = fields[2].parse().unwrap();
    let custody = PathBuf::from(fields[7]);
    assert_eq!(custody.canonicalize().unwrap(), custody);
    let registry = std::fs::read(custody.join("owned-pki-registry.json")).unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(&registry)),
        descriptor["tls"]["pki_registry_sha256"]
    );
    let registry: Value = serde_json::from_slice(&registry).unwrap();
    assert_eq!(
        registry["reviews"][0]["workspace_id"],
        crate::runtime_helpers::repository_identity(workspace)
    );
    assert_eq!(
        registry["reviews"][0]["application_id"],
        "authenticated-app"
    );
    assert_eq!(
        registry["reviews"][0]["scope"],
        json!({"kind":"conversation","id":"conversation"})
    );
    assert_eq!(
        registry["identities"][0]["leaf_sha256"],
        hex::encode(Sha256::digest(
            std::fs::read(inputs.join("fixture-client.der")).unwrap()
        ))
    );
    let proof = std::fs::read(allocation.join("owned-process-custody-denials.json")).unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(&proof)),
        descriptor["custody_denials_sha256"]
    );
    unchanged_preallocation(&allocation, &custody);
    let owner = crate::browser_package::macos_diagnostic_from_compiler_seal(trust.as_bytes())
        .await
        .unwrap();
    assert!(owner.is_owned_process_diagnostic());
    let server = fixture::Fixture::start(&origin, inputs).await;
    let requests = Arc::clone(&server.requests);
    let reviewed = Arc::clone(&server.reviewed_clients);
    let foreign = Arc::clone(&server.foreign_connections);
    let wrong = Arc::clone(&server.wrong_clients);
    let mut configuration = config(workspace);
    configuration.sandbox.network_destinations = vec![origin.clone()];
    let runtime = Runtime::open_with_options(
        &configuration,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(workspace)
            .unwrap()
            .with_browser_host(owner.host()),
    )
    .unwrap();
    let publisher = Arc::new(CapturePublisher::default());
    runtime.install_browser_artifact_publisher(publisher.clone());
    let captured = Arc::clone(&publisher);
    let held = Arc::new(StdMutex::new(None::<TcpStream>));
    let retained_held = Arc::clone(&held);
    let checked_allocation = allocation.clone();
    let checked_custody = custody.clone();
    let foreign_host = owner.host();
    let fixture_origin = origin.clone();
    let mut check = tokio::spawn(async move {
        for (run, app, scope) in [
            ("pki-foreign-app", "foreign-app", "conversation"),
            (
                "pki-foreign-scope",
                "authenticated-app",
                "unreviewed-conversation",
            ),
        ] {
            let context = scoped_context(&runtime, run, app, scope);
            assert!(
                call(
                    &executor(&runtime),
                    &context,
                    "browser.open",
                    json!({"mode":"embedded",
                "allowed_origins":[origin], "initial_url":format!("{origin}/fixture")})
                )
                .await
                .is_err()
            );
            runtime.browser.as_ref().unwrap().finish_run(run).await;
            unchanged_preallocation(&checked_allocation, &checked_custody);
            assert_eq!(requests.load(Ordering::Acquire), 0);
        }
        let foreign_workspace = private_tempdir();
        let mut foreign_config = config(foreign_workspace.path());
        foreign_config.sandbox.network_destinations = vec![origin.clone()];
        let other = Runtime::open_with_options(
            &foreign_config,
            Arc::new(DenyApproval),
            None,
            RuntimeOpenOptions::for_workspace(foreign_workspace.path())
                .unwrap()
                .with_browser_host(foreign_host),
        )
        .unwrap();
        let context = begin(&other, "pki-foreign-workspace");
        assert!(
            call(
                &executor(&other),
                &context,
                "browser.open",
                json!({"mode":"embedded",
            "allowed_origins":[origin], "initial_url":format!("{origin}/fixture")})
            )
            .await
            .is_err()
        );
        other
            .browser
            .as_ref()
            .unwrap()
            .finish_run("pki-foreign-workspace")
            .await;
        unchanged_preallocation(&checked_allocation, &checked_custody);
        assert_eq!(requests.load(Ordering::Acquire), 0);
        drop(other);
        let executor = executor(&runtime);
        let context = begin(&runtime, "native-browser-macos");
        let opened = call(
            &executor,
            &context,
            "browser.open",
            json!({"mode":"embedded",
            "allowed_origins":[origin], "initial_url":format!("{origin}/fixture")}),
        )
        .await
        .unwrap();
        let connection = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, proxy_port))
            .await
            .unwrap();
        *held.lock().unwrap() = Some(connection);
        let observed = observe(
            &executor,
            &context,
            snapshot_target(&opened),
            "Denied origin blocked",
        )
        .await;
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
            let image = captured.0.lock().unwrap();
            let image = image::load_from_memory_with_format(
                image.as_ref().unwrap(),
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
        let mut navigate = observed_target(&opened, &observed);
        navigate["url"] = json!(format!("{origin}/second"));
        let observed: BrowserObservation = serde_json::from_value(
            call(&executor, &context, "browser.navigate", navigate)
                .await
                .unwrap(),
        )
        .unwrap();
        observe(
            &executor,
            &context,
            observed_target(&opened, &observed),
            "Core Mac native identity navigation",
        )
        .await;
        assert!(requests.load(Ordering::Acquire) >= 2);
        assert!(reviewed.load(Ordering::Acquire) >= 2);
        assert_eq!(foreign.load(Ordering::Acquire), 0);
        assert_eq!(wrong.load(Ordering::Acquire), 0);
        runtime
            .browser
            .as_ref()
            .unwrap()
            .finish_run("native-browser-macos")
            .await;
        assert!(!runtime.browser_active_work());
    });
    let result = tokio::time::timeout(Duration::from_secs(70), &mut check).await;
    if result.is_err() {
        check.abort();
        let _ = check.await;
    }
    // Always settle retained native process/socket/store duties before a panic.
    let cleanup =
        tokio::time::timeout(Duration::from_secs(25), owner.shutdown_direct_child()).await;
    let custody_receipts = owner.custody_retirement_receipts().await;
    server.close().await;
    assert!(
        matches!(cleanup, Ok(Ok(_))),
        "actual native cleanup remains unknown"
    );
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("Core PKI scenario failed: {error}"),
        Err(_) => panic!("Core PKI scenario exceeded its deadline"),
    }
    assert_eq!(
        cleanup.unwrap().unwrap()[0].1,
        Some(MacosBrowserShutdownReceipt::GracefulKnownChildrenExit)
    );
    assert_eq!(custody_receipts.len(), 1);
    let receipt = custody_receipts[0]
        .1
        .expect("completed admitted native identity receipt");
    assert_eq!(receipt.proxy_port(), proxy_port);
    assert!(
        journal::completed(&custody, receipt),
        "final actual actor/generation journal missing"
    );
    assert!(
        matches!(std::fs::symlink_metadata(custody.join("colossus-tls-identity")), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
    );
    assert!(!std::fs::read_dir(&custody).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".retired-tls-identity-")
    }));
    for address in [
        std::net::SocketAddr::from(([127, 0, 0, 1], proxy_port)),
        std::net::SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, proxy_port)),
    ] {
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(address)).await,
            Ok(Err(_))
        ));
    }
    let mut held = retained_held
        .lock()
        .unwrap()
        .take()
        .expect("retained incomplete accepted proxy socket");
    let mut byte = [0; 1];
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), held.read(&mut byte)).await,
        Ok(Ok(0))
    ));
    let server_port = fixture_origin
        .strip_prefix("https://127.0.0.1:")
        .unwrap()
        .parse::<u16>()
        .unwrap();
    assert!(matches!(
        tokio::time::timeout(
            Duration::from_secs(2),
            TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, server_port))
        )
        .await,
        Ok(Err(_))
    ));
    if let Some(path) = std::env::var_os("COLOSSUS_BROWSER_MACOS_ACCEPTANCE_SCREENSHOT") {
        std::fs::write(path, publisher.0.lock().unwrap().as_ref().unwrap()).unwrap();
    }
    eprintln!(
        "{}",
        json!({"schema_version":1,"purpose":"colossus-macos-owned-core-pki-result",
        "native_client_mtls":true,"core_scope_denials_before_allocation":true,"native_png":true,
        "native_identity_removed":true,"actual_actor_generation_journal":true,
        "listeners_and_held_socket_closed":true,"graceful_known_children":true,"production_accepted":false})
    );
}
