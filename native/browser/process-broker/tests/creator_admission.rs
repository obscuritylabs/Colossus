//! Creator admission checks use only Unix sockets; no native child is launched.
#![cfg(target_os = "macos")]

use colossus_mac_browser_process_broker::{
    BrowserCreator, CreatorAllocation, Frame, HelperImage, HelperRole, KeeperClient, MainImage,
    Message, OwnerDeath, OwnerLeases,
};
use std::{ffi::OsString, os::unix::net::UnixStream, thread, time::Duration};

fn allocation() -> CreatorAllocation {
    CreatorAllocation {
        policy_sha256: [5; 32],
        nonce: [1; 16],
        main: MainImage {
            executable: "/unused/Main".into(),
            cdhash: [2; 20],
            body_sha256: [3; 32],
        },
        helpers: vec![HelperImage {
            role: HelperRole::Renderer,
            executable: "/unused/Renderer".into(),
            cdhash: [4; 20],
            body_sha256: [5; 32],
        }],
        environment: [
            "HOME=/unused/home",
            "XDG_CONFIG_HOME=/unused/config",
            "XDG_DATA_HOME=/unused/data",
            "TMPDIR=/unused/tmp",
            "MAC_CHROMIUM_TMPDIR=/unused/tmp",
            "PATH=/usr/bin:/bin",
            "LANG=en_US.UTF-8",
            "MallocNanoZone=0",
        ]
        .into_iter()
        .map(OsString::from)
        .collect(),
    }
}

#[test]
fn owner_eof_before_main_creation_does_not_attempt_a_native_spawn() {
    let (client, mut server) = UnixStream::pair().expect("keeper channel");
    let keeper = KeeperClient::new(client, [1; 16]).expect("keeper client");
    let responder = thread::spawn(move || {
        while let Ok(frame) = Frame::read(&mut server) {
            assert_eq!(frame.message, Message::ConfirmEmpty);
            Frame {
                sequence: frame.sequence,
                nonce: frame.nonce,
                message: Message::Shutdown {
                    reaped: 0,
                    unknown: 0,
                    keeper_empty: true,
                },
            }
            .write(&mut server)
            .expect("empty keeper reply");
        }
    });
    let (core, core_owner) = UnixStream::pair().expect("core lease");
    let (keeper_lease, _keeper_owner) = UnixStream::pair().expect("keeper lease");
    let mut creator = BrowserCreator::new(
        allocation(),
        keeper,
        OwnerLeases {
            core,
            keeper: keeper_lease,
            desktop: None,
        },
    )
    .expect("creator");
    drop(core_owner);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while creator.retained_cleanup().owner_death.is_none() {
        assert!(std::time::Instant::now() < deadline);
        thread::yield_now();
    }
    assert!(creator.create_main().is_err());
    creator.close().expect("close before creation");
    let facts = creator.retained_cleanup();
    assert_eq!(facts.unknown_creations, 0);
    assert_eq!(facts.main_status, None);
    assert_eq!(facts.helpers_reaped, 0);
    assert!(facts.keeper_empty);
    assert_eq!(facts.owner_death, Some(OwnerDeath::Core));
    assert!(creator.create_main().is_err());
    drop(creator);
    responder.join().expect("keeper responder");
}

#[test]
fn authority_changing_environment_is_rejected_before_main_creation() {
    let mut allocation = allocation();
    allocation
        .environment
        .push("DYLD_INSERT_LIBRARIES=/unused/untrusted.dylib".into());
    let (keeper, _server) = UnixStream::pair().expect("keeper channel");
    let (core, _core_owner) = UnixStream::pair().expect("core lease");
    let (keeper_lease, _keeper_owner) = UnixStream::pair().expect("keeper lease");
    assert!(
        BrowserCreator::new(
            allocation,
            KeeperClient::new(keeper, [1; 16]).expect("keeper client"),
            OwnerLeases {
                core,
                keeper: keeper_lease,
                desktop: None,
            },
        )
        .is_err()
    );
}
