use super::*;

fn frame(viewport: u64) -> Frame {
    Frame {
        lease: Lease {
            tab: 1,
            session_generation: 1,
            control_generation: 1,
            viewport_generation: viewport,
            document_generation: 1,
            pixel_width: 1,
            pixel_height: 1,
        },
        sequence: 1,
        stride: 4,
        pixels: vec![0; 4],
    }
}

#[test]
fn budget_is_atomic_bounded_and_rejects_overflow() {
    let bytes = AtomicUsize::new(0);
    assert!(!reserve(&bytes, 0, 32));
    assert!(reserve(&bytes, 16, 32));
    assert!(reserve(&bytes, 16, 32));
    assert!(!reserve(&bytes, 1, 32));
    assert!(!reserve(&bytes, usize::MAX, 32));
    assert_eq!(bytes.load(Ordering::Acquire), 32);
}
fn accept(cache: &CompletedCache, viewport: u64) {
    cache.activate(frame(viewport).lease);
    cache.observed(frame(viewport).lease, PageState::default());
    cache.accepted(frame(viewport));
}

#[test]
fn completion_rejects_a_previous_document_or_viewport_and_seals_cache() {
    let cache = CompletedCache::default();
    accept(&cache, 1);
    assert!(cache.take(frame(2).lease).is_none());
    accept(&cache, 2);
    assert!(cache.take(frame(2).lease).is_none());

    let cache = CompletedCache::default();
    accept(&cache, 1);
    let mut next_document = frame(1).lease;
    next_document.document_generation += 1;
    assert!(cache.take(next_document).is_none());
}

#[test]
fn extraction_moves_only_latest_accepted_frame_and_close_clears_it() {
    let cache = CompletedCache::default();
    accept(&cache, 1);
    accept(&cache, 2);
    let accepted = cache.take(frame(2).lease).unwrap();
    assert_eq!(accepted.frame.lease.viewport_generation, 2);
    assert!(cache.take(frame(2).lease).is_none());
    let cache = CompletedCache::default();
    accept(&cache, 1);
    cache.clear();
    assert!(cache.take(frame(1).lease).is_none());
}

#[test]
fn frame_without_matching_authenticated_metadata_is_not_retained() {
    let cache = CompletedCache::default();
    cache.activate(frame(1).lease);
    cache.observed(frame(1).lease, PageState::default());
    cache.accepted(frame(2));
    assert!(cache.take(frame(2).lease).is_none());
}

#[test]
fn old_observations_and_paints_cannot_readmit_a_known_invalid_document() {
    let cache = CompletedCache::default();
    accept(&cache, 1);
    cache.invalidate(frame(1).lease);
    accept(&cache, 1);
    assert!(cache.take(frame(1).lease).is_none());
}

#[test]
fn late_old_results_cannot_clear_or_relabel_a_new_document() {
    let cache = CompletedCache::default();
    accept(&cache, 1);
    cache.invalidate(frame(1).lease);
    accept(&cache, 2);
    cache.observed(
        frame(1).lease,
        PageState {
            title: "Stale".into(),
            ..PageState::default()
        },
    );
    cache.accepted(frame(1));
    cache.invalidate(frame(1).lease);
    assert_eq!(
        cache
            .take(frame(2).lease)
            .unwrap()
            .frame
            .lease
            .viewport_generation,
        2
    );
}

#[test]
fn same_document_metadata_belongs_to_the_frame_that_was_actually_accepted() {
    let cache = CompletedCache::default();
    let lease = frame(1).lease;
    cache.activate(lease);
    cache.observed(
        lease,
        PageState {
            title: "First view".into(),
            url: "https://example.com/first".into(),
            ..PageState::default()
        },
    );
    cache.accepted(frame(1));
    cache.observed(
        lease,
        PageState {
            title: "Later title".into(),
            url: "https://example.com/later".into(),
            ..PageState::default()
        },
    );
    let retained = cache.take(lease).unwrap();
    assert_eq!(retained.page.title, "First view");
    assert_eq!(retained.page.url, "https://example.com/first");
    let cache = CompletedCache::default();
    cache.activate(lease);
    cache.observed(
        lease,
        PageState {
            title: "Later title".into(),
            ..PageState::default()
        },
    );
    cache.accepted(frame(1));
    assert_eq!(cache.take(lease).unwrap().page.title, "Later title");
}

#[test]
fn new_authority_resets_epoch_fence_and_rejects_old_authority_results() {
    let cache = CompletedCache::default();
    let old = frame(90);
    cache.activate(old.lease);
    cache.observed(old.lease, PageState::default());
    cache.invalidate(old.lease);
    let mut current = frame(1);
    current.lease.control_generation = 2;
    cache.activate(current.lease);
    cache.observed(current.lease, PageState::default());
    let expected = current.lease;
    cache.accepted(current);
    cache.invalidate(old.lease);
    cache.observed(
        old.lease,
        PageState {
            title: "Old authority".into(),
            ..PageState::default()
        },
    );
    cache.accepted(old);
    assert_eq!(cache.take(expected).unwrap().frame.lease, expected);
}
