//! Unix native-lease socket tests; no CEF, child spawn, audit or launchd.
#![cfg(unix)]

use colossus_mac_browser_process_broker::{DeathFence, Frame, OwnerDeath, OwnerLeases};
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    thread,
    time::{Duration, Instant},
};

fn pair() -> (UnixStream, UnixStream) {
    UnixStream::pair().expect("socket pair")
}

// Observe the persistent EOF state instead of racing a first blocking recv
// against Darwin's shutdown notification. The original two-second bound stays
// absolute; a timeout, data byte, or I/O failure never counts as EOF.
fn assert_browser_eof(stream: &mut UnixStream) {
    stream
        .set_nonblocking(true)
        .expect("nonblocking EOF observer");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        assert!(
            Instant::now() < deadline,
            "browser EOF not observed before deadline"
        );
        match stream.read(&mut [0]) {
            Ok(0) => return,
            Ok(_) => panic!("browser emitted unexpected data before EOF"),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("browser EOF: {error}"),
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "browser EOF not observed before deadline"
        );
        let timeout = rustix::event::Timespec::try_from(remaining).expect("EOF deadline");
        let mut wait = [rustix::event::PollFd::new(
            &*stream,
            rustix::event::PollFlags::IN,
        )];
        match rustix::event::poll(&mut wait, Some(&timeout)) {
            Ok(_) | Err(rustix::io::Errno::INTR) => {}
            Err(error) => panic!("browser EOF readiness: {error}"),
        }
    }
}

#[test]
fn panicking_admitted_action_cannot_disable_owner_eof_cleanup() {
    let (mut client, server) = pair();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let (core_reader, core_owner) = pair();
    let (keeper_reader, _keeper_owner) = pair();
    let mut watchers = DeathFence::start(
        OwnerLeases {
            core: core_reader,
            keeper: keeper_reader,
            desktop: None,
        },
        &server,
    )
    .expect("watchers");
    let fence = watchers.creation_fence();
    assert!(
        std::panic::catch_unwind(|| fence.admit(|| -> std::io::Result<()> {
            panic!("failed finite action");
        }))
        .is_err()
    );
    assert!(fence.admit(|| Ok(())).is_err());
    drop(core_owner);
    assert_browser_eof(&mut client);
    assert_eq!(fence.owner_death(), Some(OwnerDeath::Core));
    watchers.retire().expect("retire poisoned admission");
}

#[test]
fn observed_owner_loss_and_native_admission_are_serialized() {
    let (mut client, server) = pair();
    client
        .set_read_timeout(Some(Duration::from_millis(30)))
        .expect("bounded observation");
    let (core_reader, core_owner) = pair();
    let (keeper_reader, _keeper_owner) = pair();
    let mut watchers = DeathFence::start(
        OwnerLeases {
            core: core_reader,
            keeper: keeper_reader,
            desktop: None,
        },
        &server,
    )
    .expect("watchers");
    let fence = watchers.creation_fence();
    let (entered, admitted) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let action_fence = fence.clone();
    let action = thread::spawn(move || {
        action_fence.admit(|| {
            entered.send(()).expect("admission acquired");
            released.recv().expect("finish finite action");
            assert_eq!(action_fence.owner_death(), None);
            Ok(())
        })
    });
    admitted.recv().expect("action entered before EOF");
    drop(core_owner);
    let mut byte = [0];
    let pending = client
        .read(&mut byte)
        .expect_err("EOF publication waits for action");
    assert!(matches!(
        pending.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ));
    release.send(()).expect("finish admission");
    action.join().expect("admitted action").expect("action");
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("EOF deadline");
    assert_browser_eof(&mut client);
    let mut ran = false;
    assert!(
        fence
            .admit(|| {
                ran = true;
                Ok(())
            })
            .is_err()
    );
    assert!(!ran);
    assert_eq!(fence.owner_death(), Some(OwnerDeath::Core));
    watchers.retire().expect("retire");
}

