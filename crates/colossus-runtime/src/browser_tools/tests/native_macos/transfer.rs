//! Operator-owned real Core transfer acceptance with one signed synthetic HTTP host.
use super::*;
use colossus_ports::{
    BrowserDownloadPublication, BrowserReleasedArtifact, BrowserUploadArtifact,
    BrowserUploadArtifactRequest, BrowserUploadDescriptor,
};
use sha2::{Digest as _, Sha256};
use std::sync::atomic::{AtomicUsize, Ordering};

const RUN: &str = "native-browser-macos-transfer";
const PAGE: &[u8] = include_bytes!("transfer_fixture.html");
const UPLOAD_LINE: &[u8] = b"Colossus ordinary owned upload\n";

fn upload_bytes() -> Vec<u8> {
    UPLOAD_LINE.repeat(3073)
}

fn download_bytes() -> Vec<u8> {
    (0_u8..=255).collect::<Vec<_>>().repeat(513)
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[derive(Default)]
struct FixtureReceipts {
    uploads: AtomicUsize,
    same_origin_redirects: AtomicUsize,
    forbidden_redirects: AtomicUsize,
    foreign_hits: AtomicUsize,
    malformed_uploads: AtomicUsize,
}

fn exact_multipart(headers: &str, body: &[u8], expected: &[u8]) -> bool {
    let Some(content_type) = headers.lines().find_map(|line| {
        let (name, value) = line.trim_end_matches('\r').split_once(':')?;
        name.eq_ignore_ascii_case("content-type")
            .then_some(value.trim_start())
    }) else {
        return false;
    };
    let Some(boundary) = content_type.strip_prefix("multipart/form-data; boundary=") else {
        return false;
    };
    if boundary.is_empty() || boundary.len() > 70 || !boundary.is_ascii() {
        return false;
    }
    let prefix = format!("--{boundary}\r\n");
    let suffix = format!("\r\n--{boundary}--\r\n");
    if !body.starts_with(prefix.as_bytes()) || !body.ends_with(suffix.as_bytes()) {
        return false;
    }
    let part = &body[prefix.len()..body.len() - suffix.len()];
    let Some(header_end) = part.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
        return false;
    };
    let Ok(part_headers) = std::str::from_utf8(&part[..header_end]) else {
        return false;
    };
    part_headers
        .contains("Content-Disposition: form-data; name=\"owned_file\"; filename=\"upload.txt\"")
        && &part[header_end + 4..] == expected
}

#[test]
fn transfer_fixture_accepts_only_exact_multipart_bytes() {
    let bytes = upload_bytes();
    let mut body = b"--fixture\r\nContent-Disposition: form-data; name=\"owned_file\"; filename=\"upload.txt\"\r\nContent-Type: text/plain\r\n\r\n".to_vec();
    let payload = body.len();
    body.extend_from_slice(&bytes);
    body.extend_from_slice(b"\r\n--fixture--\r\n");
    let headers =
        "POST /upload HTTP/1.1\r\ncontent-type: multipart/form-data; boundary=fixture\r\n";
    assert!(exact_multipart(headers, &body, &bytes));
    body[payload] ^= 1;
    assert!(!exact_multipart(headers, &body, &bytes));
    assert!(!exact_multipart(
        "Content-Type: multipart/form-data; boundary=other\r\n",
        &body,
        &bytes,
    ));
}

