use super::current_hide_failed;
use crate::{BrowserError, remote::State};
use colossus_browser_presentation::{Configure, Lease};
use serde_json::json;
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

fn state() -> Mutex<State> {
    Mutex::new(State {
        configure: serde_json::from_value::<Configure>(json!({
            "session":"bs_11111111111111111111111111111111",
            "target":{"tab_id":"bt_11111111111111111111111111111111",
                "document_id":"bd_11111111111111111111111111111111"},
            "control_generation":0,"viewport_generation":1,
            "width":800,"height":600,"scale_milli":1000,"lease_ms":1500
        }))
        .unwrap(),
        lease: Lease {
            tab: 1,
            session_generation: 1,
            control_generation: 0,
            viewport_generation: 1,
            document_generation: 1,
            pixel_width: 800,
            pixel_height: 600,
        },
        bounds: None,
        applied: None,
        visible: false,
        heartbeat: Instant::now(),
        surface: None,
    })
}

#[tokio::test]
async fn queued_hide_cannot_dispatch_after_a_new_show_or_lease() {
    for transition in 0..3 {
        let state = state();
        let lease = state.lock().unwrap().lease;
        let revision = AtomicU64::new(1);
        let dispatched = AtomicBool::new(false);
        let queued = current_hide_failed(&state, &revision, 1, lease, async {
            dispatched.store(true, Ordering::Release);
            Err(BrowserError::Closed)
        });
        match transition {
            0 => state.lock().unwrap().visible = true,
            1 => revision.store(2, Ordering::Release),
            _ => state.lock().unwrap().lease.viewport_generation = 2,
        }
        assert!(!queued.await);
        assert!(!dispatched.load(Ordering::Acquire));
    }
}

#[tokio::test]
async fn only_a_still_current_hide_failure_requests_channel_disconnect() {
    let state = state();
    let lease = state.lock().unwrap().lease;
    let revision = AtomicU64::new(1);
    assert!(
        current_hide_failed(&state, &revision, 1, lease, async {
            Err(BrowserError::Closed)
        })
        .await
    );
    let dispatched = AtomicBool::new(false);
    assert!(
        !current_hide_failed(&state, &revision, 1, lease, async {
            dispatched.store(true, Ordering::Release);
            tokio::task::yield_now().await;
            // A trusted show supersedes the already-dispatched hide while its
            // acknowledgment is pending. This is not a hide/close success receipt.
            state.lock().unwrap().visible = true;
            revision.store(2, Ordering::Release);
            Err(BrowserError::Closed)
        })
        .await
    );
    assert!(dispatched.load(Ordering::Acquire));
    assert!(state.lock().unwrap().visible);
}
