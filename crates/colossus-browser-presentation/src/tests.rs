use crate::*;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

fn lease() -> Lease {
    Lease {
        tab: 1,
        session_generation: 2,
        control_generation: 3,
        viewport_generation: 4,
        document_generation: 5,
        pixel_width: 8,
        pixel_height: 8,
    }
}
fn frame(sequence: u64) -> Frame {
    Frame {
        lease: lease(),
        sequence,
        stride: 32,
        pixels: vec![7; 256],
    }
}
fn codec() -> FrameCodec {
    FrameCodec::new(Zeroizing::new([9; 32]), [8; 32], lease()).expect("codec")
}

#[test]
fn authenticated_pixels_round_trip_and_replays_fail() {
    let bytes = codec().encode(&frame(1)).expect("encode");
    let mut reader = codec();
    let decoded = reader.decode(bytes.clone()).expect("decode");
    assert_eq!(decoded.pixels, vec![7; 256]);
    assert_eq!(decoded.lease, lease());
    assert!(reader.decode(bytes).is_err());
}
#[test]
fn every_enrollment_generation_dimension_and_pixel_byte_is_authenticated() {
    let original = codec().encode(&frame(1)).expect("encode");
    for offset in [
        8,
        40,
        48,
        56,
        64,
        72,
        80,
        88,
        92,
        96,
        100,
        104,
        HEADER_BYTES,
    ] {
        let mut modified = original.clone();
        modified[offset] ^= 1;
        assert!(codec().decode(modified).is_err(), "offset {offset}");
    }
    let mut foreign = FrameCodec::new(Zeroizing::new([1; 32]), [8; 32], lease()).expect("foreign");
    assert!(foreign.decode(original).is_err());
}
#[test]
fn oversized_truncated_or_extra_payload_never_reaches_presenter() {
    let bytes = codec().encode(&frame(1)).expect("encode");
    assert!(codec().decode(bytes[..HEADER_BYTES - 1].to_vec()).is_err());
    assert!(codec().decode(bytes[..bytes.len() - 1].to_vec()).is_err());
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(codec().decode(extra).is_err());
    let mut overflow = bytes;
    overflow[100..104].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(codec().payload_length(&overflow[..HEADER_BYTES]).is_err());
}
#[test]
fn latest_queue_bounds_memory_and_drops_superseded_frames() {
    let mut queue = LatestFrames::new(lease()).expect("queue");
    for sequence in 1..=10 {
        queue.push(frame(sequence)).expect("push");
    }
    assert_eq!(queue.retained_bytes(), 512);
    assert_eq!(queue.take_latest().expect("latest").sequence, 10);
    assert_eq!(queue.retained_bytes(), 0);
    assert!(queue.push(frame(9)).is_err());
    let mut next = lease();
    next.viewport_generation += 1;
    queue.replace(next).expect("replacement");
    assert!(queue.push(frame(11)).is_err());
    assert!(queue.replace(lease()).is_err());
}
#[test]
fn overlay_focus_expiry_and_replacement_revoke_native_input() {
    let now = Instant::now();
    let mut guard = LeaseGuard::new(lease(), now, Duration::from_millis(500)).expect("guard");
    let click = Input::MouseButton {
        x: 4,
        y: 4,
        button: 0,
        pressed: true,
    };
    assert!(guard.authorize_input(lease(), &click, now).is_err());
    guard.focus(true, now).expect("focus");
    guard.authorize_input(lease(), &click, now).expect("click");
    assert!(
        guard
            .authorize_input(lease(), &click, now + Duration::from_millis(500))
            .is_err()
    );
    let mut stale = lease();
    stale.control_generation += 1;
    assert!(guard.authorize_input(stale, &click, now).is_err());
    let expired = now + Duration::from_millis(500);
    assert!(
        guard
            .renew(lease(), expired, Duration::from_millis(500))
            .is_err()
    );
    assert!(guard.focus(true, now).is_err());
    guard.focus(false, now).expect("unfocus");
    assert!(guard.authorize_input(lease(), &click, now).is_err());
    guard.hide();
    assert!(guard.focus(true, now).is_err());
    assert!(
        guard
            .renew(lease(), now, Duration::from_millis(500))
            .is_err()
    );
}
#[test]
fn malformed_text_coordinates_and_unbounded_leases_are_rejected() {
    for input in [
        Input::MouseMove { x: 8, y: 0 },
        Input::MouseWheel {
            x: 0,
            y: 0,
            delta_x: i32::MIN,
            delta_y: 0,
        },
        Input::ImeCommit {
            text: "x".repeat(4097),
        },
        Input::ImeCommit { text: "\0".into() },
        Input::Character { text: "ab".into() },
    ] {
        assert!(input.validate(8, 8).is_err());
    }
    assert!(LeaseGuard::new(lease(), Instant::now(), Duration::from_secs(2)).is_err());
    let mut huge = lease();
    huge.pixel_width = 4096;
    huge.pixel_height = 4096;
    assert!(huge.validate().is_err());
}
