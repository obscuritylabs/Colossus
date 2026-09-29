//! Test-only `WebKit` probes, excluded from ordinary Desktop builds and IPC.

use std::{cell::RefCell, time::Duration};

use block2::RcBlock;
use objc2::{AllocAnyThread, runtime::AnyObject};
use objc2_foundation::{
    MainThreadMarker, NSError, NSJSONSerialization, NSJSONWritingOptions, NSString,
    NSUTF8StringEncoding,
};
use objc2_web_kit::WKWebView;
use tauri::Webview;

use crate::BrowserError;

/// Evaluate a static acceptance probe through `WebKit`'s own completion callback.
///
/// The hardened guest replaces Wry's navigation delegate, so Wry never drains
/// its initial script queue. Its generic evaluation API drops these callbacks.
/// This test-only adapter leaves the hardened delegates and script policy intact.
///
/// # Errors
/// Rejects a closed guest, JavaScript/serialization failure, or a missing reply.
pub async fn evaluate(view: &Webview, script: &'static str) -> Result<String, BrowserError> {
    let (send, receive) = tokio::sync::oneshot::channel();
    view.with_webview(move |native| {
        if MainThreadMarker::new().is_none() {
            let _ = send.send(Err(BrowserError::Unavailable));
            return;
        }
        let send = RefCell::new(Some(send));
        let callback = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
            let result = serialize(value, error);
            if let Some(send) = send.borrow_mut().take() {
                let _ = send.send(result);
            }
        });
        // SAFETY: Tauri owns this live WKWebView on the verified main thread.
        // WebKit copies the completion block and owns its lifetime until reply.
        unsafe {
            let view = &*native.inner().cast::<WKWebView>();
            view.evaluateJavaScript_completionHandler(&NSString::from_str(script), Some(&callback));
        }
    })
    .map_err(|_| BrowserError::Closed)?;
    tokio::time::timeout(Duration::from_secs(5), receive)
        .await
        .map_err(|_| BrowserError::TimedOut)?
        .map_err(|_| BrowserError::Closed)?
}

fn serialize(value: *mut AnyObject, error: *mut NSError) -> Result<String, BrowserError> {
    if !error.is_null() || value.is_null() {
        return Err(BrowserError::Unavailable);
    }
    // SAFETY: WebKit supplies a live bridged JavaScript result for this callback.
    // JSON serialization accepts its scalar/collection values with fragments enabled.
    let data = unsafe {
        NSJSONSerialization::dataWithJSONObject_options_error(
            &*value,
            NSJSONWritingOptions::FragmentsAllowed,
        )
    }
    .map_err(|_| BrowserError::Unavailable)?;
    NSString::initWithData_encoding(NSString::alloc(), &data, NSUTF8StringEncoding)
        .map(|value| value.to_string())
        .ok_or(BrowserError::Unavailable)
}
