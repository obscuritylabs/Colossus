use super::{
    Failure,
    arguments::Arguments,
    child::{self, Owner},
    evidence::Evidence,
    tls,
    wire::{Message, Role, Wire},
};
use colossus_darwin_process::{
    DarwinAuditPeer, DarwinParentChannel, DarwinProcessExitObserver, DarwinProcessIdentity,
    DarwinProcessSignal,
};
use std::{ffi::OsString, fs, net::Ipv4Addr, os::unix::fs::PermissionsExt as _, time::Duration};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpStream, UnixStream},
};
use zeroize::Zeroizing;

fn hex(hash: [u8; 20]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub async fn sacrificial(args: &Arguments, native: DarwinParentChannel) -> Result<(), Failure> {
    let (stream, peer) = native.into_parts();
    peer.verify()
        .map_err(|_| "crash_controller_admission_rejected")?;
    stream
        .set_nonblocking(true)
        .map_err(|_| "fixture_control_unavailable")?;
    let mut stream = UnixStream::from_std(stream).map_err(|_| "fixture_control_unavailable")?;
    let mut material = Zeroizing::new([0_u8; 64]);
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(material.as_mut()))
        .await
        .map_err(|_| "fixture_control_interrupted")?
        .map_err(|_| "fixture_control_interrupted")?;
    peer.verify()
        .map_err(|_| "crash_controller_admission_rejected")?;
    let mut key = Zeroizing::new([0; 32]);
    key.copy_from_slice(&material[..32]);
    let mut generation = [0; 32];
    generation.copy_from_slice(&material[32..]);
    drop(material);
    let mut controller = Wire::new(stream, key, generation, Role::Parent)?;
    let fixture = tls::Fixture::new().await?;
    let reservation = child::reserve_port()?;
    let port = reservation.port;
    let mut owner = Owner::spawn(args)?;
    drop(reservation);
    let (control, ready) = owner.bootstrap(&fixture, port).await?;
    let mut evidence = Evidence::default();
    fixture.verify(&ready, &mut evidence).await?;
    controller
        .send(Message::Ready {
            pid: owner.child.pid(),
            port,
            verified: evidence.normal_tls
                && evidence.leaf_pin_matches
                && evidence.upstream_observed
                && evidence.proxy_credential_stripped,
        })
        .await?;
    let command = tokio::time::timeout(Duration::from_secs(10), controller.receive())
        .await
        .map_err(|_| "fixture_control_timeout")??;
    if !matches!(command, Message::Crash) {
        return Err("fixture_control_rejected");
    }
    peer.verify()
        .map_err(|_| "crash_controller_admission_rejected")?;
    controller.send(Message::Ack).await?;
    // Keep the real broker child and control owner live through this abrupt
    // process exit. Neither DarwinChild::Drop nor BrokerClient::Drop can run.
    let _owners = (owner, control, ready, fixture, controller);
    std::process::exit(73)
}

pub async fn controller(args: &Arguments) -> Result<(), Failure> {
    let mut evidence = Evidence::default();
    let result = controller_inner(args, &mut evidence).await;
    evidence.accepted = result.is_ok();
    evidence.category =
        result
            .as_ref()
            .err()
            .copied()
            .unwrap_or(if evidence.broker_exit_code == Some(20) {
                "parent_crash_control_eof"
            } else {
                "parent_crash_unknown_drainage"
            });
    evidence.write(&args.evidence)?;
    result
}