async fn respond(
    mut stream: tokio::net::TcpStream,
    origin: &str,
    forbidden_origin: &str,
    receipts: &FixtureReceipts,
) -> std::io::Result<()> {
    let mut request = Vec::new();
    let mut block = [0_u8; 4096];
    let header_end = loop {
        let count = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut block))
            .await
            .map_err(std::io::Error::other)??;
        if count == 0 {
            return Ok(());
        }
        request.extend_from_slice(&block[..count]);
        if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break end + 4;
        }
        if request.len() > 8192 {
            return Ok(());
        }
    };
    let headers = std::str::from_utf8(&request[..header_end]).unwrap_or("");
    let mut request_line = headers
        .lines()
        .next()
        .unwrap_or("")
        .split_ascii_whitespace();
    let method = request_line.next().unwrap_or("");
    let target = request_line.next().unwrap_or("");
    // A direct origin server sees origin-form; a fixture proxy sees absolute-form.
    // Only the admitted origin may be normalized into this fixture's route.
    let path = target.strip_prefix(origin).unwrap_or(target);
    let mut body = request[header_end..].to_vec();
    let (status, extra, content): (&str, String, Vec<u8>) = match (method, path) {
        ("GET", "/fixture") =>
            ("200 OK", "Content-Type: text/html; charset=utf-8\r\n".into(), PAGE.into()),
        ("GET", "/download-redirect") => {
            receipts.same_origin_redirects.fetch_add(1, Ordering::SeqCst);
            ("302 Found", format!("Location: {origin}/download-final\r\n"), Vec::new())
        }
        ("GET", "/download-final") => (
            "200 OK",
            "Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"../../untrusted-name.bin\"\r\n".into(),
            download_bytes(),
        ),
        ("GET", "/cross-origin-redirect") => {
            receipts.forbidden_redirects.fetch_add(1, Ordering::SeqCst);
            ("302 Found", format!("Location: {forbidden_origin}/file\r\n"), Vec::new())
        }
        ("POST", "/upload") => {
            let length = headers.lines().find_map(|line| {
                let (name, value) = line.trim_end_matches('\r').split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim())
                    .and_then(|number| number.parse::<usize>().ok())
            });
            let valid = if let Some(length) = length.filter(|length| *length <= 4 * 1024 * 1024 + 65536) {
                while body.len() < length {
                    let count = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut block))
                        .await
                        .map_err(std::io::Error::other)??;
                    if count == 0 {
                        break;
                    }
                    body.extend_from_slice(&block[..count]);
                }
                body.len() == length && exact_multipart(headers, &body, &upload_bytes())
            } else {
                false
            };
            if valid {
                receipts.uploads.fetch_add(1, Ordering::SeqCst);
                ("200 OK", "Content-Type: text/plain\r\n".into(), b"ok".to_vec())
            } else {
                receipts.malformed_uploads.fetch_add(1, Ordering::SeqCst);
                ("400 Bad Request", "Content-Type: text/plain\r\n".into(), b"bad".to_vec())
            }
        }
        _ => ("404 Not Found", String::new(), Vec::new()),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
        content.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(&content).await?;
    stream.shutdown().await
}

async fn fixture() -> (
    String,
    Arc<FixtureReceipts>,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let forbidden = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let forbidden_origin = format!("http://{}", forbidden.local_addr().unwrap());
    let receipts = Arc::new(FixtureReceipts::default());
    let task_origin = origin.clone();
    let task_forbidden_origin = forbidden_origin.clone();
    let task_receipts = Arc::clone(&receipts);
    let task = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let origin = task_origin.clone();
            let forbidden_origin = task_forbidden_origin.clone();
            let receipts = Arc::clone(&task_receipts);
            tokio::spawn(async move {
                respond(stream, &origin, &forbidden_origin, &receipts)
                    .await
                    .unwrap();
            });
        }
    });
    let foreign_receipts = Arc::clone(&receipts);
    let foreign_task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = forbidden.accept().await.unwrap();
            foreign_receipts.foreign_hits.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nleak",
                    )
                    .await;
            });
        }
    });
    (origin, receipts, task, foreign_task)
}

struct TransferPublisher {
    origin: String,
    resolved: AtomicUsize,
    published: AtomicUsize,
}

impl std::fmt::Debug for TransferPublisher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TransferPublisher { private_transfer: <redacted> }")
    }
}

#[async_trait]
impl BrowserArtifactPublisher for TransferPublisher {
    async fn publish(
        &self,
        _: Arc<dyn EventJournal>,
        _: BrowserArtifactPublication,
    ) -> Result<ModelImageReference, BrowserArtifactError> {
        Err(BrowserArtifactError::Invalid)
    }

