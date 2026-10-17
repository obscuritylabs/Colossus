use super::{
    Failure,
    arguments::{Arguments, Mode},
    evidence::Evidence,
    tls,
};
use colossus_darwin_process::{
    DarwinAuditPeer, DarwinChild, DarwinProcessIdentity, spawn_suspended_parent_channel,
};
use colossus_native_browser_tls_broker::{
    Bootstrap, BootstrapEnvelope, BrokerClient, BrokerLimits, BrokerReady,
};
use std::{
    ffi::OsString,
    fs::File,
    io::Read as _,
    net::{Ipv4Addr, Ipv6Addr},
    path::Path,
    process::ExitStatus,
    sync::mpsc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpStream, UnixStream},
    time::Instant,
};
use zeroize::Zeroizing;

pub struct Owner {
    pub child: DarwinChild,
    pub peer: DarwinAuditPeer,
    pub channel: Option<std::os::unix::net::UnixStream>,
    pub reaped: Option<ExitStatus>,
    pub forced: bool,
    output: mpsc::Receiver<(bool, Result<Vec<u8>, Failure>)>,
}

impl Owner {
    pub fn spawn(args: &Arguments) -> Result<Self, Failure> {
        Self::spawn_image(&args.broker, args.broker_cdhash, &[])
    }

    pub fn spawn_image(image: &Path, cdhash: [u8; 20], argv: &[OsString]) -> Result<Self, Failure> {
        let spawned = spawn_suspended_parent_channel(image, argv, &[])
            .map_err(|_| "native_spawn_outcome_unknown")?;
        // The successful direct-child owner is retained before any identity or
        // signature check. Its Drop duty remains on every subsequent failure.
        let identity = DarwinProcessIdentity::bind(spawned.child.pid())
            .map_err(|_| "broker_identity_unavailable")?;
        let peer =
            DarwinAuditPeer::enroll(&identity, cdhash).map_err(|_| "broker_code_rejected")?;
        let (sender, output) = mpsc::channel();
        capture(spawned.stdout, true, sender.clone())?;
        capture(spawned.stderr, false, sender)?;
        Ok(Self {
            child: spawned.child,
            peer,
            channel: Some(spawned.channel),
            reaped: None,
            forced: false,
            output,
        })
    }

    pub async fn bootstrap(
        &mut self,
        fixture: &tls::Fixture,
        port: u16,
    ) -> Result<(BrokerClient, BrokerReady), Failure> {
        self.peer.verify().map_err(|_| "broker_code_changed")?;
        self.child.resume().map_err(|_| "broker_resume_failed")?;
        let stream = self.channel.take().ok_or("broker_channel_unavailable")?;
        stream
            .set_nonblocking(true)
            .map_err(|_| "broker_channel_unavailable")?;
        let mut stream = UnixStream::from_std(stream).map_err(|_| "broker_channel_unavailable")?;
        let (key, generation) = material()?;
        let mut entry = Zeroizing::new([0_u8; 64]);
        entry[..32].copy_from_slice(&*key);
        entry[32..].copy_from_slice(&generation);
        tokio::time::timeout(Duration::from_secs(5), stream.write_all(entry.as_ref()))
            .await
            .map_err(|_| "broker_entry_timeout")?
            .map_err(|_| "broker_entry_closed")?;
        drop(entry);
        let mut control = BrokerClient::from_admitted_stream(stream, key, generation)
            .map_err(|_| "broker_control_unavailable")?;
        let ready = control
            .bootstrap(BootstrapEnvelope {
                bootstrap: Bootstrap {
                    origin: fixture.origin(),
                    destinations: vec![fixture.address],
                    ca_der: vec![fixture.root.clone()],
                    generation,
                },
                limits: BrokerLimits {
                    connections: 8,
                    lifetime: Duration::from_secs(45),
                    exchange_timeout: Duration::from_secs(8),
                    exchange_bytes: 1024 * 1024,
                },
                port,
            })
            .await
            .map_err(|_| "broker_bootstrap_rejected")?;
        self.peer.verify().map_err(|_| "broker_code_changed")?;
        if ready.port() != port {
            return Err("broker_exact_port_mismatch");
        }
        Ok((control, ready))
    }

    pub async fn finish(&mut self) -> Result<(ExitStatus, bool, bool, Vec<u8>), Failure> {
        let (status, forced, stdout, stderr) = self.finish_output().await?;
        Ok((status, forced, stdout.is_empty(), stderr))
    }