async fn controller_inner(args: &Arguments, evidence: &mut Evidence) -> Result<(), Failure> {
    let self_identity = DarwinProcessIdentity::bind(std::process::id())
        .map_err(|_| "controller_identity_unavailable")?;
    DarwinAuditPeer::enroll(&self_identity, args.parent_cdhash)
        .map_err(|_| "controller_code_rejected")?;
    let parent_evidence = args.evidence.join("parent");
    fs::create_dir(&parent_evidence).map_err(|_| "fixture_evidence_unavailable")?;
    fs::set_permissions(&parent_evidence, fs::Permissions::from_mode(0o700))
        .map_err(|_| "fixture_evidence_unavailable")?;
    let argv = vec![
        OsString::from("--mode"),
        OsString::from("crash-child"),
        OsString::from("--broker"),
        args.broker.clone().into_os_string(),
        OsString::from("--broker-cdhash"),
        OsString::from(hex(args.broker_cdhash)),
        OsString::from("--parent-cdhash"),
        OsString::from(hex(args.parent_cdhash)),
        OsString::from("--evidence"),
        parent_evidence.into_os_string(),
    ];
    let image = std::env::current_exe().map_err(|_| "controller_image_unavailable")?;
    let mut parent = Owner::spawn_image(&image, args.parent_cdhash, &argv)?;
    parent
        .peer
        .verify()
        .map_err(|_| "controller_code_changed")?;
    parent
        .child
        .resume()
        .map_err(|_| "controller_resume_failed")?;
    let stream = parent.channel.take().ok_or("fixture_control_unavailable")?;
    stream
        .set_nonblocking(true)
        .map_err(|_| "fixture_control_unavailable")?;
    let mut stream = UnixStream::from_std(stream).map_err(|_| "fixture_control_unavailable")?;
    let (key, generation) = child::material()?;
    let mut material = Zeroizing::new([0; 64]);
    material[..32].copy_from_slice(&*key);
    material[32..].copy_from_slice(&generation);
    tokio::time::timeout(Duration::from_secs(5), stream.write_all(material.as_ref()))
        .await
        .map_err(|_| "fixture_control_timeout")?
        .map_err(|_| "fixture_control_interrupted")?;
    drop(material);
    let mut wire = Wire::new(stream, key, generation, Role::Controller)?;
    let ready = tokio::time::timeout(Duration::from_secs(25), wire.receive())
        .await
        .map_err(|_| "fixture_ready_timeout")??;
    let (pid, port) = match ready {
        Message::Ready {
            pid,
            port,
            verified: true,
        } => (pid, port),
        _ => return Err("fixture_ready_rejected"),
    };
    let identity = DarwinProcessIdentity::bind(pid).map_err(|_| "broker_identity_unavailable")?;
    let broker_peer = DarwinAuditPeer::enroll(&identity, args.broker_cdhash)
        .map_err(|_| "broker_code_rejected")?;
    let mut observer = DarwinProcessExitObserver::register(&identity)
        .map_err(|_| "broker_exit_observer_unavailable")?;
    let attempt = async {
        let mut held = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|_| "tracked_socket_unavailable")?;
        held.write_all(b"CONNECT ")
            .await
            .map_err(|_| "tracked_socket_unavailable")?;
        broker_peer.verify().map_err(|_| "broker_code_changed")?;
        tokio::time::timeout(Duration::from_secs(5), wire.send(Message::Crash))
            .await
            .map_err(|_| "fixture_control_timeout")??;
        let ack = tokio::time::timeout(Duration::from_secs(5), wire.receive())
            .await
            .map_err(|_| "fixture_control_timeout")??;
        if !matches!(ack, Message::Ack) {
            return Err("fixture_crash_not_acknowledged");
        }
        let cleanup = parent.finish().await;
        if let Some(status) = parent.reaped {
            evidence.parent_reaped = true;
            evidence.parent_exit_code = status.code();
        }
        evidence.forced = parent.forced;
        evidence.cleanup_category = cleanup
            .as_ref()
            .err()
            .copied()
            .unwrap_or("direct_parent_and_output_closed");
        let (parent_status, forced, stdout_empty, stderr) = cleanup?;
        evidence.forced = forced;
        evidence.stdout_empty = stdout_empty;
        evidence.parent_crashed_without_drop = parent_status.code() == Some(73) && !forced;
        if !evidence.parent_crashed_without_drop || !stdout_empty || !stderr.is_empty() {
            return Err("fixture_parent_exit_incomplete");
        }
        let exit = observer
            .wait(Duration::from_secs(5))
            .map_err(|_| "broker_exit_observation_unknown")?
            .ok_or("broker_exit_observation_timeout")?;
        evidence.broker_exit_observed = exit.identity() == &identity;
        evidence.broker_exit_code = exit.status().code();
        let mut byte = [0; 1];
        evidence.tracked_socket_closed = matches!(
            tokio::time::timeout(Duration::from_secs(2), held.read(&mut byte)).await,
            Ok(Ok(0) | Err(_))
        );
        child::denied(port, evidence).await;
        evidence.normal_tls = true;
        evidence.leaf_pin_matches = true;
        evidence.upstream_observed = true;
        evidence.proxy_credential_stripped = true;
        if !evidence.broker_exit_observed
            || !matches!(evidence.broker_exit_code, Some(20 | 24))
            || !evidence.tracked_socket_closed
            || !evidence.listener_v4_closed
            || !evidence.listener_v6_closed
        {
            return Err("fixture_crash_evidence_incomplete");
        }
        Ok(())
    }
    .await;
    if attempt.is_err() {
        // This identity-bound fallback cannot signal a later process with a
        // recycled PID. It fails acceptance and grants no whole-tree receipt.
        evidence.forced = true;
        let _ = identity.signal(DarwinProcessSignal::Kill);
        if let Ok(Some(exit)) = observer.wait(Duration::from_secs(5)) {
            evidence.broker_exit_observed = exit.identity() == &identity;
            evidence.broker_exit_code = exit.status().code();
        }
    }
    attempt
}
