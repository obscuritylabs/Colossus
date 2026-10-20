//! Native failure receipts remain failures after loading ends.
use super::*;

fn receipt(callbacks: &Callbacks, tab: u64, kind: u32, success: i32) {
    // SAFETY: callbacks remain owned throughout the synchronous empty-payload event.
    unsafe {
        event(
            std::ptr::from_ref(callbacks).cast_mut().cast(),
            tab,
            1,
            kind,
            0,
            success,
            std::ptr::null(),
            0,
        );
    }
}

#[test]
fn native_failure_during_navigation_cannot_become_success_when_loading_ends() {
    for failure in [4, 9, 10] {
        let callbacks = Callbacks::default();
        receipt(&callbacks, 7, 1, 1);
        receipt(&callbacks, 7, 2, 1);
        receipt(&callbacks, 7, failure, 0);
        {
            let state = callbacks.state.lock().unwrap();
            assert_eq!(
                state.tab_outcome(7),
                Err(BrowserDriverError::OutcomeUnknown)
            );
        }
        // CEF emits loading=false even for a certificate or main-frame error.
        receipt(&callbacks, 7, 2, 0);
        let state = callbacks.state.lock().unwrap();
        assert!(!state.loading.contains(&7));
        assert_eq!(state.completed.get(&7), Some(&1));
        assert_eq!(
            state.tab_outcome(7),
            Err(BrowserDriverError::OutcomeUnknown)
        );
    }
}

#[test]
fn successful_load_and_another_tabs_failure_preserve_the_owned_tabs_outcome() {
    let callbacks = Callbacks::default();
    receipt(&callbacks, 7, 1, 1);
    receipt(&callbacks, 7, 2, 1);
    receipt(&callbacks, 8, 9, 0);
    receipt(&callbacks, 7, 2, 0);
    let state = callbacks.state.lock().unwrap();
    assert_eq!(state.tab_outcome(7), Ok(()));
    assert_eq!(state.completed.get(&7), Some(&1));
    assert_eq!(
        state.tab_outcome(8),
        Err(BrowserDriverError::OutcomeUnknown)
    );
}
