use super::*;

fn reply(callbacks: &Callbacks, tab: u64, generation: u64, command: i32, bytes: &[u8]) {
    // SAFETY: callbacks and payload remain live throughout this synchronous callback.
    unsafe {
        event(
            std::ptr::from_ref(callbacks).cast_mut().cast(),
            tab,
            generation,
            11,
            command,
            1,
            bytes.as_ptr(),
            bytes.len(),
        );
    }
}

#[test]
fn gesture_requires_both_exact_acknowledgements_and_ignores_duplicates() {
    let callbacks = Callbacks::default();
    callbacks.state.lock().unwrap().pending = (10..12)
        .map(|command| Pending {
            tab: 7,
            command,
            result: None,
        })
        .collect();
    reply(&callbacks, 8, 1, 10, b"{}");
    reply(&callbacks, 7, 2, 10, b"{}");
    reply(&callbacks, 7, 1, 9, b"{}");
    assert!(
        callbacks
            .state
            .lock()
            .unwrap()
            .pending
            .iter()
            .all(|pending| pending.result.is_none())
    );
    reply(&callbacks, 7, 1, 11, b"{\"release\":true}");
    assert!(callbacks.state.lock().unwrap().pending[0].result.is_none());
    reply(&callbacks, 7, 1, 11, b"malformed duplicate");
    reply(&callbacks, 7, 1, 10, b"{\"press\":true}");
    let state = callbacks.state.lock().unwrap();
    assert_eq!(state.pending[0].result, Some(Ok(json!({"press":true}))));
    assert_eq!(state.pending[1].result, Some(Ok(json!({"release":true}))));
}

#[test]
fn malformed_matching_acknowledgement_does_not_settle_the_other_half() {
    let callbacks = Callbacks::default();
    callbacks.state.lock().unwrap().pending = (10..12)
        .map(|command| Pending {
            tab: 7,
            command,
            result: None,
        })
        .collect();
    reply(&callbacks, 7, 1, 10, b"invalid");
    let state = callbacks.state.lock().unwrap();
    assert_eq!(
        state.pending[0].result,
        Some(Err(BrowserDriverError::Failed))
    );
    assert!(state.pending[1].result.is_none());
}