#[test]
fn owner_loss_before_main_creation_rejects_and_closes_late_stream() {
    let (core_reader, core_owner) = pair();
    let (keeper_reader, _keeper_owner) = pair();
    let mut watchers = DeathFence::start_before_creation(OwnerLeases {
        core: core_reader,
        keeper: keeper_reader,
        desktop: None,
    })
    .expect("precreation watchers");
    let fence = watchers.creation_fence();
    drop(core_owner);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while fence.check().is_ok() {
        assert!(
            std::time::Instant::now() < deadline,
            "owner EOF not observed"
        );
        thread::yield_now();
    }
    let (mut client, server) = pair();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    assert!(watchers.attach_wakeup(&server).is_err());
    assert_browser_eof(&mut client);
    assert_eq!(fence.owner_death(), Some(OwnerDeath::Core));
    watchers.retire().expect("retire");
}

#[test]
fn main_channel_attachment_racing_owner_loss_cannot_miss_wakeup() {
    for _ in 0..32 {
        let (core_reader, core_owner) = pair();
        let (keeper_reader, _keeper_owner) = pair();
        let mut watchers = DeathFence::start_before_creation(OwnerLeases {
            core: core_reader,
            keeper: keeper_reader,
            desktop: None,
        })
        .expect("precreation watchers");
        let (mut client, server) = pair();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");
        let death = thread::spawn(move || drop(core_owner));
        let _attached = watchers.attach_wakeup(&server);
        death.join().expect("owner loss");
        assert_browser_eof(&mut client);
        assert_eq!(
            watchers.creation_fence().owner_death(),
            Some(OwnerDeath::Core)
        );
        watchers.retire().expect("retire");
    }
}

fn owner_death_closes_idle_browser(cause: OwnerDeath) {
    let (mut client, server) = pair();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let (core_reader, core_owner) = pair();
    let (keeper_reader, keeper_owner) = pair();
    let (desktop_reader, desktop_owner) = pair();
    let mut watchers = DeathFence::start(
        OwnerLeases {
            core: core_reader,
            keeper: keeper_reader,
            desktop: Some(desktop_reader),
        },
        &server,
    )
    .expect("watchers");
    let fence = watchers.creation_fence();
    assert!(fence.check().is_ok());
    let mut core = Some(core_owner);
    let mut keeper = Some(keeper_owner);
    let mut desktop = Some(desktop_owner);
    match cause {
        OwnerDeath::Core => drop(core.take()),
        OwnerDeath::Keeper => drop(keeper.take()),
        OwnerDeath::Desktop => drop(desktop.take()),
    }
    assert_browser_eof(&mut client);
    assert_eq!(fence.owner_death(), Some(cause));
    assert!(fence.check().is_err());
    watchers.retire().expect("retire");
    assert_eq!(fence.owner_death(), Some(cause));
}

#[test]
fn core_parent_eof_fences_and_wakes_idle_creator() {
    owner_death_closes_idle_browser(OwnerDeath::Core);
}
#[test]
fn desktop_parent_eof_fences_and_wakes_idle_creator() {
    owner_death_closes_idle_browser(OwnerDeath::Desktop);
}
#[test]
fn keeper_peer_eof_fences_and_wakes_idle_creator() {
    owner_death_closes_idle_browser(OwnerDeath::Keeper);
}

#[test]
fn keeper_death_interrupts_a_half_received_browser_frame() {
    let (mut client, mut server) = pair();
    let (core_reader, _core_owner) = pair();
    let (keeper_reader, keeper_owner) = pair();
    let mut watchers = DeathFence::start(
        OwnerLeases {
            core: core_reader,
            keeper: keeper_reader,
            desktop: None,
        },
        &server,
    )
    .expect("watchers");
    let fence = watchers.creation_fence();
    client.write_all(b"CBROK001").expect("partial header");
    let reader = thread::spawn(move || Frame::read(&mut server));
    drop(keeper_owner);
    assert!(reader.join().expect("reader thread").is_err());
    assert_eq!(fence.owner_death(), Some(OwnerDeath::Keeper));
    watchers.retire().expect("retire");
}

#[test]
fn orderly_retire_keeps_the_acknowledgement_channel_open() {
    let (mut client, mut server) = pair();
    let (core_reader, _core_owner) = pair();
    let (keeper_reader, _keeper_owner) = pair();
    let mut watchers = DeathFence::start(
        OwnerLeases {
            core: core_reader,
            keeper: keeper_reader,
            desktop: None,
        },
        &server,
    )
    .expect("watchers");
    let fence = watchers.creation_fence();
    watchers.retire().expect("retire");
    assert!(fence.check().is_err());
    assert_eq!(fence.owner_death(), None);
    server
        .write_all(b"cleanup acknowledged")
        .expect("acknowledgement");
    let mut bytes = [0; 20];
    client.read_exact(&mut bytes).expect("read acknowledgement");
    assert_eq!(&bytes, b"cleanup acknowledged");
}

