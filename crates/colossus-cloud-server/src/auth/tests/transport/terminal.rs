//! Real signed OIDC, two HTTP/gRPC replicas, Fleet mTLS, public SDK and native PTY.
use super::*;
use base64::engine::general_purpose::STANDARD;
use colossus_cloud::{
    ProjectRole,
    storage::{CloudStore, MemoryCloudStore},
};
use colossus_sdk::{
    Backend as _, HumanTerminalFrameKind, HumanTerminalOperation, HumanTerminalReply,
};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
mod public;
#[cfg(target_os = "macos")]
mod readiness;

async fn socket(
    address: std::net::SocketAddr,
    origin: &str,
    headers: &HeaderMap,
    node: &str,
) -> Result<Socket, tokio_tungstenite::tungstenite::Error> {
    let mut request = format!("ws://{address}/api/projects/project-a/nodes/{node}/terminal")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", origin.parse().unwrap());
    request
        .headers_mut()
        .insert("cookie", headers["cookie"].clone());
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(socket, _)| socket)
}
async fn call(socket: &mut Socket, operation: HumanTerminalOperation) -> Value {
    let id = uuid::Uuid::new_v4().simple().to_string();
    socket
        .send(Message::Text(
            json!({"id":id,"operation":operation}).to_string().into(),
        ))
        .await
        .unwrap();
    let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await
        .expect("terminal WebSocket reply deadline")
        .unwrap()
        .unwrap();
    let Message::Text(text) = message else {
        panic!("terminal JSON expected");
    };
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["id"], id);
    value
}
async fn ok(socket: &mut Socket, operation: HumanTerminalOperation) -> HumanTerminalReply {
    let value = call(socket, operation).await;
    assert!(
        value.get("error").is_none(),
        "terminal categorical error: {:?}",
        value.get("error").map(|error| &error["code"])
    );
    serde_json::from_value(value["reply"].clone()).unwrap()
}
async fn output_until(
    socket: &mut Socket,
    terminal: &str,
    after: &mut u64,
    marker: &str,
) -> Vec<colossus_sdk::HumanTerminalFrame> {
    let mut text = String::new();
    let mut frames = Vec::new();
    tokio::time::timeout(Duration::from_secs(15), async {
        while !text.contains(marker) {
            let reply = ok(
                socket,
                HumanTerminalOperation::Read {
                    terminal_id: terminal.into(),
                    after_sequence: *after,
                    max_bytes: 65536,
                    wait_ms: 200,
                },
            )
            .await;
            assert!(!reply.gap, "complete retained display history expected");
            for frame in &reply.frames {
                assert_eq!(frame.sequence, *after + 1);
                *after = frame.sequence;
                if let HumanTerminalFrameKind::Output { data_base64 } = &frame.kind {
                    text.push_str(&String::from_utf8_lossy(
                        &STANDARD.decode(data_base64).unwrap(),
                    ));
                }
            }
            assert_eq!(reply.next_sequence, *after);
            frames.extend(reply.frames);
        }
    })
    .await
    .expect("real shell must emit marker");
    frames
}
async fn write(socket: &mut Socket, terminal: &str, epoch: u64, sequence: u64, command: &str) {
    #[cfg(target_os = "macos")]
    readiness::wait_for_line_editor(socket, terminal).await;
    let reply = ok(
        socket,
        HumanTerminalOperation::Write {
            terminal_id: terminal.into(),
            writer_epoch: epoch,
            input_sequence: sequence,
            data_base64: STANDARD.encode(command),
        },
    )
    .await;
    assert_eq!(reply.input_sequence, Some(sequence));
}
async fn wait_connected(online: &mut watch::Receiver<ConnectorStatus>) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while *online.borrow() != ConnectorStatus::Connected {
            online.changed().await.unwrap();
        }
    })
    .await
    .expect("Fleet mTLS connection readiness");
}

