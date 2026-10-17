use colossus_darwin_process::{
    DarwinAuditPeer, DarwinProcessExitObserver, DarwinProcessIdentity,
    spawn_suspended_parent_channel,
};
use colossus_mac_browser_process_broker::ProcessReceipt;
use colossus_mac_supervisor_sentinel_fixture::{
    ROOT,
    contract::Evidence,
    inventory::{Inventory, invalid},
    native, wire,
};
use std::{
    ffi::OsString,
    fs::{self, File},
    io::{self, Read as _, Write as _},
    os::unix::{fs::PermissionsExt as _, process::ExitStatusExt as _},
    path::Path,
    time::{Duration, Instant},
};

fn main() {
    if let Err(error) = run() {
        eprintln!("Supervisor fixture failed: {error}");
        std::process::exit(20)
    }
}
fn run() -> io::Result<()> {
    let case = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [value] if value == "cancel" => ("cancel", 1),
        [value] if value == "eof" => ("eof", 2),
        [value] if value == "parent-crash" => ("parent-crash", 3),
        [value] if value == "parent-crash-output-loss" => ("parent-crash-output-loss", 4),
        _ => return Err(invalid("only fixed supervisor cases are accepted")),
    };
    let inventory = Inventory::load()?;
    let identity = DarwinProcessIdentity::bind(std::process::id())?;
    DarwinAuditPeer::enroll(&identity, inventory.controller_cdhash)?.verify()?;
    let output = Path::new(ROOT).join(format!("case-{}", case.0));
    fs::create_dir(&output)?;
    fs::set_permissions(&output, fs::Permissions::from_mode(0o700))?;
    let mut evidence = Evidence {
        case: case.0.into(),
        accepted: false,
        core_reaped: false,
        core_crashed_without_drop: false,
        core_exit_code: None,
        creator_exit_observed: false,
        keeper_exit_observed: false,
        creator_exit_code: None,
        keeper_exit_code: None,
        creator_exit_signal: None,
        keeper_exit_signal: None,
        creator_reaped_by_core: false,
        keeper_reaped_by_core: false,
        known_children_closed: false,
        helpers_reaped: 0,
        output_eof: false,
        forced: false,
        full_helper_cleanup_proven: false,
        production_enabled: false,
    };
    let result = attempt(case.1, &inventory, &mut evidence, &output);
    evidence.accepted = result.is_ok();
    let target = output.join("result.json");
    fs::write(
        &target,
        serde_json::to_vec_pretty(&evidence).map_err(io::Error::other)?,
    )?;
    fs::set_permissions(target, fs::Permissions::from_mode(0o600))?;
    result
}
fn attempt(
    mode: u8,
    inventory: &Inventory,
    evidence: &mut Evidence,
    output: &Path,
) -> io::Result<()> {
    let environment = inventory
        .setup
        .environment
        .iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    let mut core =
        spawn_suspended_parent_channel(&Path::new(ROOT).join("core"), &[], &environment)?;
    let mut stage = "core_spawn";
    let mut diagnostics = Vec::new();
    let result = attempt_owned(
        mode,
        inventory,
        evidence,
        &mut core,
        &mut stage,
        output,
        &mut diagnostics,
    );
    if let Err(error) = &result {
        eprintln!(
            "EVIDENCE supervisor-controller stage={stage} kind={:?}",
            error.kind()
        );
    }
    // Settle the actual direct Core child and its output on every return path,
    // including admission/readiness failures. Keep failure evidence categorical;
    // no absent receipt or eventual EOF can replace known-roster proof.
    let deadline = Instant::now() + Duration::from_secs(20);
    let settled = loop {
        match core.child.try_wait() {
            Ok(Some(status)) => {
                evidence.core_reaped = true;
                evidence.core_exit_code = status.code();
                evidence.core_crashed_without_drop =
                    matches!(mode, 3 | 4) && status.code() == Some(73) && !evidence.forced;
                break Ok(());
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                evidence.forced = true;
                match core.child.kill_and_reap() {
                    Ok(status) => {
                        evidence.core_reaped = true;
                        evidence.core_exit_code = status.code();
                        break Err(invalid("Core cleanup required force"));
                    }
                    Err(error) => break Err(error),
                }
            }
            Err(error) => break Err(error),
        }
    };
    let stdout = drain(
        core.stdout,
        &output.join("core.stdout.log"),
        Duration::from_secs(2),
    );
    let stderr = drain(
        core.stderr,
        &output.join("core.stderr.log"),
        Duration::from_secs(2),
    );
    let mut diagnostic_eof = true;
    for worker in diagnostics {
        diagnostic_eof &= matches!(worker.join(), Ok(Ok(true)));
    }
    evidence.output_eof =
        matches!(stdout, Ok(true)) && matches!(stderr, Ok(true)) && diagnostic_eof;
    result.and(settled)?;
    if !evidence.facts_accepted() {
        return Err(invalid("known supervisor cleanup unaccepted"));
    }
    Ok(())
}
fn attempt_owned(
    mode: u8,
    inventory: &Inventory,
    evidence: &mut Evidence,
    core: &mut colossus_darwin_process::SpawnedParentChannel,
    stage: &mut &'static str,
    output: &Path,
    diagnostics: &mut Vec<std::thread::JoinHandle<io::Result<bool>>>,
) -> io::Result<()> {
    *stage = "core_admission";
    let core_identity = DarwinProcessIdentity::bind(core.child.pid())?;
    let core_peer = DarwinAuditPeer::enroll(&core_identity, inventory.core_cdhash)?;
    core_peer.verify()?;
    core.child.resume()?;
    core.channel
        .set_read_timeout(Some(Duration::from_secs(30)))?;
    core.channel
        .set_write_timeout(Some(Duration::from_secs(5)))?;
    core.channel.write_all(&[mode])?;
    *stage = "sentinel_ready";
    if wire::read_exact::<8>(&mut core.channel)? != *wire::READY {
        return Err(invalid("Core sentinel not ready"));
    }
    let mut observations = None;
    let mut receipt_reader = None;
    if matches!(mode, 3 | 4) {
        *stage = "crash_observation";
        let creator = u32::from_be_bytes(wire::read_exact(&mut core.channel)?);
        let keeper = u32::from_be_bytes(wire::read_exact(&mut core.channel)?);
        receipt_reader = Some(native::receive_receipt(&core.channel)?);
        let diagnostic_names = if mode == 3 {
            &[
                "keeper.stdout.log",
                "keeper.stderr.log",
                "creator.stderr.log",
            ][..]
        } else {
            &[]
        };
        for name in diagnostic_names {
            let reader = native::receive_receipt(&core.channel)?;
            let destination = output.join(name);
            diagnostics.push(
                std::thread::Builder::new()
                    .name("supervisor-fixture-output".into())
                    .spawn(move || drain(reader, &destination, Duration::from_secs(25)))?,
            );
        }
        let creator = observe(creator, core.child.pid(), inventory.broker_cdhash)?;
        let keeper = observe(keeper, core.child.pid(), inventory.broker_cdhash)?;
        revalidate(&creator.0, core.child.pid())?;
        revalidate(&keeper.0, core.child.pid())?;
        core_peer.verify()?;
        observations = Some((creator, keeper));
    }
    // This absolute cleanup window is shorter than every sentinel body's
    // sixty-second failsafe. Natural failsafe exit cannot become an EOF pass.
    let cleanup_deadline = Instant::now() + Duration::from_secs(20);
    core.channel
        .set_read_timeout(Some(Duration::from_secs(20)))?;
    *stage = "case_ack";
    core.channel.write_all(b"CSACK001")?;
    *stage = "cleanup_result";
    let receipt = if matches!(mode, 3 | 4) {
        if wire::read_exact::<8>(&mut core.channel)? != *b"CEXIT073" {
            return Err(invalid("Core abrupt exit not acknowledged"));
        }
        None
    } else {
        let bytes = wire::receive(&mut core.channel)?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        let statuses: [Option<i32>; 2] =
            serde_json::from_value(value["statuses"].clone()).map_err(io::Error::other)?;
        evidence.creator_reaped_by_core = statuses[0].is_some();
        evidence.keeper_reaped_by_core = statuses[1].is_some();
        evidence.creator_exit_code =
            statuses[0].and_then(|status| std::process::ExitStatus::from_raw(status).code());
        evidence.keeper_exit_code =
            statuses[1].and_then(|status| std::process::ExitStatus::from_raw(status).code());
        let receipt: ProcessReceipt =
            serde_json::from_value(value["receipt"].clone()).map_err(io::Error::other)?;
        evidence.record_receipt(receipt, inventory.setup.nonce);
        if value.get("close_ok") != Some(&serde_json::Value::Bool(true)) {
            return Err(invalid("Core known-child close failed"));
        }
        Some(receipt)
    };
    *stage = "core_exit";
    let deadline = cleanup_deadline;
    let status = loop {
        match core.child.try_wait()? {
            Some(status) => break status,
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            None => {
                evidence.forced = true;
                let _ = core.child.kill_and_reap();
                return Err(invalid("Core exit unknown"));
            }
        }
    };
    evidence.core_reaped = true;
    evidence.core_exit_code = status.code();
    evidence.core_crashed_without_drop = matches!(mode, 3 | 4) && status.code() == Some(73);
    if status.code() != Some(if matches!(mode, 3 | 4) { 73 } else { 0 }) {
        return Err(invalid("Core exit status rejected"));
    }
    *stage = "supervisor_exit";
    let receipt =
        if let Some(((creator, mut creator_exit), (keeper, mut keeper_exit))) = observations {
            let c = creator_exit
                .wait(cleanup_deadline.saturating_duration_since(Instant::now()))?
                .ok_or_else(|| invalid("creator exit unknown"))?;
            let k = keeper_exit
                .wait(cleanup_deadline.saturating_duration_since(Instant::now()))?
                .ok_or_else(|| invalid("keeper exit unknown"))?;
            evidence.creator_exit_observed = c.identity() == creator.identity();
            evidence.keeper_exit_observed = k.identity() == keeper.identity();
            evidence.creator_exit_code = c.status().code();
            evidence.keeper_exit_code = k.status().code();
            evidence.creator_exit_signal = c.status().signal();
            evidence.keeper_exit_signal = k.status().signal();
            wire::receipt(
                receipt_reader
                    .take()
                    .ok_or_else(|| invalid("plain creator receipt missing"))?,
            )?
        } else {
            receipt.ok_or_else(|| invalid("Core receipt missing"))?
        };
    evidence.record_receipt(receipt, inventory.setup.nonce);

    if Instant::now() >= cleanup_deadline {
        return Err(invalid("known supervisor cleanup unaccepted"));
    }
    Ok(())
}
fn observe(
    hint: u32,
    parent: u32,
    cdhash: [u8; 20],
) -> io::Result<(DarwinAuditPeer, DarwinProcessExitObserver)> {
    if hint <= 1 || hint == parent {
        return Err(invalid("invalid direct supervisor hint"));
    }
    let identity = DarwinProcessIdentity::bind(hint)?;
    let peer = DarwinAuditPeer::enroll(&identity, cdhash)?;
    revalidate(&peer, parent)?;
    let observer = DarwinProcessExitObserver::register(&identity)?;
    revalidate(&peer, parent)?;
    Ok((peer, observer))
}
fn revalidate(peer: &DarwinAuditPeer, parent: u32) -> io::Result<()> {
    peer.verify()?;
    if !native::parent_matches(peer.identity().pid(), parent)
        || DarwinProcessIdentity::bind(peer.identity().pid())? != *peer.identity()
    {
        return Err(invalid("supervisor is not exact live Core child"));
    }
    peer.verify()
}
fn drain(file: File, destination: &Path, bound: Duration) -> io::Result<bool> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut capture = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)?;
    let mut captured = 0;
    let flags = rustix::fs::fcntl_getfl(&file)?;
    rustix::fs::fcntl_setfl(&file, flags | rustix::fs::OFlags::NONBLOCK)?;
    let mut reader = file;
    let deadline = Instant::now() + bound;
    let mut buffer = [0; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                let retain = count.min(64 * 1024 - captured);
                capture.write_all(&buffer[..retain])?;
                captured += retain;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(_) => return Ok(false),
        }
    }
}
