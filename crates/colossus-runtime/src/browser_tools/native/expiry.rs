//! Retire a native context at its original, immutable network authority deadline.
use super::{lifecycle, types::*};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) fn track(
    browser: &Arc<super::super::RuntimeBrowserTools>,
    entry: &Arc<Entry>,
    deadline: Option<Instant>,
) -> Result<(), RuntimeNativeBrowserError> {
    let Some(deadline) = deadline else {
        return Ok(());
    };
    {
        let mut view = entry
            .view
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
        if let Some(original) = view.expires_at {
            return if original == deadline {
                Ok(())
            } else {
                Err(RuntimeNativeBrowserError::Denied)
            };
        }
        view.expires_at = Some(deadline);
    }
    let browser = Arc::downgrade(browser);
    let entry = Arc::downgrade(entry);
    tokio::spawn(async move {
        loop {
            let Some(entry) = entry.upgrade() else {
                return;
            };
            if entry
                .view
                .lock()
                .is_ok_and(|view| view.closed || view.retired)
            {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let Some(browser) = browser.upgrade() else {
                    return;
                };
                // Retain both cleanup owners after expiry until a positive receipt.
                // A lost viewer or cancelled request cannot abandon this obligation.
                while lifecycle::close_entry(&browser, &entry).await.is_err() {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                return;
            }
            drop(entry);
            tokio::time::sleep(remaining.min(Duration::from_secs(1))).await;
        }
    });
    Ok(())
}
