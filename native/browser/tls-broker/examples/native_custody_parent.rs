//! Root/operator-only signed no-CEF native mTLS and physical custody fixture.
#![forbid(unsafe_code)]
#[path = "native_parent_acceptance/arguments.rs"]
#[allow(dead_code)]
mod arguments;
#[path = "../src/entry_seal.rs"]
mod cdhash;
#[path = "native_parent_acceptance/child.rs"]
#[allow(dead_code)]
mod child;
#[path = "native_custody_parent/diagnostics.rs"]
mod diagnostics;
#[path = "native_parent_acceptance/evidence.rs"]
#[allow(dead_code)]
mod evidence;
#[path = "native_custody_parent/journal.rs"]
mod journal;
#[path = "native_parent_acceptance/tls.rs"]
#[allow(dead_code)]
mod tls;
use colossus_native_browser_tls_broker::{
    Bootstrap, BootstrapEnvelope, BrokerClient, BrokerLimits,
};
use rustls::{
    RootCertStore, ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};
use sha2::{Digest as _, Sha256};
use std::{
    io::Write as _, net::SocketAddr, path::PathBuf, process::ExitCode, sync::Arc, time::Duration,
};
use tokio::io::AsyncWriteExt as _;
use zeroize::Zeroizing;
type Failure = &'static str;
fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [root] = args.as_slice() else {
        return fail("fixed_fixture_root_required");
    };
    let root = PathBuf::from(root);
    if !root.is_absolute() || root.canonicalize().ok().as_ref() != Some(&root) {
        return fail("fixture_root_rejected");
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(_) => return fail("runtime_unavailable"),
    };
    match runtime.block_on(run(root)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(category) => fail(category),
    }
}
async fn run(root: PathBuf) -> Result<(), Failure> {
    let descriptor: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("native-custody-acceptance.json"))
            .map_err(|_| "descriptor_missing")?,
    )
    .map_err(|_| "descriptor_invalid")?;
    if descriptor["purpose"] != "colossus-macos-native-custody-acceptance"
        || descriptor["production_accepted"] != false
        || descriptor["cef_artifact_receipt"] != false
    {
        return Err("fixture_descriptor_rejected");
    }
    let hash = cdhash::parse(
        descriptor["broker_cdhash"]
            .as_str()
            .ok_or("broker_code_missing")?,
    )
    .map_err(|_| "broker_code_invalid")?;
    let inputs = root.join("custody/acceptance-inputs");
    let config: serde_json::Value = serde_json::from_slice(
        &std::fs::read(inputs.join("identity.json")).map_err(|_| "fixture_input_missing")?,
    )
    .map_err(|_| "fixture_input_invalid")?;
    let origin = config["origin"]
        .as_str()
        .ok_or("fixture_origin_missing")?
        .to_owned();
    let port = origin
        .strip_prefix("https://localhost:")
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|p| *p != 0)
        .ok_or("fixture_origin_invalid")?;
    let root_der =
        std::fs::read(inputs.join("private-ca.der")).map_err(|_| "fixture_ca_missing")?;
    let reviewed =
        std::fs::read(inputs.join("reviewed-client.der")).map_err(|_| "fixture_leaf_missing")?;
    let server_der =
        std::fs::read(inputs.join("server.der")).map_err(|_| "fixture_server_missing")?;
    let server_key = Zeroizing::new(
        std::fs::read(inputs.join("server-key.pk8")).map_err(|_| "fixture_server_missing")?,
    );
    let mut roots = RootCertStore::empty();
    roots
        .add(root_der.clone().into())
        .map_err(|_| "fixture_ca_invalid")?;
    let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .map_err(|_| "fixture_client_verifier_invalid")?;
    let mut server =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|_| "fixture_tls_invalid")?
            .with_client_cert_verifier(verifier)
            .with_single_cert(
                vec![CertificateDer::from(server_der)],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key.to_vec())),
            )
            .map_err(|_| "fixture_server_invalid")?;
    server.alpn_protocols = vec![b"http/1.1".to_vec()];
    let fixture = tls::Fixture::from_native_material(
        SocketAddr::from(([127, 0, 0, 1], port)),
        root_der.clone(),
        Arc::new(server),
        Sha256::digest(&reviewed).into(),
    )
    .await?;
    let mut owner =
        child::Owner::spawn_image(&root.join("colossus-native-custody-acceptance"), hash, &[])?;
    let mut evidence = evidence::Evidence::default();
    let mut enrollment_generation = None;
    let scenario = perform(
        &mut owner,
        &fixture,
        origin,
        root_der,
        &mut evidence,
        &mut enrollment_generation,
    )
    .await;
    // Every returned scenario error closes control before exact child reaping and
    // bounded output settlement. Keep original failure beside cleanup evidence.
    let output = owner.finish_output().await;
    let mut cleanup_category = None;
    let mut exit_code = None;
    let mut native_receipt = None;
    let mut forced = false;
    let mut output_settled = false;
    let mut native_diagnostics = Vec::new();
    match output {
        Ok((status, was_forced, stdout, stderr)) => {
            output_settled = true;
            forced = was_forced;
            exit_code = status.code();
            native_receipt = serde_json::from_slice::<serde_json::Value>(&stdout).ok();
            if forced
                || !status.success()
                || !stderr.is_empty()
                || native_receipt
                    != Some(
                        serde_json::json!({"schema_version":1,"native_identity_removed":true,"default_and_search_metadata_unchanged":true,"cef_loaded":false,"production_accepted":false}),
                    )
            {
                cleanup_category = Some("native_child_receipt_or_exit_rejected");
            }
            if !stderr.is_empty() {
                match diagnostics::capture(&stderr) {
                    Ok((traces, category)) => {
                        native_diagnostics = traces;
                        cleanup_category = category.or(cleanup_category);
                    }
                    Err(()) => cleanup_category = Some("native_diagnostics_rejected"),
                }
            }
        }
        Err(category) => cleanup_category = Some(category),
    }
    let namespace_absent = matches!(std::fs::symlink_metadata(root.join("custody/colossus-tls-identity")),Err(error) if error.kind()==std::io::ErrorKind::NotFound)
        && std::fs::read_dir(root.join("custody"))
            .map_err(|_| "custody_directory_unknown")?
            .all(|entry| {
                entry.is_ok_and(|entry| {
                    !entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".retired-tls-identity-")
                })
            });
    let store_removed = namespace_absent
        && enrollment_generation
            .is_some_and(|generation| journal::completed(&root, &owner.peer, generation, hash))
        && cleanup_category.is_none()
        && output_settled
        && !forced
        && exit_code == Some(0)
        && native_receipt.is_some();
    let accepted = scenario.is_ok()
        && cleanup_category.is_none()
        && store_removed
        && evidence.normal_tls
        && evidence.proxy_credential_stripped
        && evidence.tracked_socket_closed
        && evidence.listener_v4_closed
        && evidence.listener_v6_closed;
    let receipt = serde_json::json!({"schema_version":1,"purpose":"colossus-macos-native-custody-tls-result","accepted":accepted,"original_failure":scenario.err(),"cleanup_failure":cleanup_category,"native_diagnostics":native_diagnostics,"server_observed_reviewed_native_client":evidence.normal_tls&&evidence.upstream_observed,"normal_tls":evidence.normal_tls,"private_proxy_credentials_stripped":evidence.proxy_credential_stripped,"listener_v4_closed":evidence.listener_v4_closed,"listener_v6_closed":evidence.listener_v6_closed,"held_socket_closed":evidence.tracked_socket_closed,"native_namespace_absent":namespace_absent,"native_store_removed":store_removed,"broker_reaped":owner.reaped.is_some(),"broker_exit_code":exit_code,"forced":forced,"output_settled":output_settled,"native_receipt":native_receipt,"cef_loaded":false,"production_accepted":false});
    std::fs::write(
        root.join("native-custody-tls-result.json"),
        serde_json::to_vec_pretty(&receipt).map_err(|_| "receipt_invalid")?,
    )
    .map_err(|_| "receipt_unavailable")?;
    if !accepted {
        return Err("native_custody_acceptance_failed_see_retained_receipt");
    }
    let _ = writeln!(
        std::io::stdout().lock(),
        "native custody TLS fixture passed; production remains disabled"
    );
    Ok(())
}
async fn perform(
    owner: &mut child::Owner,
    fixture: &tls::Fixture,
    origin: String,
    root_der: Vec<u8>,
    evidence: &mut evidence::Evidence,
    enrollment_generation: &mut Option<[u8; 32]>,
) -> Result<(), Failure> {
    owner.peer.verify().map_err(|_| "broker_code_changed")?;
    owner.child.resume().map_err(|_| "broker_resume_failed")?;
    let stream = owner.channel.take().ok_or("broker_channel_missing")?;
    stream
        .set_nonblocking(true)
        .map_err(|_| "broker_channel_invalid")?;
    let mut stream =
        tokio::net::UnixStream::from_std(stream).map_err(|_| "broker_channel_invalid")?;
    let mut key = Zeroizing::new([0; 32]);
    let mut generation = [0; 32];
    getrandom::fill(key.as_mut()).map_err(|_| "entropy_unavailable")?;
    getrandom::fill(&mut generation).map_err(|_| "entropy_unavailable")?;
    *enrollment_generation = Some(generation);
    let mut entry = Zeroizing::new([0; 64]);
    entry[..32].copy_from_slice(key.as_ref());
    entry[32..].copy_from_slice(&generation);
    tokio::time::timeout(Duration::from_secs(5), stream.write_all(entry.as_ref()))
        .await
        .map_err(|_| "broker_bootstrap_timeout")?
        .map_err(|_| "broker_bootstrap_closed")?;
    drop(entry);
    let mut control = BrokerClient::from_admitted_stream(stream, key, generation)
        .map_err(|_| "control_rejected")?;
    let proxy =
        std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|_| "proxy_port_unavailable")?;
    let port = proxy
        .local_addr()
        .map_err(|_| "proxy_port_unavailable")?
        .port();
    drop(proxy);
    let ready = control
        .bootstrap(BootstrapEnvelope {
            bootstrap: Bootstrap {
                origin,
                destinations: vec![fixture.address],
                ca_der: vec![root_der],
                generation,
            },
            limits: BrokerLimits {
                connections: 4,
                lifetime: Duration::from_secs(90),
                exchange_timeout: Duration::from_secs(12),
                exchange_bytes: 1024 * 1024,
            },
            port,
        })
        .await
        .map_err(|_| "native_identity_or_control_rejected")?;
    fixture.verify(&ready, evidence).await?;
    let mut held = tokio::net::TcpStream::connect((std::net::Ipv6Addr::LOCALHOST, port))
        .await
        .map_err(|_| "held_proxy_unavailable")?;
    control
        .shutdown()
        .await
        .map_err(|_| "native_identity_cleanup_unacknowledged")?;
    evidence.shutdown_ack = true;
    drop(control);
    let mut byte = [0];
    use tokio::io::AsyncReadExt as _;
    evidence.tracked_socket_closed =
        match tokio::time::timeout(Duration::from_secs(2), held.read(&mut byte)).await {
            Ok(Ok(0)) => true,
            Ok(Err(error)) => matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
            ),
            _ => false,
        };
    drop(held);
    evidence.listener_v4_closed = matches!(tokio::time::timeout(Duration::from_secs(2),tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST,port))).await,Ok(Err(error)) if error.kind()==std::io::ErrorKind::ConnectionRefused);
    evidence.listener_v6_closed = matches!(tokio::time::timeout(Duration::from_secs(2),tokio::net::TcpStream::connect((std::net::Ipv6Addr::LOCALHOST,port))).await,Ok(Err(error)) if error.kind()==std::io::ErrorKind::ConnectionRefused);
    if !evidence.tracked_socket_closed
        || !evidence.listener_v4_closed
        || !evidence.listener_v6_closed
    {
        return Err("native_socket_drainage_incomplete");
    }
    Ok(())
}
fn fail(category: Failure) -> ExitCode {
    let _ = writeln!(
        std::io::stderr().lock(),
        "colossus_native_custody_parent: {category}"
    );
    ExitCode::from(1)
}
