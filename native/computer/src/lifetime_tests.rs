use super::*;
use std::cell::Cell;

#[test]
fn admission_rejects_first_inspected_window_destroyed_and_recycled_before_drain() {
    let table = Table::new();
    let current = Cell::new('A');
    let inspected = Cell::new(None);
    // A and B share the handle, process birth and geometry. Only the ordered
    // destroy event distinguishes their lifetimes. Inject it in the admission
    // seam actually used before publishing window and owned-modal metadata.
    let result = table.admit(
        713,
        || {
            let facts = (current.get(), 42, (0, 0, 800, 600));
            inspected.set(Some(facts));
            Ok(facts)
        },
        || {
            table.destroyed(713);
            current.set('B');
        },
    );
    assert!(matches!(result, Err(Error::Stale)));
    assert_eq!(inspected.get().unwrap().0, 'A');
    let fresh = table
        .admit(713, || Ok((current.get(), 42, (0, 0, 800, 600))), || {})
        .unwrap();
    assert_eq!(fresh.value.0, 'B');
    assert!(table.alive(fresh.stamp, 713));
}

#[test]
fn released_provisional_seal_and_old_fence_never_revive_after_slot_reuse() {
    let table = Arc::new(Table::new());
    let old = table.admit(713, || Ok(()), || {}).unwrap().stamp;
    let old_fence = Fence {
        table: Arc::clone(&table),
        root: (old, 713),
        target: (old, 713),
    };
    table.destroyed(713);
    let replacement = table.admit(713, || Ok(()), || {}).unwrap().stamp;
    assert_ne!(old, replacement);
    assert!(!old_fence.valid());
    assert!(table.alive(replacement, 713));
    table.release(replacement, 713);
    let reused = table.admit(713, || Ok(()), || {}).unwrap().stamp;
    assert_ne!(replacement, reused);
    assert!(!table.alive(replacement, 713));
    assert!(!old_fence.valid());
    assert!(table.alive(reused, 713));
}

#[test]
fn repeated_listing_and_denied_candidates_do_not_exhaust_lifetime_slots() {
    let table = Table::new();
    let admitted = table.admit(1, || Ok(()), || {}).unwrap().stamp;
    for handle in 2..=(CAPACITY as u64 * 4) {
        assert!(matches!(
            table.admit(handle, || Err::<(), _>(Error::Denied), || {}),
            Err(Error::Denied)
        ));
        let repeated = table.admit(1, || Ok(()), || {}).unwrap().stamp;
        assert_eq!(admitted, repeated);
    }
    for handle in 2..=CAPACITY as u64 {
        table.admit(handle, || Ok(()), || {}).unwrap();
    }
    assert!(matches!(
        table.admit(7000, || Ok(()), || {}),
        Err(Error::Busy)
    ));
    assert!(table.alive(admitted, 1));
}

#[test]
fn failed_reinspection_does_not_release_an_existing_published_lifetime() {
    let table = Table::new();
    let published = table.admit(1, || Ok(()), || {}).unwrap().stamp;
    assert!(matches!(
        table.admit(1, || Err::<(), _>(Error::Unavailable), || {}),
        Err(Error::Unavailable)
    ));
    assert!(table.alive(published, 1));
    table.destroyed(1);
    assert!(!table.alive(published, 1));
}

#[test]
fn reader_cannot_mix_old_generation_with_recycled_slots_clear_destroyed_flag() {
    let table = Table::new();
    let old = table.admit(713, || Ok(()), || {}).unwrap().stamp;
    let replacement = Cell::new(None);
    let alive = table.alive_during(old, 713, || {
        // Pause after handle/gen1, then publish the same handle as gen2 with
        // destroyed=false. A final generation read must still reject gen1.
        table.destroyed(713);
        replacement.set(Some(table.track(713).unwrap().0));
    });
    assert!(!alive);
    assert_ne!(old, replacement.get().unwrap());
    assert!(table.alive(replacement.get().unwrap(), 713));
}
