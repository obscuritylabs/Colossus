//! Explicit signed no-CEF native identity/TLS fixture; no command-line selector.
#![forbid(unsafe_code)]
#[cfg(not(all(target_os = "macos", debug_assertions)))]
compile_error!("native custody acceptance requires an explicit Mac Debug build");
#[path = "native_custody_acceptance/enrollment.rs"]
mod enrollment;
use colossus_darwin_process::take_connected_parent_channel;
use colossus_native_browser_tls_broker::{
    ControlExitReason, DrainEvidence, serve_control_with_local_identity,
};
use std::{
    io::{Read as _, Write as _},
    process::ExitCode,
    time::Duration,
};
use zeroize::Zeroizing;
include!(concat!(env!("OUT_DIR"), "/tls-broker-parent-seal.rs"));
fn main() -> ExitCode {
    if std::env::args_os().nth(1).is_some() {
        return fail(10, "fixture_arguments_rejected");
    }
    let channel = match take_connected_parent_channel(EXPECTED_PARENT_CDHASH) {
        Ok(c) => c,
        Err(_) => return fail(11, "fixture_parent_rejected"),
    };
    let (mut stream, parent) = channel.into_parts();
    if stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .is_err()
    {
        return fail(12, "fixture_bootstrap_interrupted");
    }
    let mut material = Zeroizing::new([0; 64]);
    if stream.read_exact(material.as_mut()).is_err() || parent.verify().is_err() {
        return fail(12, "fixture_bootstrap_interrupted");
    }
    let mut key = Zeroizing::new([0; 32]);
    key.copy_from_slice(&material[..32]);
    let mut generation = [0; 32];
    generation.copy_from_slice(&material[32..]);
    drop(material);
    if *key == [0; 32] || generation == [0; 32] || *key == generation {
        return fail(13, "fixture_authentication_rejected");
    }
    let prepared = match enrollment::prepare(&parent, generation) {
        Ok(p) => p,
        Err(error) => return fail(25, error.category()),
    };
    if stream.set_read_timeout(None).is_err() || stream.set_nonblocking(true).is_err() {
        return pre_tls_failure(&prepared, "fixture_transport_interrupted");
    }
    let close = match stream.try_clone() {
        Ok(stream) => stream,
        Err(_) => return pre_tls_failure(&prepared, "fixture_transport_interrupted"),
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(_) => return pre_tls_failure(&prepared, "fixture_runtime_interrupted"),
    };
    let result = runtime.block_on(async {
        let stream = match tokio::net::UnixStream::from_std(stream) {
            Ok(stream) => stream,
            Err(_) => return Service::NoTasks,
        };
        let identity = match prepared.identity(generation) {
            Ok(identity) => identity,
            Err(_) => return Service::NoTasks,
        };
        let served = serve_control_with_local_identity(stream, key, generation, identity);
        tokio::pin!(served);
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                biased;
                _ = interval.tick() => {
                    if parent.verify().is_err() {
                        prepared.revoke();
                        let _ = close.shutdown(std::net::Shutdown::Both);
                        // Closing control requests drainage; keep the service
                        // future until a positive terminal receipt. A timeout
                        // retains store state beneath unobserved TLS tasks.
                        return match tokio::time::timeout(Duration::from_secs(6), &mut served).await {
                            Ok(exit) => Service::Exit(exit, true),
                            Err(_) => Service::Unknown,
                        };
                    }
                }
                exit = &mut served => return Service::Exit(exit, false),
            }
        }
    });
    let (successful, may_retire) = match result {
        Service::NoTasks => (false, true),
        Service::Unknown => (false, false),
        Service::Exit(exit, parent_lost) => (
            !parent_lost
                && exit.reason == ControlExitReason::Shutdown
                && exit.drainage == DrainEvidence::TrackedSocketsClosed,
            matches!(
                exit.drainage,
                DrainEvidence::TrackedSocketsClosed | DrainEvidence::NoLeaseAllocated
            ),
        ),
    };
    if !may_retire {
        return fail(24, "fixture_tls_drainage_unknown_store_retained");
    }
    if prepared.finish().is_err() {
        return fail(24, "fixture_native_retirement_unknown");
    }
    if !successful {
        return fail(26, "fixture_control_failed_after_verified_retirement");
    }
    let _ = writeln!(
        std::io::stdout().lock(),
        "{{\"schema_version\":1,\"native_identity_removed\":true,\"default_and_search_metadata_unchanged\":true,\"cef_loaded\":false,\"production_accepted\":false}}"
    );
    ExitCode::SUCCESS
}
enum Service {
    NoTasks,
    Exit(colossus_native_browser_tls_broker::ControlExit, bool),
    Unknown,
}
fn pre_tls_failure(prepared: &enrollment::Prepared, category: &'static str) -> ExitCode {
    if prepared.finish().is_err() {
        fail(24, "fixture_pre_tls_retirement_unknown")
    } else {
        fail(26, category)
    }
}
fn fail(status: u8, category: &'static str) -> ExitCode {
    let _ = writeln!(
        std::io::stderr().lock(),
        "colossus_native_custody: {category}"
    );
    ExitCode::from(status)
}
