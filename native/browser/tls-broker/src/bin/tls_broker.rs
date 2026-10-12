//! Private macOS Debug child entry. No command-line or runtime-file nomination.
#![forbid(unsafe_code)]

#[cfg(not(all(target_os = "macos", debug_assertions)))]
compile_error!("the unaccepted TLS broker entry requires an explicit macOS Debug build");

use colossus_darwin_process::{DarwinParentChannel, take_connected_parent_channel};
use colossus_native_browser_tls_broker::{ControlExitReason, DrainEvidence, serve_control};
use std::{io::Write as _, process::ExitCode, time::Duration};
use tokio::{io::AsyncReadExt as _, net::UnixStream};
use zeroize::Zeroizing;

include!(concat!(env!("OUT_DIR"), "/tls-broker-parent-seal.rs"));

const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(5);
const PARENT_RECHECK: Duration = Duration::from_millis(250);

fn main() -> ExitCode {
    if std::env::args_os().nth(1).is_some() {
        return failure(10, "entry_arguments_rejected");
    }
    let channel = match take_connected_parent_channel(EXPECTED_PARENT_CDHASH) {
        Ok(channel) => channel,
        Err(_) => return failure(11, "parent_admission_rejected"),
    };
    // Consume and admit the fixed inherited descriptor before the async runtime
    // can allocate descriptors. A missing fd3 must not be confused with a runtime
    // descriptor that happened to reuse that number.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return failure(12, "bootstrap_interrupted"),
    };
    runtime.block_on(run(channel))
}

async fn run(channel: DarwinParentChannel) -> ExitCode {
    let (stream, parent) = channel.into_parts();
    if parent.verify().is_err() {
        return failure(11, "parent_admission_rejected");
    }
    if stream.set_nonblocking(true).is_err() {
        return failure(12, "bootstrap_interrupted");
    }
    let close = match stream.try_clone() {
        Ok(close) => close,
        Err(_) => return failure(12, "bootstrap_interrupted"),
    };
    let mut stream = match UnixStream::from_std(stream) {
        Ok(stream) => stream,
        Err(_) => return failure(12, "bootstrap_interrupted"),
    };
    // Nothing is read until fixed-channel kernel provenance and the independently
    // compiled parent code requirement have both been validated. This frame has
    // no peer-supplied PID, expected code hash, path, descriptor or environment.
    let mut material = Zeroizing::new([0_u8; 64]);
    match tokio::time::timeout(BOOTSTRAP_TIMEOUT, stream.read_exact(material.as_mut())).await {
        Ok(Ok(_)) => {}
        _ => return failure(12, "bootstrap_interrupted"),
    }
    if parent.verify().is_err() {
        return failure(11, "parent_admission_rejected");
    }
    let (key, generation) = match authentication(&material) {
        Some(authentication) => authentication,
        None => return failure(13, "bootstrap_authentication_rejected"),
    };
    drop(material);
    #[cfg(feature = "macos-native-custody-development")]
    let custody_authority =
        match colossus_native_browser_mac_identity::NativeMutationAuthority::admit(
            &parent, generation,
        ) {
            Ok(authority) => authority,
            Err(_) => return failure(25, "custody_package_admission_rejected"),
        };
    #[cfg(feature = "macos-native-custody-development")]
    let registry = if custody_authority.registry_digest().is_some() {
        match colossus_native_browser_tls_broker::NativeRegistry::admit(
            custody_authority,
            generation,
        ) {
            Ok(registry) => Some(registry),
            Err(_) => return failure(25, "custody_registry_admission_rejected"),
        }
    } else {
        None
    };
    let mut server = std::pin::pin!(async {
        #[cfg(feature = "macos-native-custody-development")]
        if let Some(registry) = &registry {
            return colossus_native_browser_tls_broker::serve_control_with_native_registry(
                stream, key, generation, registry,
            )
            .await;
        }
        serve_control(stream, key, generation).await
    });
    let mut recheck = tokio::time::interval(PARENT_RECHECK);
    recheck.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut parent_lost = false;
    let exit = loop {
        tokio::select! {
            biased;
            _ = recheck.tick() => {
                if parent.verify().is_err() {
                    parent_lost = true;
                    #[cfg(feature = "macos-native-custody-development")]
                    if let Some(registry) = &registry { registry.revoke(); }
                    let _ = close.shutdown(std::net::Shutdown::Both);
                    // Keep the actual service until all tracked TLS tasks have
                    // positively drained. Timeout leaves owned store obligations.
                    match tokio::time::timeout(Duration::from_secs(6), &mut server).await {
                        Ok(exit) => break exit,
                        Err(_) => return failure(24, "parent_changed_drainage_unknown"),
                    }
                }
            }
            exit = &mut server => break exit,
        }
    };
    if exit.drainage == DrainEvidence::OutcomeUnknown {
        return failure(24, "socket_drainage_unknown");
    }
    #[cfg(feature = "macos-native-custody-development")]
    if let Some(registry) = &registry
        && registry.finish().is_err()
    {
        return failure(24, "native_identity_retirement_unknown");
    }
    if parent_lost {
        return failure(24, "parent_changed_after_verified_drainage");
    }
    match exit.reason {
        ControlExitReason::Shutdown if exit.drainage == DrainEvidence::TrackedSocketsClosed => {
            ExitCode::SUCCESS
        }
        ControlExitReason::ControlClosed => failure(20, "control_closed"),
        ControlExitReason::Expired => failure(21, "lease_expired"),
        ControlExitReason::Rejected => failure(22, "control_rejected"),
        _ => failure(23, "control_interrupted"),
    }
}

fn failure(status: u8, category: &'static str) -> ExitCode {
    // Parent death can close this pipe. A failed diagnostic write must not
    // replace the categorical terminal outcome with a printing panic.
    let _ = writeln!(std::io::stderr().lock(), "colossus_tls_broker: {category}");
    ExitCode::from(status)
}

fn authentication(material: &[u8; 64]) -> Option<(Zeroizing<[u8; 32]>, [u8; 32])> {
    let mut key = Zeroizing::new([0_u8; 32]);
    key.copy_from_slice(&material[..32]);
    let mut generation = [0_u8; 32];
    generation.copy_from_slice(&material[32..]);
    (*key != [0; 32] && generation != [0; 32] && *key != generation).then_some((key, generation))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication_requires_distinct_nonzero_key_and_generation() {
        let mut material = Zeroizing::new([0_u8; 64]);
        assert!(authentication(&material).is_none());
        material[..32].fill(3);
        assert!(authentication(&material).is_none());
        material[32..].fill(3);
        assert!(authentication(&material).is_none());
        material[32..].fill(4);
        let (key, generation) = authentication(&material).unwrap();
        assert_eq!(*key, [3; 32]);
        assert_eq!(generation, [4; 32]);
        material[..32].fill(0);
        assert!(authentication(&material).is_none());
    }
}