#[tokio::test]
async fn web_terminal_cross_replica_reconnect_keeps_the_actual_shell_and_checks_authority() {
    const TEST: &str = "auth::tests::transport::terminal::web_terminal_cross_replica_reconnect_keeps_the_actual_shell_and_checks_authority";
    if isolated::run(TEST, false).await {
        return;
    }
    let (auth, issuer, oidc) = fixture().await;
    let directory = isolated::directory();
    let root = directory.path().canonicalize().unwrap();
    let home = ColossusHome::ensure_at(root.join("native")).unwrap();
    let browser_a = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc_a = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let browser_b = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc_b = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let browser_b_address = browser_b.local_addr().unwrap();
    let mut config = auth.config.clone();
    config.http_bind = browser_a.local_addr().unwrap();
    config.grpc_bind = grpc_a.local_addr().unwrap();
    config.public_origin = format!("http://{}", config.http_bind);
    config.grpc_endpoint = format!("https://localhost:{}", config.grpc_bind.port());
    config.memberships[0].permissions = BTreeSet::from([
        CloudPermission::Read,
        CloudPermission::Execute,
        CloudPermission::Control,
        CloudPermission::Terminal,
        CloudPermission::Administer,
    ]);
    tls_files(&root, &mut config);
    drop(auth);
    let store: Arc<dyn CloudStore> = Arc::new(MemoryCloudStore::default());
    let server_a = crate::server::CloudServer::with_store(config.clone(), store.clone())
        .await
        .unwrap();
    let state_a = server_a.state.clone();
    let mut config_b = config.clone();
    config_b.http_bind = browser_b_address;
    config_b.grpc_bind = grpc_b.local_addr().unwrap();
    config_b.grpc_endpoint = format!("https://localhost:{}", config_b.grpc_bind.port());
    let server_b = crate::server::CloudServer::with_store(config_b, store.clone())
        .await
        .unwrap();
    let state_b = server_b.state.clone();
    assert_ne!(state_a.replica_id, state_b.replica_id);
    let (stop_a, shutdown_a) = watch::channel(false);
    let (stop_b, shutdown_b) = watch::channel(false);
    let host_a = tokio::spawn(server_a.serve_listeners(browser_a, grpc_a, shutdown_a));
    let host_b = tokio::spawn(server_b.serve_listeners(browser_b, grpc_b, shutdown_b));
    let origin = config.public_origin.clone();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let headers = sign_in(&client, &origin, &issuer).await;
    let invite = post(
        &client,
        &origin,
        &headers,
        "/api/projects/project-a/invitations",
        json!({"label":"Human shell fixture","roles":["primary"]}),
    )
    .await;
    let vault = Arc::new(
        PlatformCredentialVault::with_key_store(
            home.confined_root().clone(),
            "terminal-acceptance",
            Arc::new(TestKeys::default()),
        )
        .unwrap(),
    );
    let enrollment = EnrollmentStore::new(vault, "terminal-node").unwrap();
    let connection = enrollment
        .enroll(
            invite["enrollment_url"].as_str().unwrap().into(),
            Zeroizing::new(invite["token"].as_str().unwrap().into()),
            "terminal-runtime".into(),
            vec![],
            true,
        )
        .await
        .unwrap();
    let (_, key) = enrollment.load().unwrap().unwrap();

    // An ordinary enrolled client certificate is insufficient for replica RPC.
    let channel = tonic::transport::Endpoint::from_shared(connection.endpoint.clone())
        .unwrap()
        .tls_config(
            tonic::transport::ClientTlsConfig::new()
                .ca_certificate(tonic::transport::Certificate::from_pem(&connection.ca_pem))
                .identity(tonic::transport::Identity::from_pem(
                    &connection.certificate_pem,
                    key.as_bytes(),
                )),
        )
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut relay = colossus_cloud_protocol::v1alpha1::replica_terminal_relay_client::ReplicaTerminalRelayClient::new(channel);
    let denied = relay
        .dispatch(colossus_cloud_protocol::v1alpha1::ReplicaTerminalRequest::default())
        .await
        .unwrap_err();
    assert_eq!(denied.code(), tonic::Code::Unauthenticated);
    let mut forged =
        tonic::Request::new(colossus_cloud_protocol::v1alpha1::ReplicaTerminalRequest::default());
    forged.metadata_mut().insert(
        "x-colossus-terminal-relay",
        "invalid-purpose-token".parse().unwrap(),
    );
    assert_eq!(
        relay.dispatch(forged).await.unwrap_err().code(),
        tonic::Code::Unauthenticated
    );

    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let runtime_config = RuntimeConfig::offline_template(workspace.join("runtime.redb"));
    let interactions = Arc::new(PublicInteractionRouter::new(Arc::new(DenyApproval), None));
    let runtime = Arc::new(
        Runtime::open_with_options(
            &runtime_config,
            interactions.clone(),
            Some(interactions.clone()),
            RuntimeOpenOptions::for_workspace(&workspace).unwrap(),
        )
        .unwrap(),
    );
    let native_disabled =
        public::NativeRuntime::start(runtime.clone(), interactions.clone(), false).await;
    assert!(
        !native_disabled
            .full
            .capabilities()
            .contains(colossus_api::HUMAN_TERMINAL_CAPABILITY)
    );
    let disabled = native_disabled
        .full
        .agent_runs()
        .human_terminal(colossus_sdk::HumanTerminalRequest {
            owner: "human-disabled".into(),
            operation: HumanTerminalOperation::Open {
                request_id: "disabled".into(),
                controller_id: "disabled".into(),
                rows: 24,
                cols: 80,
            },
        })
        .await
        .unwrap();
    assert!(!disabled.available && disabled.session.is_none());
    native_disabled.shutdown().await;
    let native = public::NativeRuntime::start(runtime, interactions, true).await;
    assert!(
        native
            .full
            .capabilities()
            .contains(colossus_api::HUMAN_TERMINAL_CAPABILITY)
    );
    assert!(
        !native
            .read_only
            .capabilities()
            .contains(colossus_api::HUMAN_TERMINAL_CAPABILITY)
    );
    let denied = native
        .read_only
        .agent_runs()
        .human_terminal(colossus_sdk::HumanTerminalRequest {
            owner: "human-scope-denied".into(),
            operation: HumanTerminalOperation::Open {
                request_id: "denied".into(),
                controller_id: "denied".into(),
                rows: 24,
                cols: 80,
            },
        })
        .await
        .unwrap_err();
    assert_eq!(denied.reason, colossus_sdk::ApiErrorReason::ScopeDenied);
    let runs = native.full.agent_runs();
    let (disconnect, close) = watch::channel(false);
    let (status, mut online) = watch::channel(ConnectorStatus::Connecting);
    let connector = RuntimeConnector::new(connection.clone(), key, runs.clone())
        .unwrap()
        .with_enrollment_store(enrollment.clone());
    let connected = tokio::spawn(connector.run(close, status));
    wait_connected(&mut online).await;
    let lease = store
        .read_lease("project-a", &connection.node_id, crate::http::now())
        .await
        .unwrap();
    assert!(
        lease
            .owner_id
            .starts_with(&format!("{}-", state_a.replica_id))
    );
    // The web UI's capability gate must work when discovery lands on a replica
    // that does not own the connector, before it can ever open the WebSocket.
    let remote_origin = format!("http://{browser_b_address}");
    let nodes: Value = client
        .get(format!("{remote_origin}/api/projects/project-a/nodes"))
        .headers(headers.clone())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let presence = &nodes["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["node"]["node_id"] == connection.node_id)
        .expect("remote replica must list the connected node")["presence"];
    assert!(
        presence["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|capability| capability == colossus_cloud_protocol::TERMINAL_CAPABILITY)
    );
    let detail: Value = client
        .get(format!(
            "{remote_origin}/api/projects/project-a/nodes/{}",
            connection.node_id
        ))
        .headers(headers.clone())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        detail["presence"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|capability| capability == colossus_cloud_protocol::TERMINAL_CAPABILITY)
    );

    assert!(
        socket(
            browser_b_address,
            "https://untrusted.invalid",
            &headers,
            &connection.node_id
        )
        .await
        .is_err()
    );
    let mut browser = socket(browser_b_address, &origin, &headers, &connection.node_id)
        .await
        .unwrap();
    let context = ok(&mut browser, HumanTerminalOperation::Context).await;
    assert!(context.available);
    assert!(context.sessions.is_empty());
    let opened = ok(
        &mut browser,
        HumanTerminalOperation::Open {
            request_id: "open-once".into(),
            controller_id: "controller-a".into(),
            rows: 24,
            cols: 80,
        },
    )
    .await;
    let session = opened.session.unwrap();
    let terminal = session.terminal_id.clone();
    let epoch = session.writer_epoch;
    #[cfg(windows)]
    let command = "$env:COLOSSUS_TERMINAL_TEST='kept'; Write-Output ('COLOSSUS-' + 'REMOTE-ONE')\r";
    #[cfg(not(windows))]
    let command = "export COLOSSUS_TERMINAL_TEST=kept; printf '%s%s\\n' 'COLOSSUS-' 'REMOTE-ONE'\r";
    write(&mut browser, &terminal, epoch, 1, command).await;
    let mut cursor = 0;
    output_until(&mut browser, &terminal, &mut cursor, "COLOSSUS-REMOTE-ONE").await;
    browser.close(None).await.unwrap();

    // Stop only Fleet transport: the native owner and same shell remain live.
    disconnect.send_replace(true);
    tokio::time::timeout(Duration::from_secs(10), connected)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let (_, key) = enrollment.load().unwrap().unwrap();
    let (disconnect, close) = watch::channel(false);
    let (status, mut online) = watch::channel(ConnectorStatus::Connecting);
    let connector = RuntimeConnector::new(connection.clone(), key, runs.clone())
        .unwrap()
        .with_enrollment_store(enrollment.clone());
    let connected = tokio::spawn(connector.run(close, status));
    wait_connected(&mut online).await;
    let mut browser = socket(browser_b_address, &origin, &headers, &connection.node_id)
        .await
        .unwrap();
    let denied = call(
        &mut browser,
        HumanTerminalOperation::Write {
            terminal_id: terminal.clone(),
            writer_epoch: epoch,
            input_sequence: 2,
            data_base64: STANDARD.encode("unattached input\r"),
        },
    )
    .await;
    assert_eq!(denied["error"]["code"], "permission_denied");
    let attached = ok(
        &mut browser,
        HumanTerminalOperation::Attach {
            terminal_id: terminal.clone(),
            controller_id: "controller-a".into(),
            takeover: false,
        },
    )
    .await;
    assert_eq!(attached.incarnation, context.incarnation);
    let epoch_after = attached.session.unwrap().writer_epoch;
    ok(
        &mut browser,
        HumanTerminalOperation::Resize {
            terminal_id: terminal.clone(),
            writer_epoch: epoch_after,
            rows: 31,
            cols: 101,
        },
    )
    .await;
    #[cfg(windows)]
    let command = "Write-Output ('COLOSSUS-STATE-' + $env:COLOSSUS_TERMINAL_TEST)\r";
    #[cfg(not(windows))]
    let command = "printf 'COLOSSUS-STATE-%s\\n' \"$COLOSSUS_TERMINAL_TEST\"\r";
    write(
        &mut browser,
        &terminal,
        epoch_after,
        if epoch_after == epoch { 2 } else { 1 },
        command,
    )
    .await;
    let frames = output_until(&mut browser, &terminal, &mut cursor, "COLOSSUS-STATE-kept").await;
    assert!(frames.iter().any(|frame| matches!(
        frame.kind,
        HumanTerminalFrameKind::Resize {
            rows: 31,
            cols: 101
        }
    )));
    ok(
        &mut browser,
        HumanTerminalOperation::Close {
            terminal_id: terminal.clone(),
            writer_epoch: epoch_after,
        },
    )
    .await;

    // Membership revocation closes the existing socket and rejects new upgrades.
    let caller = state_a
        .auth
        .caller(&headers, "project-a", false)
        .await
        .unwrap();
    let member = state_a
        .repo
        .user_memberships(caller.subject())
        .await
        .unwrap()
        .into_iter()
        .find(|member| member.project_id == "project-a")
        .unwrap();
    state_a
        .repo
        .save_membership(
            &caller,
            caller.subject(),
            ProjectRole::Viewer,
            member.revision,
        )
        .await
        .unwrap();
    browser
        .send(Message::Text(
            json!({"id":"revoked","operation":{"operation":"context"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(5), browser.next())
        .await
        .unwrap();
    let Some(Ok(Message::Close(Some(frame)))) = closed else {
        panic!("membership revocation must send an explicit policy close");
    };
    assert_eq!(
        frame.code,
        tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy
    );
    assert!(
        socket(browser_b_address, &origin, &headers, &connection.node_id)
            .await
            .is_err()
    );
    disconnect.send_replace(true);
    tokio::time::timeout(Duration::from_secs(10), connected)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    stop_a.send_replace(true);
    stop_b.send_replace(true);
    host_a.await.unwrap().unwrap();
    host_b.await.unwrap().unwrap();
    oidc.abort();
    assert!(
        runs.human_terminal(colossus_sdk::HumanTerminalRequest {
            owner: "unrelated-human".into(),
            operation: HumanTerminalOperation::Context
        })
        .await
        .unwrap()
        .sessions
        .is_empty()
    );
    drop(runs);
    native.shutdown().await;
}
