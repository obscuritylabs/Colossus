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
    let result = loop {
        while let Ok(queue::Control::Close(session, sender)) = channels.control.try_recv() {
            let result = close_egress().and_then(|()| host.close(&session));
            let _ = sender.send(result);
        }
        if lifetime.finished.load(Ordering::Acquire) {
            break Ok(());
        }
        match channels.data.try_recv() {
            Ok(queue::Data::Open(request, control, sender)) => {
                let _ = sender.send(host.open(request, &control));
            }
            Ok(queue::Data::Execute(command, control, sender)) => {
                let _ = sender.send(host.execute(command, &control));
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
        if lifetime.presentation_revoked.load(Ordering::Acquire) {
            host.revoke_human_presentation();
        }
        for _ in 0..16 {
            match channels.presentation.try_recv() {
                Ok(presentation::Command::Control(command, sender)) => {
                    if sender.is_closed() {
                        continue;
                    }
                    let result = if lifetime.presentation_revoked.load(Ordering::Acquire) {
                        Err(colossus_browser_presentation::PresentationError::Hidden)
                    } else {
                        host.presentation_command(command)
                    };
                    let _ = sender.send(result);
                }
                Ok(presentation::Command::Frame(lease, sender)) => {
                    if sender.is_closed() {
                        continue;
                    }
                    let result = if lifetime.presentation_revoked.load(Ordering::Acquire) {
                        Err(colossus_browser_presentation::PresentationError::Hidden)
                    } else {
                        host.presentation_frame(lease)
                    };
                    let _ = sender.send(result);
                }
                Ok(presentation::Command::Revoke) => host.revoke_human_presentation(),
                Err(_) => break,
            }
        }
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
