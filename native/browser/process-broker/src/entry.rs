//! Dedicated native executable modes, with independently sealed parent entry.

use crate::{
    BrowserCreator, DeploymentSetup, KeeperClient, KeeperServer, OwnerLeases, ProcessReceipt,
    relay::{OutputDrains, Relays},
};
use colossus_darwin_process::{
    DarwinStreamDisconnectObserver, take_connected_creator_channels, take_connected_parent_channel,
};
use std::{
    io::{self, Write as _},
    net::Shutdown,
    os::unix::net::UnixStream,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

include!(concat!(env!("OUT_DIR"), "/process-parent-seal.rs"));

/// Run exactly one finite mode. No runtime path, FD, PID or parent-seal selector.
#[must_use]
pub fn run_native_entry() -> i32 {
    let mode = match std::env::args_os().skip(1).collect::<Vec<_>>().as_slice() {
        [value] if value == "creator" => "creator",
        [value] if value == "keeper" => "keeper",
        _ => return 64,
    };
    let mut phase = "mode_admission";
    let result = match mode {
        "creator" => creator(&mut phase),
        _ => keeper(&mut phase),
    };
    if let Err(error) = result {
        let _ = writeln!(
            io::stderr().lock(),
            "EVIDENCE native-process-stage phase={phase} reason={}",
            diagnostic_reason(&error)
        );
        let _ = writeln!(
            io::stderr().lock(),
            "EVIDENCE native-process-entry mode={mode} kind={:?}",
            error.kind()
        );
        20
    } else {
        0
    }
}

fn keeper(phase: &mut &'static str) -> io::Result<()> {
    *phase = "keeper_admission";
    let (mut entry, parent) = take_connected_parent_channel(EXPECTED_PARENT_CDHASH)?.into_parts();
    setup_timeouts(&entry)?;
    *phase = "keeper_setup";
    let setup = DeploymentSetup::read(&mut entry)?;
    *phase = "keeper_parent_recheck";
    parent.verify()?;
    entry.set_read_timeout(None)?;
    let (lifetime, _retained) = UnixStream::pair()?;
    *phase = "keeper_service";
    KeeperServer::new(setup.nonce)?.serve(entry, lifetime)
}

fn creator(phase: &mut &'static str) -> io::Result<()> {
    *phase = "creator_admission";
    let mut entry = take_connected_creator_channels(EXPECTED_PARENT_CDHASH)?;
    setup_timeouts(&entry.bootstrap)?;
    *phase = "creator_setup";
    let setup = DeploymentSetup::read(&mut entry.bootstrap)?;
    *phase = "creator_parent_recheck";
    entry.parent.verify()?;
    *phase = "creator_keeper_recheck";
    entry.keeper_peer.verify()?;
    #[cfg(feature = "macos-process-output-diagnostic")]
    let diagnostic = crate::private_output::Diagnostic::open(
        &setup.allocation()?.main.executable,
        setup.policy_sha256,
        DIAGNOSTIC_ROOT,
    )?;
    entry.bootstrap.set_read_timeout(None)?;
    // Watch native stream EOF without consuming authenticated application or
    // keeper frames. Each watcher owns a unique internal lifetime endpoint; the
    // browser inherits neither the observer nor either lifetime endpoint.
    let (core_lease, core_observation) = UnixStream::pair()?;
    let (keeper_lease, keeper_observation) = UnixStream::pair()?;
    let relay_failure = core_observation.try_clone()?;
    *phase = "creator_watchers";
    let mut watchers = Watchers::new();
    watchers.add(&entry.data, core_observation)?;
    watchers.add(&entry.keeper, keeper_observation)?;
    *phase = "creator_owner";
    let mut owner = BrowserCreator::new(
        setup.allocation()?,
        KeeperClient::new(entry.keeper, setup.nonce)?,
        OwnerLeases {
            core: core_lease,
            desktop: None,
            keeper: keeper_lease,
        },
    )?;
    let mut relays = Relays::new();
    relays.fence_failures(&relay_failure)?;
    let mut output = None;
    let result = (|| {
        *phase = "main_spawn";
        let main = owner.create_main()?;
        *phase = "main_output";
        output = Some(OutputDrains::start(vec![main.stdout, main.stderr])?);
        *phase = "main_relays";
        relays.connect(entry.bootstrap, main.bootstrap)?;
        relays.connect(entry.data, main.data)?;
        relays.connect(entry.control, main.control)?;
        relays.connect(entry.presentation, main.presentation)?;
        *phase = "main_serve";
        owner.serve()
    })();
    let failed_phase = *phase;
    *phase = "creator_cleanup";
    let cleanup = owner.close();
    let watchers_stopped = watchers.stop();
    let channels_closed = relays.close_and_settle(Duration::from_secs(2)).is_ok();
    let output_drained = output
        .as_mut()
        .is_some_and(|output| output.settle(Duration::from_secs(2)).is_ok());
    let facts = owner.retained_cleanup();
    let receipt = ProcessReceipt {
        nonce: setup.nonce,
        main_status: facts.main_status,
        helpers_reaped: facts.helpers_reaped,
        unknown_creations: facts.unknown_creations,
        keeper_empty: facts.keeper_empty,
        output_drained: output_drained && facts.helper_output_drained,
        channels_closed,
    };
    #[cfg(feature = "macos-process-output-diagnostic")]
    let diagnostics = diagnostic.persist(output.as_ref(), &receipt);
    *phase = "creator_receipt";
    receipt.write(&mut io::stdout().lock())?;
    if !watchers_stopped || !receipt.known_children_closed(setup.nonce) {
        return Err(io::Error::other(
            "independent creator cleanup remains unknown",
        ));
    }
    *phase = if result.is_err() {
        failed_phase
    } else {
        "creator_cleanup"
    };
    #[cfg(feature = "macos-process-output-diagnostic")]
    return result.and(cleanup).and(diagnostics);
    #[cfg(not(feature = "macos-process-output-diagnostic"))]
    result.and(cleanup)
}

fn diagnostic_reason(error: &io::Error) -> &'static str {
    match error.to_string().as_str() {
        "native entry peer is not the exact unprivileged parent" => "peer_parent",
        "native keeper is not an unprivileged sibling incarnation" => "keeper_scope",
        "native keeper does not have the exact live Core parent" => "keeper_parent",
        "native keeper has no genuine stream identity" => "keeper_identity",
        "native keeper stream is disconnected" => "keeper_stream",
        "native keeper credentials differ from stream" => "keeper_credentials",
        "native creator signed information has no CDHash"
        | "native creator CDHash length is invalid"
        | "native creator signed information is not a dictionary" => "self_code_hash",
        "native entry channels have different parent incarnations" => "peer_incarnation",
        "native entry has no kernel peer identity" => "peer_identity",
        "native entry has no kernel peer credentials" => "peer_credentials",
        "native entry is not a connected Unix stream" => "peer_stream",
        "enrolled Darwin audit peer failed signed code validation" => "peer_code",
        "enrolled Darwin audit peer has no matching signed guest" => "peer_guest",
        "enrolled Darwin audit peer changed identity" => "peer_changed",
        "incomplete native deployment" => "setup_incomplete",
        "invalid native deployment header" => "setup_header",
        "invalid native image inventory" => "setup_images",
        "unreviewed helper environment" => "setup_environment",
        "invalid custody marker namespace" => "marker_namespace",
        "invalid custody policy resource" => "marker_policy",
        "custody marker generation changed" => "marker_generation",
        _ => "unclassified",
    }
}

