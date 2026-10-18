use super::initialize_configured;
use crate::{
    BrowserError,
    remote::{Bounds, State},
};
use colossus_browser_presentation::{Configure, Lease};
use serde_json::json;
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

fn configure(viewport: u64) -> Configure {
    serde_json::from_value(json!({
        "session":"bs_11111111111111111111111111111111",
        "target":{"tab_id":"bt_11111111111111111111111111111111",
                  "document_id":"bd_11111111111111111111111111111111"},
        "control_generation":1,"viewport_generation":viewport,
        "width":800,"height":600,"scale_milli":1000,"lease_ms":1500
    }))
    .unwrap()
}
fn lease(viewport: u64) -> Lease {
    Lease {
        tab: 1,
        session_generation: 1,
        control_generation: 1,
        viewport_generation: viewport,
        document_generation: 1,
        pixel_width: 800,
        pixel_height: 600,
    }
}
fn state() -> Mutex<State> {
    let bounds = Some(Bounds {
        x: 0,
        y: 0,
        width: 800,
        height: 600,
        scale_milli: 1000,
    });
    Mutex::new(State {
        configure: configure(1),
        lease: lease(1),
        bounds,
        applied: bounds,
        visible: true,
        heartbeat: Instant::now(),
        surface: None,
    })
}

#[tokio::test]
async fn acknowledged_lease_survives_stale_initialization_and_allows_a_fresh_retry() {
    let state = state();
    let result: Result<(), _> = initialize_configured(&state, configure(2), lease(2), async {
        let current = state.lock().unwrap();
        assert_eq!(current.lease, lease(2));
        assert_eq!(current.configure, configure(2));
        assert!(current.applied.is_none());
        Err(BrowserError::Stale)
    })
    .await;
    assert_eq!(result, Err(BrowserError::Stale));
    assert_eq!(state.lock().unwrap().lease, lease(2));
    initialize_configured(&state, configure(3), lease(3), async {
        assert_eq!(state.lock().unwrap().lease, lease(3));
        Ok(())
    })
    .await
    .unwrap();
    let current = state.lock().unwrap();
    assert_eq!(current.configure, configure(3));
    assert_eq!(current.lease, lease(3));
    assert!(current.visible && current.bounds.is_some() && current.applied.is_none());
}

#[tokio::test]
async fn wrong_or_replayed_ack_cannot_advance_ownership_or_poll_the_initializer() {
    for violation in 0..6 {
        let state = state();
        let mut config = configure(2);
        let mut native = lease(2);
        match violation {
            0 => native.viewport_generation = 1,
            1 => {
                config.session =
                    serde_json::from_value(json!("bs_22222222222222222222222222222222")).unwrap();
            }
            2 => {
                config.control_generation = 2;
                native.control_generation = 2;
            }
            3 => native.session_generation = 2,
            4 => native.tab = 2,
            _ => native.pixel_width = 801,
        }
        let polled = AtomicBool::new(false);
        let result = initialize_configured(&state, config, native, async {
            polled.store(true, Ordering::Release);
            Ok(())
        })
        .await;
        assert_eq!(result, Err(BrowserError::Closed));
        assert!(!polled.load(Ordering::Acquire));
        assert_eq!(state.lock().unwrap().lease, lease(1));
        assert_eq!(state.lock().unwrap().configure, configure(1));
    }
}

#[tokio::test]
async fn ui_failure_keeps_the_acknowledged_lease_without_restoring_visibility() {
    let state = state();
    let result: Result<(), _> = initialize_configured(&state, configure(2), lease(2), async {
        state.lock().unwrap().visible = false;
        Err(BrowserError::Closed)
    })
    .await;
    assert_eq!(result, Err(BrowserError::Closed));
    let current = state.lock().unwrap();
    assert_eq!(current.lease, lease(2));
    assert!(current.applied.is_none() && !current.visible);
}
