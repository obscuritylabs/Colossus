//! Real networkless CEF initialization. No WFP or loopback mutation is requested.
use super::*;
use colossus_browser_bridge::{
    BrowserBridgeDriver, BrowserBridgeEnrollment, BrowserBridgeKey, InheritedBrowserChannel,
};
use colossus_browser_presentation::{PresentationChannel, PresentationClient};
use colossus_contracts::{
    BrowserActionKind, BrowserDocumentId, BrowserLimits, BrowserMode, BrowserOpenOptions,
    BrowserOrigin, BrowserScope, BrowserSessionBinding, BrowserTabId,
};
use colossus_ports::{BrowserDriver as _, RunControl};
use colossus_windows_native::{
    ExclusiveAppContainerProfile, PrivateDirectoryCreation, system_windows_directory,
};
use colossus_windows_process::{PrivatePipe, PrivateReader, SpawnRequest};
use sha2::{Digest as _, Sha256};
use std::{collections::BTreeMap, os::windows::fs::MetadataExt as _, time::Duration};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use zeroize::Zeroizing;

fn no_reparse(path: &std::path::Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_attributes() & 0x400 != 0 {
        return Err(std::io::Error::other(
            "fixture profile contains a reparse point",
        ));
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            no_reparse(&entry?.path())?;
        }
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires the explicit Windows pinned CEF stage and owner-private state parent; no administrator token required"]
async fn real_cef_initializes_in_networkless_appcontainer_without_elevation() {
    let component = PathBuf::from(
        std::env::var_os("COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_COMPONENT")
            .expect("explicit pinned false-mode component"),
    );
    let parent = PathBuf::from(
        std::env::var_os("COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_STATE_PARENT")
            .expect("explicit owner-private state parent"),
    );
    let manifest = std::fs::read(component.join("browser-component.json")).unwrap();
    let metadata: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).unwrap();
    let mut state = PrivateDirectoryCreation::create(
        &parent.join(format!("networkless-startup-{}", hex::encode(nonce))),
    )
    .unwrap();
    let profile_path = state.path().join("profile");
    let capabilities = BrowserCapabilities {
        available: true,
        engine_version: Some(metadata["cef_version"].as_str().unwrap().into()),
        modes: vec![BrowserMode::Embedded],
        actions: vec![BrowserActionKind::Navigate, BrowserActionKind::Snapshot],
        limits: BrowserLimits::default(),
        private_ca_trust: false,
        client_identities: false,
        restrictive_egress: true,
    };
    // Use the same full installed-byte/ACL verifier as the real factory. The
    // diagnostic then launches a networkless token, never a proxy exemption.
    let installation = installation::bind(
        WindowsBrowserConfig {
            component_root: component,
            manifest_sha256: Sha256::digest(&manifest).into(),
            state_root: state.path().to_owned(),
            capabilities: capabilities.clone(),
            memory_bytes: 1024 * 1024 * 1024,
            max_processes: 64,
            egress: BrowserEgressLimits {
                max_connections: 4,
                lifetime: Duration::from_secs(60),
                connection_timeout: Duration::from_secs(10),
                max_connection_bytes: 1024 * 1024,
            },
        },
        true,
    )
    .unwrap();
    let mut profile =
        ExclusiveAppContainerProfile::create(&format!("colossus.browser.{}", hex::encode(nonce)))
            .unwrap();
    let system = system_windows_directory().unwrap();
    let home = profile_path.to_string_lossy().into_owned();
    let environment = BTreeMap::from([
        ("SystemRoot".into(), system.to_string_lossy().into_owned()),
        ("windir".into(), system.to_string_lossy().into_owned()),
        (
            "PATH".into(),
            system.join("System32").to_string_lossy().into_owned(),
        ),
        ("HOME".into(), home.clone()),
        ("USERPROFILE".into(), home.clone()),
        ("LOCALAPPDATA".into(), home.clone()),
        ("APPDATA".into(), home.clone()),
        ("TEMP".into(), home.clone()),
        ("TMP".into(), home),
    ]);
    let supervised = colossus_windows_process::spawn_with_private_channels(
        &SpawnRequest {
            executable: installation
                .component
                .canonical_path()
                .join("colossus-native-browser-host.exe"),
            arguments: Vec::new(),
            cwd: installation.component.canonical_path().to_owned(),
            environment,
            appcontainer_sid: profile.sid_string().unwrap(),
            max_processes: 64,
            max_memory_bytes: 1024 * 1024 * 1024,
            proxy_port: None,
            network_filter_id: None,
        },
        4,
    )
    .unwrap();
    let mut child = supervised.child;
    let profile_directory = profile
        .verify_process(child.process_handle())
        .unwrap()
        .create_directory(&profile_path)
        .unwrap();
    let profile_handle = child.enroll_profile_directory(&profile_directory).unwrap();
    let profile_identity = colossus_windows_native::FileIdentity::of(&profile_directory).unwrap();
    let mut leases = Vec::new();
    let mut pipes = Vec::new();
    for channel in supervised.channels {
        let pipe = PrivatePipe::from_files(channel.reader, channel.writer)
            .unwrap_or_else(|_| panic!("networkless fixture private I/O allocation failed"));
        leases.push(pipe.lease.clone());
        pipes.push(pipe);
    }
    let mut drains = Vec::new();
    for file in [child.stdout.take(), child.stderr.take()]
        .into_iter()
        .flatten()
    {
        let (reader, lease) = PrivateReader::from_file(file).unwrap();
        leases.push(lease);
        drains.push(tokio::spawn(async move {
            let mut output = Vec::new();
            let _ = reader.take(64 * 1024).read_to_end(&mut output).await;
            Ok::<_, std::io::Error>(output)
        }));
    }
    let control = BrowserDriverControl::new(RunControl::default(), RunControl::default());
    let binding = BrowserSessionBinding {
        runtime_id: "windows-networkless-runtime".into(),
        workspace_id: "windows-networkless-workspace".into(),
        application_id: "windows-networkless-application".into(),
        scope: BrowserScope::Conversation {
            id: "windows-networkless-conversation".into(),
        },
    };
    let enrollment = BrowserBridgeEnrollment {
        binding: binding.clone(),
        mode: BrowserMode::Embedded,
        profile: Default::default(),
        allowed_origins: vec![BrowserOrigin::parse("http://127.0.0.1:9").unwrap()],
        instance_nonce: nonce,
        component_digest: installation.digest,
        cancellation_closes_context: true,
        capabilities,
    };
    let digest = Sha256::digest(serde_json::to_vec(&enrollment).unwrap()).into();
    let request = BrowserDriverOpenRequest {
        binding,
        run_id: Some("networkless-startup".into()),
        session_id: BrowserSessionId::parse(format!("bs_{}", hex::encode(nonce))).unwrap(),
        tab_id: BrowserTabId::parse(format!("bt_{}", hex::encode(nonce))).unwrap(),
        document_id: BrowserDocumentId::parse(format!("bd_{}", hex::encode(nonce))).unwrap(),
        options: BrowserOpenOptions {
            profile: Default::default(),
            mode: BrowserMode::Embedded,
            allowed_origins: enrollment.allowed_origins.clone(),
            initial_url: None,
        },
    };
    let mut key = Zeroizing::new([0; 32]);
    getrandom::fill(key.as_mut()).unwrap();
    let data = Zeroizing::new(serde_json::to_vec(&serde_json::json!({"enrollment": enrollment, "profile_path":profile_path, "profile_handle":profile_handle, "profile_volume":profile_identity.volume_serial_number, "profile_file_id":profile_identity.file_id, "proxy":{"address":"127.0.0.1", "port":9, "username":"colossus", "password":"networkless-fixture-unused"}, "pki":null, "presentation":{"human_input":false}})).unwrap());
    let result = tokio::time::timeout(Duration::from_secs(45), async {
        let mut pipes = pipes.into_iter();
        let mut bootstrap = pipes.next().unwrap();
        let action = pipes.next().unwrap();
        let cancellation = pipes.next().unwrap();
        let presentation = pipes.next().unwrap();
        bootstrap.writer.write_all(key.as_ref()).await?;
        bootstrap
            .writer
            .write_all(&(data.len() as u32).to_be_bytes())
            .await?;
        bootstrap.writer.write_all(&data).await?;
        bootstrap.writer.shutdown().await?;
        for phase in 1..=7 {
            let mut receipt = [0; 5];
            bootstrap
                .reader
                .read_exact(&mut receipt)
                .await
                .inspect_err(|_| {
                    eprintln!("networkless CEF startup missing_phase={phase}");
                })?;
            if receipt != [b'C', b'B', b'H', 1, phase] {
                return Err(BrowserDriverError::Denied.into());
            }
            eprintln!("networkless CEF startup phase={phase}");
        }
        drop(bootstrap);
        let key = BrowserBridgeKey::from_bootstrap(key);
        let presentation_key = key.derive_presentation_key();
        let (bridge, presenter) = tokio::try_join!(
            async {
                BrowserBridgeDriver::connect(
                    InheritedBrowserChannel::new(action.reader, action.writer),
                    InheritedBrowserChannel::new(cancellation.reader, cancellation.writer),
                    enrollment,
                    key,
                )
                .await
                .map_err(|_| Box::<dyn std::error::Error>::from(BrowserDriverError::OutcomeUnknown))
            },
            async {
                PresentationClient::connect(
                    PresentationChannel::new(presentation.reader, presentation.writer),
                    presentation_key,
                    digest,
                )
                .await
                .map_err(|_| Box::<dyn std::error::Error>::from(BrowserDriverError::OutcomeUnknown))
            },
        )?;
        bridge.open_session(request.clone(), &control).await?;
        let tree = child.observe_process_tree()?;
        for process in &tree {
            profile.verify_process(process.process_handle())?;
        }
        eprintln!("networkless CEF retained_live_processes={}", tree.len());
        bridge.close_session(&request.session_id).await?;
        presenter.disconnect();
        bridge.disconnect_for_shutdown();
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    let exit = child.wait_timeout(Duration::from_secs(5)).unwrap();
    let graceful = child.wait_tree_timeout(Duration::from_secs(2)).unwrap();
    if !graceful {
        child.terminate(1).unwrap();
    }
    assert!(
        child.wait_tree_timeout(Duration::from_secs(5)).unwrap(),
        "networkless fixture's full Job cleanup unknown"
    );
    child.retire_network().unwrap();
    for drain in &mut drains {
        let drain_result = tokio::time::timeout(Duration::from_secs(2), &mut *drain).await;
        if let Ok(Ok(Ok(output))) = drain_result {
            // Retain only fixed diagnostic categories from our Debug client.
            // Arbitrary Chromium stderr, paths and private page bytes are omitted.
            for line in String::from_utf8_lossy(&output).lines() {
                if line.starts_with("native Windows profile rejection ") {
                    eprintln!("{line}");
                } else if result.is_err() || result.as_ref().is_ok_and(|value| value.is_err()) {
                    // This fixture has a cleared environment, an empty owned
                    // profile and only about:blank. No personal/browser content
                    // is loaded. Keep the native CHECK category for diagnosis.
                    if line.contains("FATAL:")
                        || line.starts_with("native cef bootstrap ")
                        || line.starts_with("native cef startup breakpoint ")
                    {
                        eprintln!("{}", line.chars().take(4096).collect::<String>());
                    }
                }
            }
        } else {
            drain.abort();
            let _ = drain.await;
        }
    }
    for lease in &leases {
        lease.cancel();
    }
    for lease in &leases {
        assert!(
            lease.drain(Duration::from_secs(2)),
            "networkless fixture private I/O cleanup unknown"
        );
    }
    drop(child);
    profile.close().unwrap();
    drop(profile_directory);
    if result.is_err() || result.as_ref().is_ok_and(|value| value.is_err()) {
        let log_path = profile_path.join("startup.log");
        if let Ok(log) = colossus_windows_native::BoundPath::open_file(&log_path) {
            use std::io::Read as _;
            let mut output = String::new();
            let _ = log
                .try_clone_file()
                .unwrap()
                .take(64 * 1024)
                .read_to_string(&mut output);
            for line in output.lines().filter(|line| line.contains("FATAL:")) {
                eprintln!("{}", line.chars().take(4096).collect::<String>());
            }
        }
    }
    let bound = state.bind().unwrap();
    bound.revalidate().unwrap();
    no_reparse(&profile_path).unwrap();
    std::fs::remove_dir_all(&profile_path).unwrap();
    drop((bound, installation));
    state.remove_empty().unwrap();
    eprintln!(
        "networkless CEF cleanup direct_exit={exit:?}, job_empty=true, forced={}",
        !graceful
    );
    result
        .expect("bounded networkless CEF startup")
        .expect("actual networkless CEF startup and authenticated channels");
    assert!(
        graceful && exit == Some(0),
        "networkless CEF graceful shutdown required"
    );
}