struct Watchers {
    stop: Arc<AtomicBool>,
    workers: Vec<thread::JoinHandle<()>>,
}

impl Watchers {
    fn new() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            workers: Vec::new(),
        }
    }
    fn add(&mut self, stream: &UnixStream, lifetime: UnixStream) -> io::Result<()> {
        self.workers
            .push(watch(stream, lifetime, Arc::clone(&self.stop))?);
        Ok(())
    }
    fn stop(&mut self) -> bool {
        self.stop.store(true, Ordering::Release);
        let mut clean = true;
        for worker in self.workers.drain(..) {
            clean &= worker.join().is_ok();
        }
        clean
    }
}

impl Drop for Watchers {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn setup_timeouts(stream: &UnixStream) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))
}

fn watch(
    stream: &UnixStream,
    lifetime: UnixStream,
    stop: Arc<AtomicBool>,
) -> io::Result<thread::JoinHandle<()>> {
    let mut observer = DarwinStreamDisconnectObserver::register(stream)?;
    thread::Builder::new()
        .name("browser-native-stream-loss".into())
        .spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match observer.wait(Duration::from_millis(100)) {
                    Ok(false) => continue,
                    Ok(true) | Err(_) => {
                        let _ = lifetime.shutdown(Shutdown::Both);
                        return;
                    }
                }
            }
            // Orderly retirement is performed by BrowserCreator before this sender
            // is dropped; it cannot masquerade as a native graceful CEF response.
        })
}
