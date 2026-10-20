//! The native UI-thread actor shared by platform-specific private channel entries.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use colossus_ports::BrowserDriverError;

use crate::{cef, presentation, queue};

pub struct Channels {
    pub data: tokio::sync::mpsc::Receiver<queue::Data>,
    pub control: tokio::sync::mpsc::Receiver<queue::Control>,
    pub presentation: tokio::sync::mpsc::Receiver<presentation::Command>,
}
pub struct Lifetime {
    pub finished: Arc<AtomicBool>,
    pub presentation_revoked: Arc<AtomicBool>,
    pub cancelled: Arc<AtomicBool>,
}

/// Run only on CEF's original bootstrap thread. Close acknowledgements require
/// the platform egress closure and actual native browser shutdown to settle.
pub fn pump(
    host: &mut cef::Host,
    mut channels: Channels,
    lifetime: Lifetime,
    mut close_egress: impl FnMut() -> Result<(), BrowserDriverError>,
) -> Result<(), BrowserDriverError> {
    host.install_presentation_lane(presentation::Lane::new(
        channels.presentation,
        Arc::clone(&lifetime.presentation_revoked),
    ));
    let result = loop {
        while let Ok(command) = channels.control.try_recv() {
            match command {
                queue::Control::Close(session, sender) => {
                    let result = close_egress().and_then(|()| host.close(&session));
                    let _ = sender.send(result);
                }
                queue::Control::Suspend(session, sender) => {
                    let _ = sender.send(host.suspend(&session));
                }
                queue::Control::Resume(session, generation, sender) => {
                    let _ = sender.send(host.resume(&session, generation));
                }
            }
        }
        if lifetime.finished.load(Ordering::Acquire) {
            break Ok(());
        }
        match channels.data.try_recv() {
            Ok(queue::Data::Open(request, control, sender)) => {
                let _ = sender.send(host.open(request, &control));
            }
            Ok(queue::Data::Execute(command, control, sender)) => {
                let action = command.action.kind();
                let result = host.execute(command, &control);
                crate::execution_diagnostic::trace(
                    crate::execution_diagnostic::Stage::Host,
                    action,
                    &result,
                );
                let _ = sender.send(result);
            }
            Ok(queue::Data::Capture(command, control, sender)) => {
                let _ = sender.send(host.capture(command, &control));
            }
            Ok(queue::Data::ReadScreenshot(request, control, sender)) => {
                let _ = sender.send(host.read_screenshot_chunk(request, &control));
            }
            Ok(queue::Data::ConfirmHandoff(request, control, sender)) => {
                let _ = sender.send(host.confirm_native_handoff(request, &control));
            }
            Ok(queue::Data::PrepareUpload(request, control, sender)) => {
                let _ = sender.send(host.prepare_upload(request, &control));
            }
            Ok(queue::Data::WriteUpload(request, control, sender)) => {
                let _ = sender.send(host.write_upload_chunk(request, &control));
            }
            Ok(queue::Data::CommitUpload(request, control, sender)) => {
                let _ = sender.send(host.commit_upload(request, &control));
            }
            Ok(queue::Data::Download(command, control, sender)) => {
                let _ = sender.send(host.download(command, &control));
            }
            Ok(queue::Data::ReadDownload(request, control, sender)) => {
                let _ = sender.send(host.read_download_chunk(request, &control));
            }
            Err(_) => {}
        }
        host.service_presentation(false);
        if let Err(error) = host.idle_pump() {
            break Err(error);
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    lifetime.cancelled.store(true, Ordering::Release);
    close_egress()?;
    host.shutdown()?;
    result
}