    /// Retain exact reaping before positively settling both bounded output EOFs.
    pub async fn finish_output(&mut self) -> Result<(ExitStatus, bool, Vec<u8>, Vec<u8>), Failure> {
        let deadline = Instant::now() + Duration::from_secs(8);
        let (status, forced) = loop {
            match self.child.try_wait().map_err(|_| "broker_wait_unknown")? {
                Some(status) => break (status, false),
                None if Instant::now() >= deadline => {
                    self.forced = true;
                    break (
                        self.child
                            .kill_and_reap()
                            .map_err(|_| "broker_wait_unknown")?,
                        true,
                    );
                }
                None => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        };
        // Retain the positive child receipt before draining output channels,
        // whose independent failure must not erase a completed native reap.
        self.reaped = Some(status);
        self.forced = forced;
        let mut stdout_bytes = None;
        let mut stderr = None;
        for _ in 0..2 {
            let (stdout, result) = self
                .output
                .recv_timeout(Duration::from_secs(2))
                .map_err(|_| "broker_output_unknown")?;
            let bytes = result?;
            if stdout {
                stdout_bytes = Some(bytes);
            } else {
                stderr = Some(bytes);
            }
        }
        Ok((
            status,
            forced,
            stdout_bytes.ok_or("broker_output_unknown")?,
            stderr.ok_or("broker_output_unknown")?,
        ))
    }
}

fn capture(
    file: File,
    stdout: bool,
    sender: mpsc::Sender<(bool, Result<Vec<u8>, Failure>)>,
) -> Result<(), Failure> {
    std::thread::Builder::new()
        .name("tls-fixture-output".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            let result = file
                .take(4097)
                .read_to_end(&mut bytes)
                .map_err(|_| "broker_output_unknown")
                .and({
                    if bytes.len() > 4096 {
                        Err("broker_output_limit")
                    } else {
                        Ok(bytes)
                    }
                });
            let _ = sender.send((stdout, result));
        })
        .map(|_| ())
        .map_err(|_| "broker_output_unavailable")
}

pub fn material() -> Result<(Zeroizing<[u8; 32]>, [u8; 32]), Failure> {
    for _ in 0..4 {
        let mut key = Zeroizing::new([0; 32]);
        let mut generation = [0; 32];
        getrandom::fill(key.as_mut()).map_err(|_| "fixture_random_unavailable")?;
        getrandom::fill(&mut generation).map_err(|_| "fixture_random_unavailable")?;
        if *key != [0; 32] && generation != [0; 32] && *key != generation {
            return Ok((key, generation));
        }
    }
    Err("fixture_random_unavailable")
}

pub fn reserve_port() -> Result<Reservation, Failure> {
    use socket2::{Domain, Protocol, Socket, Type};
    for _ in 0..16 {
        let v4 = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| "fixture_port_unavailable")?;
        let port = v4
            .local_addr()
            .map_err(|_| "fixture_port_unavailable")?
            .port();
        let v6 = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))
            .map_err(|_| "fixture_port_unavailable")?;
        v6.set_only_v6(true)
            .map_err(|_| "fixture_port_unavailable")?;
        if v6
            .bind(&std::net::SocketAddr::from((Ipv6Addr::LOCALHOST, port)).into())
            .is_err()
        {
            continue;
        }
        v6.listen(1).map_err(|_| "fixture_port_unavailable")?;
        return Ok(Reservation {
            port,
            _v4: v4,
            _v6: v6,
        });
    }
    Err("fixture_port_unavailable")
}

pub struct Reservation {
    pub port: u16,
    _v4: std::net::TcpListener,
    _v6: socket2::Socket,
}

pub async fn denied(port: u16, evidence: &mut Evidence) {
    evidence.listener_v4_closed = matches!(
        tokio::time::timeout(
            Duration::from_secs(1),
            TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        )
        .await,
        Ok(Err(_))
    );
    evidence.listener_v6_closed = matches!(
        tokio::time::timeout(
            Duration::from_secs(1),
            TcpStream::connect((Ipv6Addr::LOCALHOST, port))
        )
        .await,
        Ok(Err(_))
    );
}