#[test]
fn retirement_racing_reader_start_is_bounded_with_live_owner_leases() {
    for _ in 0..64 {
        let (mut client, mut server) = pair();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("acknowledgement deadline");
        let (core_reader, _core_owner) = pair();
        let (keeper_reader, _keeper_owner) = pair();
        let (desktop_reader, _desktop_owner) = pair();
        let mut watchers = DeathFence::start(
            OwnerLeases {
                core: core_reader,
                keeper: keeper_reader,
                desktop: Some(desktop_reader),
            },
            &server,
        )
        .expect("watchers");
        let fence = watchers.creation_fence();
        let (completed, completion) = std::sync::mpsc::sync_channel(1);
        let retirement = thread::spawn(move || {
            completed
                .send(watchers.retire())
                .expect("retirement result");
        });
        completion
            .recv_timeout(Duration::from_secs(2))
            .expect("retirement must not wait for live owners to close")
            .expect("retire");
        retirement.join().expect("retirement thread");
        assert!(fence.check().is_err());
        assert_eq!(fence.owner_death(), None);
        server
            .write_all(b"cleanup acknowledged")
            .expect("acknowledgement after bounded retirement");
        let mut bytes = [0; 20];
        client.read_exact(&mut bytes).expect("read acknowledgement");
        assert_eq!(&bytes, b"cleanup acknowledged");
    }
}

#[test]
fn idle_live_leases_do_not_revoke_authority_on_read_timeout() {
    let (mut client, server) = pair();
    client
        .set_read_timeout(Some(Duration::from_millis(350)))
        .expect("live lease observation deadline");
    let (core_reader, _core_owner) = pair();
    let (keeper_reader, _keeper_owner) = pair();
    let mut watchers = DeathFence::start(
        OwnerLeases {
            core: core_reader,
            keeper: keeper_reader,
            desktop: None,
        },
        &server,
    )
    .expect("watchers");
    let pending = client
        .read(&mut [0])
        .expect_err("idle live leases must not close the browser");
    assert!(matches!(
        pending.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ));
    let fence = watchers.creation_fence();
    assert!(fence.check().is_ok());
    assert_eq!(fence.owner_death(), None);
    watchers.retire().expect("retire idle readers");
}

#[test]
fn unexpected_lease_data_revokes_instead_of_renewing_authority() {
    let (mut client, server) = pair();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let (core_reader, mut core_owner) = pair();
    let (keeper_reader, _keeper_owner) = pair();
    let mut watchers = DeathFence::start(
        OwnerLeases {
            core: core_reader,
            keeper: keeper_reader,
            desktop: None,
        },
        &server,
    )
    .expect("watchers");
    let fence = watchers.creation_fence();
    core_owner
        .write_all(b"still alive")
        .expect("unexpected bytes");
    assert_browser_eof(&mut client);
    assert_eq!(fence.owner_death(), Some(OwnerDeath::Core));
    watchers.retire().expect("retire");
}

#[test]
fn simultaneous_owner_loss_has_one_sticky_fence() {
    for _ in 0..64 {
        let (mut client, server) = pair();
        let (core_reader, core_owner) = pair();
        let (keeper_reader, keeper_owner) = pair();
        let mut watchers = DeathFence::start(
            OwnerLeases {
                core: core_reader,
                keeper: keeper_reader,
                desktop: None,
            },
            &server,
        )
        .expect("watchers");
        let fence = watchers.creation_fence();
        let loss = std::sync::Arc::new(std::sync::Barrier::new(3));
        let core_loss = loss.clone();
        let core = thread::spawn(move || {
            core_loss.wait();
            drop(core_owner);
        });
        let keeper_loss = loss.clone();
        let keeper = thread::spawn(move || {
            keeper_loss.wait();
            drop(keeper_owner);
        });
        loss.wait();
        assert_browser_eof(&mut client);
        core.join().expect("Core owner closed");
        keeper.join().expect("keeper owner closed");
        let first = fence.owner_death();
        assert!(matches!(first, Some(OwnerDeath::Core | OwnerDeath::Keeper)));
        watchers.retire().expect("retire");
        assert_eq!(fence.owner_death(), first);
    }
}
