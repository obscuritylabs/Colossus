//! Process-wide retained-pixel accounting across replacement and shared custody.
use super::*;

fn frame(width: u32, height: u32, viewport: u64, sequence: u64) -> Frame {
    Frame {
        lease: Lease {
            tab: 1,
            session_generation: 1,
            control_generation: 1,
            viewport_generation: viewport,
            document_generation: 1,
            pixel_width: width,
            pixel_height: height,
        },
        sequence,
        stride: width * 4,
        pixels: vec![0; usize::try_from(width * height * 4).unwrap()],
    }
}

fn accept(cache: &CompletedCache, frame: Frame) {
    cache.activate(frame.lease);
    cache.observed(
        frame.lease,
        PageState {
            title: frame.sequence.to_string(),
            ..PageState::default()
        },
    );
    cache.accepted(frame);
}

fn assert_cached(cache: &CompletedCache, sequence: u64, width: u32) {
    let state = cache.0.lock().unwrap();
    let frame = state.frame.as_ref().unwrap();
    assert_eq!(frame.frame.sequence, sequence);
    assert_eq!(frame.page.title, sequence.to_string());
    assert_eq!(frame.frame.lease.pixel_width, width);
}

#[test]
fn two_1440p_caches_keep_advancing_within_the_shared_budget() {
    let _exclusive = tests::exclusive_budget();
    let first = CompletedCache::default();
    let second = CompletedCache::default();
    let bytes = 2560 * 1440 * 4;
    for sequence in 1..=3 {
        accept(&first, frame(2560, 1440, 1, sequence));
        accept(&second, frame(2560, 1440, 1, sequence));
        assert_cached(&first, sequence, 2560);
        assert_cached(&second, sequence, 2560);
        assert_eq!(RETAINED_BYTES.load(Ordering::Acquire), 2 * bytes);
    }
    // Even under budget pressure, only exact authenticated metadata may replace
    // a candidate; an unobserved viewport or foreign writer cannot consume its charge.
    first.accepted(frame(2560, 1440, 2, 99));
    let mut foreign = frame(1, 1, 1, 99);
    foreign.lease.control_generation = 2;
    first.accepted(foreign);
    assert_cached(&first, 3, 2560);
    assert_eq!(RETAINED_BYTES.load(Ordering::Acquire), 2 * bytes);
    first.clear();
    second.clear();
    assert_eq!(RETAINED_BYTES.load(Ordering::Acquire), 0);
}

#[test]
fn replacement_charges_only_growth_and_preserves_pixels_when_growth_is_denied() {
    let _exclusive = tests::exclusive_budget();
    let first = CompletedCache::default();
    let second = CompletedCache::default();
    let second_bytes = 2560 * 1440 * 4;
    accept(&first, frame(2560, 1440, 1, 1));
    accept(&second, frame(2560, 1440, 1, 1));
    accept(&first, frame(2880, 1440, 2, 2));
    assert_cached(&first, 2, 2880);
    let grown_bytes = 2880 * 1440 * 4;
    assert_eq!(
        RETAINED_BYTES.load(Ordering::Acquire),
        grown_bytes + second_bytes
    );
    accept(&first, frame(3840, 2160, 3, 3));
    assert_cached(&first, 2, 2880);
    assert_eq!(
        RETAINED_BYTES.load(Ordering::Acquire),
        grown_bytes + second_bytes
    );
    accept(&first, frame(1280, 720, 4, 4));
    assert_cached(&first, 4, 1280);
    assert_eq!(
        RETAINED_BYTES.load(Ordering::Acquire),
        1280 * 720 * 4 + second_bytes
    );
    first.clear();
    second.clear();
    assert_eq!(RETAINED_BYTES.load(Ordering::Acquire), 0);
}

#[test]
fn external_arc_custody_keeps_its_own_charge_until_the_last_owner_drops() {
    let _exclusive = tests::exclusive_budget();
    let first = CompletedCache::default();
    let second = CompletedCache::default();
    let bytes = 2560 * 1440 * 4;
    accept(&first, frame(2560, 1440, 1, 1));
    accept(&second, frame(2560, 1440, 1, 1));
    let external = Arc::clone(first.0.lock().unwrap().frame.as_ref().unwrap());
    accept(&first, frame(2560, 1440, 1, 2));
    assert_cached(&first, 1, 2560);
    assert_eq!(RETAINED_BYTES.load(Ordering::Acquire), 2 * bytes);
    second.clear();
    accept(&first, frame(2560, 1440, 1, 2));
    assert_cached(&first, 2, 2560);
    assert_eq!(external.frame.sequence, 1);
    assert_eq!(external.page.title, "1");
    assert_eq!(RETAINED_BYTES.load(Ordering::Acquire), 2 * bytes);
    accept(&first, frame(2560, 1440, 1, 3));
    assert_cached(&first, 3, 2560);
    first.clear();
    assert_eq!(RETAINED_BYTES.load(Ordering::Acquire), bytes);
    drop(external);
    assert_eq!(RETAINED_BYTES.load(Ordering::Acquire), 0);
}
