//! Developer-only real CEF fourth-channel acceptance. No release capability proof.
#[path = "presentation_probe/commit.rs"]
#[cfg(unix)]
mod commit;
#[path = "presentation_probe/exercise.rs"]
#[cfg(unix)]
mod exercise;
#[path = "presentation_probe/fixture.rs"]
#[cfg(unix)]
mod fixture;
#[path = "presentation_probe/generation.rs"]
#[cfg(unix)]
mod generation;
#[path = "presentation_probe/handoff.rs"]
#[cfg(unix)]
mod handoff;
#[path = "presentation_probe/launch.rs"]
#[cfg(unix)]
mod launch;
#[path = "presentation_probe/readonly.rs"]
#[cfg(unix)]
mod readonly;
#[path = "presentation_probe/receipt.rs"]
#[cfg(unix)]
mod receipt;
#[path = "presentation_probe/recovery.rs"]
#[cfg(unix)]
mod recovery;

#[cfg(unix)]
fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build();
    let result = runtime
        .map_err(|_| "native probe runtime unavailable")
        .and_then(|runtime| runtime.block_on(exercise::run()));
    match result {
        Ok(()) => println!(
            "{{\"authenticated_presentation\":true,\"actual_bgra\":true,\"native_input\":true,\"stale_input_denied\":true,\"stale_document_denied\":true,\"manual_document_handoff\":true,\"agent_snapshot_document_recovery\":true,\"read_only_document_adoption_denied\":true,\"committed_document_epoch_denied\":true,\"read_only_agent_view\":true,\"cef_shutdown\":true,\"production_containment\":false}}"
        ),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!(
        "the inherited Unix-channel presentation acceptance harness is unavailable on this platform"
    );
    std::process::exit(1);
}
