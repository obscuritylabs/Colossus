use super::{current_hide_failed, hide_once};
use crate::{BrowserError, remote::State};
use colossus_browser_presentation::{Configure, Lease, PresentationError};
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
        confirmed_hidden_lease: None,
        bounds: None,
        applied: None,
        visible: false,
        heartbeat: Instant::now(),
        surface: None,
    })
}

#[tokio::test]
async fn acknowledged_hide_coalesces_pulses_but_not_a_new_lease() {
    let state = state();
    let first = state.lock().unwrap().lease;
    let dispatched = AtomicU64::new(0);
    for _ in 0..8 {
        hide_once(&state, first, async {
            dispatched.fetch_add(1, Ordering::AcqRel);
            Ok(())
        })
        .await
        .unwrap();
    }
    assert_eq!(dispatched.load(Ordering::Acquire), 1);
    assert_eq!(state.lock().unwrap().confirmed_hidden_lease, Some(first));

    let next = Lease {
        viewport_generation: first.viewport_generation + 1,
        ..first
    };
    state.lock().unwrap().lease = next;
    hide_once(&state, next, async {
        dispatched.fetch_add(1, Ordering::AcqRel);
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(dispatched.load(Ordering::Acquire), 2);
    assert_eq!(state.lock().unwrap().confirmed_hidden_lease, Some(next));
}

#[tokio::test]
async fn rejected_hides_never_count_as_a_positive_ack() {
    for error in [
        PresentationError::OutcomeUnknown,
        PresentationError::Unauthenticated,
        PresentationError::Stale,
        PresentationError::Hidden,
        PresentationError::Expired,
        PresentationError::Invalid,
        PresentationError::LimitExceeded,
    ] {
        let state = state();
        let lease = state.lock().unwrap().lease;
        let dispatched = AtomicBool::new(false);
        assert_eq!(
            hide_once(&state, lease, async {
                dispatched.store(true, Ordering::Release);
                Err(error)
            })
            .await,
            Err(error)
        );
        assert!(dispatched.load(Ordering::Acquire));
        assert_eq!(state.lock().unwrap().confirmed_hidden_lease, None);
    }
}

#[tokio::test]
async fn a_late_hide_ack_cannot_mark_the_replacement_lease() {
    let state = state();
    let first = state.lock().unwrap().lease;
    let next = Lease {
        viewport_generation: first.viewport_generation + 1,
        ..first
    };
    hide_once(&state, first, async {
        tokio::task::yield_now().await;
        state.lock().unwrap().lease = next;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(state.lock().unwrap().confirmed_hidden_lease, None);
    let dispatched = AtomicBool::new(false);
    hide_once(&state, next, async {
        dispatched.store(true, Ordering::Release);
        Ok(())
    })
    .await
    .unwrap();
    assert!(dispatched.load(Ordering::Acquire));
    assert_eq!(state.lock().unwrap().confirmed_hidden_lease, Some(next));
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
