//! Pure metadata checks; no descriptors or native audit-device calls.

use super::*;

fn descriptor() -> DeviceMetadata {
    DeviceMetadata {
        dev: 1,
        ino: 2,
        rdev: 0x1200_0001,
        mode: u32::from(libc::S_IFCHR) | 0o644,
        flags: (libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW) as u32,
    }
}

#[test]
fn cloned_minor_preserves_devnode_provenance_and_exact_open_descriptor() {
    let expected = descriptor();
    let native = DeviceMetadata {
        rdev: 0x1200_0002,
        flags: 0,
        ..expected
    };
    assert!(matches_device(expected, expected, native));
    assert!(!matches_device(
        DeviceMetadata {
            rdev: native.rdev,
            ..expected
        },
        expected,
        native
    ));
    for changed in [
        DeviceMetadata { dev: 3, ..native },
        DeviceMetadata { ino: 3, ..native },
        DeviceMetadata {
            rdev: 0x1300_0002,
            ..native
        },
        DeviceMetadata {
            mode: u32::from(libc::S_IFREG) | 0o644,
            ..native
        },
    ] {
        assert!(!matches_device(expected, expected, changed));
    }
}

#[test]
fn exact_metadata_cannot_authorize_writable_blocking_or_mutating_flags() {
    let native = descriptor();
    for flags in [
        libc::O_WRONLY | libc::O_NONBLOCK,
        libc::O_RDWR | libc::O_NONBLOCK,
        libc::O_RDONLY,
        libc::O_NONBLOCK | libc::O_APPEND,
        libc::O_NONBLOCK | libc::O_ASYNC,
        libc::O_NONBLOCK | libc::O_CREAT,
        libc::O_NONBLOCK | libc::O_SHLOCK,
    ] {
        let bad = DeviceMetadata {
            flags: flags as u32,
            ..descriptor()
        };
        assert!(!matches_device(bad, bad, native));
    }
    let expected = descriptor();
    assert!(!matches_device(
        DeviceMetadata {
            flags: (libc::O_RDONLY | libc::O_NONBLOCK) as u32,
            ..expected
        },
        expected,
        native
    ));
}