    async fn resolve_upload(
        &self,
        _: Arc<dyn EventJournal>,
        request: BrowserUploadArtifactRequest,
    ) -> Result<BrowserUploadArtifact, BrowserArtifactError> {
        assert_eq!(request.binding.application_id, "authenticated-app");
        assert_eq!(request.run_id, RUN);
        self.resolved.fetch_add(1, Ordering::SeqCst);
        let bytes = upload_bytes();
        let approved = format!("artifact-{}", hash(&bytes));
        let wrong_hash = format!("artifact-{}", "e".repeat(64));
        if request.artifact_id != approved && request.artifact_id != wrong_hash {
            return Err(BrowserArtifactError::Invalid);
        }
        Ok(BrowserUploadArtifact {
            descriptor: BrowserUploadDescriptor {
                artifact_id: request.artifact_id.clone(),
                size_bytes: bytes.len() as u32,
                sha256: if request.artifact_id == wrong_hash {
                    "0".repeat(64)
                } else {
                    hash(&bytes)
                },
                file_name: "upload.txt".into(),
            },
            bytes: zeroize::Zeroizing::new(bytes),
        })
    }

    async fn publish_download(
        &self,
        _: Arc<dyn EventJournal>,
        request: BrowserDownloadPublication,
    ) -> Result<BrowserReleasedArtifact, BrowserArtifactError> {
        assert_eq!(request.binding.application_id, "authenticated-app");
        assert_eq!(request.run_id, RUN);
        assert_eq!(request.bytes.as_slice(), download_bytes());
        assert_eq!(request.descriptor.origin.as_str(), self.origin.as_str());
        assert_eq!(request.descriptor.size_bytes as usize, request.bytes.len());
        assert_eq!(request.descriptor.sha256, hash(&request.bytes));
        self.published.fetch_add(1, Ordering::SeqCst);
        Ok(BrowserReleasedArtifact {
            artifact_id: format!("artifact-{}", request.descriptor.sha256),
            file_name: "browser-download.bin".into(),
            media_type: "application/octet-stream".into(),
            size_bytes: request.bytes.len() as u64,
            sha256: request.descriptor.sha256,
        })
    }
}

async fn reference(
    executor: &BrowserToolExecutor,
    context: &ExecutionContext,
    opened: &Value,
    role: &str,
    name: &str,
) -> Value {
    let observed = observe(executor, context, snapshot_target(opened), name).await;
    let nodes = &observed.snapshot.as_ref().unwrap().nodes;
    let mut matching = nodes
        .iter()
        .filter(|node| node.role == role && node.name == name);
    let node = matching.next().unwrap_or_else(|| {
        panic!(
            "fixture action must have one exact AX role and name; matching roles: {}",
            matching_role_counts(nodes, name)
        )
    });
    assert!(
        matching.next().is_none(),
        "fixture AX role and name are ambiguous; matching roles: {}",
        matching_role_counts(nodes, name)
    );
    let mut arguments = observed_target(opened, &observed);
    arguments["snapshot_id"] = json!(node.element.snapshot_id);
    arguments["element_id"] = json!(node.element.element_id);
    arguments
}

fn matching_role_counts(nodes: &[colossus_contracts::BrowserSnapshotNode], name: &str) -> String {
    let mut button = 0;
    let mut link = 0;
    let mut other = 0;
    for node in nodes.iter().filter(|node| node.name == name) {
        match node.role.as_str() {
            "button" => button += 1,
            "link" => link += 1,
            _ => other += 1,
        }
    }
    format!("button={button},link={link},other={other}")
}

