//! Closed native fixture probes; never renderer IPC or production authority.
use super::{PresentationSurface, ffi};
use crate::BrowserError;
use std::{sync::atomic::Ordering, time::Duration};
use tokio::sync::oneshot;

/// Actual foreground Windows desktop pixels covering this exact native view.
#[derive(Debug)]
pub struct WindowsScreenEvidence {
    /// Physical client width.
    pub width: u32,
    /// Physical client height.
    pub height: u32,
    /// Actual native window DPI.
    pub dpi: u32,
    /// The native parent is the foreground window.
    pub foreground: bool,
    /// The native child is visible.
    pub visible: bool,
    /// Windows hit testing identifies this exact presenter at its center.
    pub hit_test: bool,
    /// Center pixel sampled from the desktop DC.
    pub center_bgra: [u8; 4],
    /// Top-down BGRA bytes captured from the desktop DC, bounded to one viewport.
    pub pixels: Vec<u8>,
}

impl PresentationSurface {
    pub(crate) async fn screen(&self) -> Result<WindowsScreenEvidence, BrowserError> {
        let handle = self.handle.load(Ordering::Acquire);
        let (send, receive) = oneshot::channel();
        self.window
            .run_on_main_thread(move || {
                let result = (|| {
                    let mut probe = ffi::WindowsScreen::default();
                    // SAFETY: live opaque identity on its UI thread and correctly sized output.
                    let status = unsafe {
                        ffi::colossus_presenter_screen_probe(handle, &raw mut probe)
                    };
                    if status != 0 {
                        eprintln!("EVIDENCE Windows screen probe failed status={status}");
                        return Err(BrowserError::Unavailable);
                    }
                    let bytes =
                        usize::try_from(u64::from(probe.width) * u64::from(probe.height) * 4)
                            .ok()
                            .filter(|bytes| {
                                *bytes <= colossus_browser_presentation::MAX_FRAME_BYTES
                            })
                            .ok_or(BrowserError::Unavailable)?;
                    let mut pixels = vec![0; bytes];
                    // SAFETY: bounded owned output and exact dimensions just measured by Windows.
                    let status = unsafe {
                        ffi::colossus_presenter_screen_capture(
                            handle,
                            probe.width,
                            probe.height,
                            pixels.as_mut_ptr(),
                            pixels.len(),
                        )
                    };
                    if status != 0 {
                        eprintln!(
                            "EVIDENCE Windows screen capture failed status={status} foreground={} visible={} hit_test={} width={} height={}",
                            probe.foreground, probe.visible, probe.hit_test, probe.width, probe.height
                        );
                        return Err(BrowserError::Unavailable);
                    }
                    Ok(WindowsScreenEvidence {
                        width: probe.width,
                        height: probe.height,
                        dpi: probe.dpi,
                        foreground: probe.foreground != 0,
                        visible: probe.visible != 0,
                        hit_test: probe.hit_test != 0,
                        center_bgra: probe.center_bgra,
                        pixels,
                    })
                })();
                let _ = send.send(result);
            })
            .map_err(|_| BrowserError::Closed)?;
        tokio::time::timeout(Duration::from_secs(2), receive)
            .await
            .map_err(|_| BrowserError::TimedOut)?
            .map_err(|_| BrowserError::Closed)?
    }

    pub(crate) async fn fixture_input(&self) -> Result<(), BrowserError> {
        let handle = self.handle.load(Ordering::Acquire);
        let (send, receive) = oneshot::channel();
        self.window
            .run_on_main_thread(move || {
                // SAFETY: source-only fixed input sequence for the exact synthetic fixture view.
                let status = unsafe { ffi::colossus_presenter_fixture_input(handle) };
                let _ = send.send(status == 0);
            })
            .map_err(|_| BrowserError::Closed)?;
        tokio::time::timeout(Duration::from_secs(2), receive)
            .await
            .map_err(|_| BrowserError::TimedOut)?
            .map_err(|_| BrowserError::Closed)?
            .then_some(())
            .ok_or(BrowserError::Unavailable)
    }

    pub(crate) async fn fixture_keyboard_reentrancy(&self) -> Result<(), BrowserError> {
        let handle = self.handle.load(Ordering::Acquire);
        let (send, receive) = oneshot::channel();
        self.window
            .run_on_main_thread(move || {
                // SAFETY: closed source-only Return sequence, deriving only this
                // presenter's same-process foreground root on its owning UI thread.
                let status = unsafe { ffi::colossus_presenter_fixture_keyboard_reentrancy(handle) };
                let _ = send.send(status == 0);
            })
            .map_err(|_| BrowserError::Closed)?;
        tokio::time::timeout(Duration::from_secs(3), receive)
            .await
            .map_err(|_| BrowserError::TimedOut)?
            .map_err(|_| BrowserError::Closed)?
            .then_some(())
            .ok_or(BrowserError::Unavailable)
    }
}