pub async fn accept(args: &Arguments) -> Result<(), Failure> {
    let mut evidence = Evidence::default();
    let attempt = accept_inner(args, &mut evidence).await;
    evidence.category = attempt.as_ref().err().copied().unwrap_or(match args.mode {
        Mode::Positive => "shutdown_acknowledged",
        Mode::Eof => "control_eof_drained",
        _ => "parent_admission_rejected",
    });
    evidence.accepted = attempt.is_ok();
    evidence.write(&args.evidence)?;
    attempt
}

async fn accept_inner(args: &Arguments, evidence: &mut Evidence) -> Result<(), Failure> {
    let fixture = tls::Fixture::new().await?;
    let reservation = reserve_port()?;
    let port = reservation.port;
    let mut owner = Owner::spawn(args)?;
    drop(reservation);
    let attempt = tokio::time::timeout(Duration::from_secs(20), async {
        let (mut control, ready) = owner.bootstrap(&fixture, port).await?;
        if args.mode == Mode::WrongParent {
            return Err("wrong_parent_was_admitted");
        }
        fixture.verify(&ready, evidence).await?;
        let mut partial = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|_| "tracked_socket_unavailable")?;
        partial
            .write_all(b"CONNECT ")
            .await
            .map_err(|_| "tracked_socket_unavailable")?;
        if args.mode == Mode::Positive {
            control
                .shutdown()
                .await
                .map_err(|_| "shutdown_ack_unavailable")?;
            evidence.shutdown_ack = true;
        }
        drop(control);
        let mut byte = [0; 1];
        evidence.tracked_socket_closed = matches!(
            tokio::time::timeout(Duration::from_secs(5), partial.read(&mut byte)).await,
            Ok(Ok(0) | Err(_))
        );
        Ok(())
    })
    .await
    .map_err(|_| "fixture_exchange_timeout")
    .and_then(|result| result);
    // Even a failed control exchange must discharge direct-child/output duties.
    let cleanup = owner.finish().await;
    denied(port, evidence).await;
    if let Some(status) = owner.reaped {
        evidence.broker_reaped = true;
        evidence.broker_exit_observed = true;
        evidence.broker_exit_code = status.code();
    }
    evidence.forced = owner.forced;
    let cleanup = cleanup.and_then(|(status, forced, stdout_empty, stderr)| {
        evidence.stdout_empty = stdout_empty;
        let expected = match args.mode {
            Mode::Positive => (0, &b""[..]),
            Mode::Eof => (20, &b"colossus_tls_broker: control_closed\n"[..]),
            Mode::WrongParent => (11, &b"colossus_tls_broker: parent_admission_rejected\n"[..]),
            _ => return Err("invalid_fixture_mode"),
        };
        if forced
            || !stdout_empty
            || status.code() != Some(expected.0)
            || stderr != expected.1
            || !evidence.listener_v4_closed
            || !evidence.listener_v6_closed
        {
            return Err("native_exit_or_output_not_clean");
        }
        Ok(())
    });
    evidence.cleanup_category = cleanup
        .as_ref()
        .err()
        .copied()
        .unwrap_or("direct_child_and_output_closed");
    if args.mode == Mode::WrongParent {
        cleanup?;
        if attempt.is_ok() || evidence.normal_tls || evidence.shutdown_ack {
            return Err("wrong_parent_was_admitted");
        }
        return Ok(());
    }
    primary_failure(attempt, cleanup)?;
    if !evidence.tracked_socket_closed
        || !evidence.normal_tls
        || !evidence.leaf_pin_matches
        || !evidence.upstream_observed
        || !evidence.proxy_credential_stripped
    {
        return Err("native_tls_evidence_incomplete");
    }
    Ok(())
}

fn primary_failure(
    exchange: Result<(), Failure>,
    cleanup: Result<(), Failure>,
) -> Result<(), Failure> {
    // Both duties have already executed. Preserve the original exchange failure
    // and record cleanup separately instead of replacing the actionable cause.
    exchange.and(cleanup)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_exchange_failure_survives_independent_cleanup_failure() {
        assert_eq!(
            primary_failure(
                Err("fixture_tls_evidence_incomplete"),
                Err("native_exit_or_output_not_clean")
            ),
            Err("fixture_tls_evidence_incomplete")
        );
        assert_eq!(
            primary_failure(Ok(()), Err("broker_output_unknown")),
            Err("broker_output_unknown")
        );
        assert_eq!(
            primary_failure(Err("fixture_tls_timeout"), Ok(())),
            Err("fixture_tls_timeout")
        );
    }
}