/// Run only with a fresh signed, policy-bound Debug Mac component and owned HTTP fixture.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an explicit fresh signed synthetic-profile Mac CEF stage; never run the native_session_ports probe"]
async fn real_mac_chromium_transfers_actual_core_artifacts_and_denies_unreviewed_paths() {
    let app = PathBuf::from(
        std::env::var_os("COLOSSUS_BROWSER_MACOS_ACCEPTANCE_APP")
            .expect("explicit signed diagnostic app"),
    );
    let allocation = app.parent().unwrap().to_path_buf();
    let metadata: Value = serde_json::from_slice(
        &std::fs::read(app.with_file_name(format!(
            "{}.browser-component.json",
            app.file_name().unwrap().to_str().unwrap()
        )))
        .unwrap(),
    )
    .unwrap();
    let workspace = private_tempdir();
    let (origin, fixture_receipts, fixture_task, foreign_task) = fixture().await;
    let limits = BrowserLimits {
        max_sessions: 1,
        max_tabs: 1,
        ..BrowserLimits::default()
    };
    let supervisor = Arc::new(
        MacosBrowserSupervisor::install_for_acceptance(MacosBrowserAcceptanceConfig {
            owned_process: None,
            tls_owner: None,
            tls_config: None,
            allocation_root: allocation.clone(),
            app_bundle: app,
            manifest_sha256: digest("COLOSSUS_BROWSER_MACOS_MANIFEST_SHA256"),
            front_cdhash: digest("COLOSSUS_BROWSER_MACOS_FRONT_CDHASH"),
            body_cdhash: digest("COLOSSUS_BROWSER_MACOS_BODY_CDHASH"),
            policy_sha256: digest("COLOSSUS_BROWSER_MACOS_POLICY_SHA256"),
            capabilities: BrowserCapabilities {
                available: true,
                engine_version: Some(metadata["cef_version"].as_str().unwrap().into()),
                modes: vec![BrowserMode::Embedded],
                actions: vec![
                    BrowserActionKind::Snapshot,
                    BrowserActionKind::Click,
                    BrowserActionKind::Upload,
                    BrowserActionKind::Download,
                ],
                limits,
                private_ca_trust: false,
                client_identities: false,
                restrictive_egress: true,
            },
            egress: BrowserEgressLimits {
                max_connections: 4,
                lifetime: Duration::from_secs(120),
                connection_timeout: Duration::from_secs(10),
                max_connection_bytes: 1024 * 1024,
            },
        })
        .await
        .unwrap(),
    );
    assert!(!supervisor.readiness().is_available());
    let captured = Arc::new(CapturedHost {
        supervisor: Arc::clone(&supervisor),
        request: StdMutex::new(None),
    });
    let (host, retained) =
        RuntimeBrowserHost::macos_supervised_for_acceptance(Arc::clone(&captured)).unwrap();
    let host = host.with_native_profile_protection(vec![
        colossus_home::ConfinedRoot::bind(&allocation).unwrap(),
    ]);
    let mut configuration = config(workspace.path());
    configuration.sandbox.network_destinations = vec![origin.clone()];
    let runtime = Runtime::open_with_options(
        &configuration,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(workspace.path())
            .unwrap()
            .with_browser_host(host),
    )
    .unwrap();
    let publisher = Arc::new(TransferPublisher {
        origin: origin.clone(),
        resolved: AtomicUsize::new(0),
        published: AtomicUsize::new(0),
    });
    runtime.install_browser_artifact_publisher(publisher.clone());
    let task_receipts = Arc::clone(&fixture_receipts);
    let task_publisher = Arc::clone(&publisher);
    let mut check = tokio::spawn(async move {
        let executor = executor(&runtime);
        let context = begin(&runtime, RUN);
        let opened = call(
            &executor,
            &context,
            "browser.open",
            json!({"mode":"embedded", "allowed_origins":[origin],
                "initial_url":format!("{origin}/fixture")}),
        )
        .await
        .unwrap();
        let upload_input = reference(
            &executor,
            &context,
            &opened,
            "button",
            "Owned native file input",
        )
        .await;
        let mut unknown = upload_input.clone();
        unknown["artifact_id"] = json!(format!("artifact-{}", "f".repeat(64)));
        assert!(matches!(
            call(&executor, &context, "browser.upload", unknown).await,
            Err(ToolError::Denied(_))
        ));
        let mut mismatched = reference(
            &executor,
            &context,
            &opened,
            "button",
            "Owned native file input",
        )
        .await;
        mismatched["artifact_id"] = json!(format!("artifact-{}", "e".repeat(64)));
        assert!(matches!(
            call(&executor, &context, "browser.upload", mismatched).await,
            Err(ToolError::Denied(_))
        ));
        assert_eq!(task_receipts.uploads.load(Ordering::SeqCst), 0);
        let mut approved = reference(
            &executor,
            &context,
            &opened,
            "button",
            "Owned native file input",
        )
        .await;
        approved["artifact_id"] = json!(format!("artifact-{}", hash(&upload_bytes())));
        let uploaded = call(&executor, &context, "browser.upload", approved)
            .await
            .unwrap();
        assert_eq!(
            uploaded["tab"]["document_id"],
            opened["session"]["tabs"][0]["document_id"]
        );
        let submit = reference(
            &executor,
            &context,
            &opened,
            "button",
            "Submit owned upload",
        )
        .await;
        call(&executor, &context, "browser.click", submit)
            .await
            .unwrap();
        observe(
            &executor,
            &context,
            snapshot_target(&opened),
            "Owned upload received",
        )
        .await;
        assert_eq!(task_receipts.uploads.load(Ordering::SeqCst), 1);
        assert_eq!(task_receipts.malformed_uploads.load(Ordering::SeqCst), 0);

        let download = reference(
            &executor,
            &context,
            &opened,
            "link",
            "Owned binary download",
        )
        .await;
        let released = call(&executor, &context, "browser.download", download)
            .await
            .unwrap();
        let expected = download_bytes();
        assert_eq!(released["artifact"]["size_bytes"], expected.len());
        assert_eq!(released["artifact"]["sha256"], hash(&expected));
        assert_eq!(
            released["artifact"]["artifact_id"],
            format!("artifact-{}", hash(&expected))
        );
        assert_eq!(released["artifact"]["file_name"], "browser-download.bin");
        assert_eq!(
            released["artifact"]["media_type"],
            "application/octet-stream"
        );
        let public = serde_json::to_string(&released).unwrap();
        assert!(!public.contains("transfer_id") && !public.contains("content_base64"));
        assert_eq!(task_publisher.published.load(Ordering::SeqCst), 1);
        assert_eq!(
            task_receipts.same_origin_redirects.load(Ordering::SeqCst),
            1
        );

        let mut denied_executor = super::super::executor(&runtime);
        denied_executor.gateway = Arc::new(EffectGateway::new(
            runtime.journal(),
            Arc::new(PostDeny(
                BuiltInPolicy::offline_default()
                    .with_action("browser.download", DecisionOutcome::Allow)
                    .with_network_destination(&origin),
            )),
            Arc::new(DenyApproval),
            SafetyKernel::new(["browser.download".into()]),
            [9; 32],
        ));
        let denied_download = reference(
            &executor,
            &context,
            &opened,
            "link",
            "Owned binary download",
        )
        .await;
        assert!(matches!(
            call(
                &denied_executor,
                &context,
                "browser.download",
                denied_download
            )
            .await,
            Err(ToolError::Denied(_))
        ));
        assert_eq!(task_publisher.published.load(Ordering::SeqCst), 1);
        assert_eq!(
            task_receipts.same_origin_redirects.load(Ordering::SeqCst),
            2
        );

        let forbidden = reference(
            &executor,
            &context,
            &opened,
            "link",
            "Forbidden redirect download",
        )
        .await;
        assert!(
            call(&executor, &context, "browser.download", forbidden)
                .await
                .is_err()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(task_receipts.forbidden_redirects.load(Ordering::SeqCst), 1);
        assert_eq!(task_receipts.foreign_hits.load(Ordering::SeqCst), 0);
        assert_eq!(task_publisher.published.load(Ordering::SeqCst), 1);
        assert_eq!(task_publisher.resolved.load(Ordering::SeqCst), 3);
        runtime.browser.as_ref().unwrap().finish_run(RUN).await;
    });
    let result = tokio::time::timeout(Duration::from_secs(150), &mut check).await;
    if result.is_err() {
        check.abort();
        let _ = check.await;
    }
    let cleanup = tokio::time::timeout(Duration::from_secs(20), supervisor.shutdown()).await;
    let shutdown_receipts = supervisor.shutdown_receipts().await;
    fixture_task.abort();
    foreign_task.abort();
    assert!(
        matches!(cleanup, Ok(Ok(()))),
        "diagnostic native cleanup unknown; allocation retained"
    );
    drop(retained);
    match result {
        Ok(Ok(())) => {
            assert_eq!(shutdown_receipts.len(), 1);
            assert_eq!(
                shutdown_receipts[0].1,
                Some(MacosBrowserShutdownReceipt::GracefulDirectChildExit)
            );
            eprintln!(
                "Core Mac transfer diagnostic: actual HTTP multipart upload, same-origin redirected binary download, exact SHA-256 and owner metadata, unapproved/hash/redirect denial, direct-child shutdown passed; production acceptance false"
            );
        }
        Ok(Err(error)) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        other => panic!("Core Mac transfer diagnostic did not finish: {other:?}"),
    }
}
