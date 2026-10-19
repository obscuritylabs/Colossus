use super::*;
use std::sync::atomic::{AtomicU64, AtomicUsize};
use windows_sys::Win32::Foundation::{GetHandleInformation, HANDLE_FLAG_INHERIT};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn id() -> String {
    format!(
        "colossus.instance.{}.{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

#[test]
fn bounded_names_bind_logon_and_reject_namespace_injection() {
    for invalid in ["", "Global\\victim", "nul\0suffix", "app/name", "a b"] {
        assert!(Names::new(invalid, 7).is_err());
    }
    assert!(Names::new(&"a".repeat(129), 7).is_err());
    let first = Names::new(&"a".repeat(128), 7).unwrap();
    let other = Names::new(&"a".repeat(128), 8).unwrap();
    assert_ne!(first.lifetime, other.lifetime);
    assert!(first.startup.len() < 260 && first.class.len() < 256);
    let compatibility = Names::new("example.app", 7).unwrap();
    assert_eq!(compatibility.legacy_startup, "Local\\example.app-startup");
    assert_eq!(compatibility.legacy_class, "example.app-sic");
    assert_eq!(compatibility.legacy_title, "example.app-siw");
}

#[test]
fn unacknowledged_retirement_keeps_custody_and_fences_new_installation() {
    let slot = reserve_installation().unwrap();
    let wake = Arc::new(WakeState {
        pending: AtomicBool::new(false),
        callback: Arc::new(|| true),
    });
    let (finished_tx, finished) = mpsc::sync_channel(1);
    let thread = thread::spawn(move || {
        finished_tx
            .send(Err(failed(
                "injected retirement failure",
                std::io::ErrorKind::Other,
            )))
            .unwrap();
    });
    let mut owner = DesktopInstanceOwner {
        custody: Some(ListenerCustody {
            stop: Arc::new(AtomicBool::new(false)),
            finished,
            thread: Some(thread),
            _wake: Arc::clone(&wake),
            _image: Arc::new(
                BoundPath::open_immutable_file(&std::env::current_exe().unwrap()).unwrap(),
            ),
            cleanup_failed: false,
            _slot: slot,
        }),
    };
    assert!(owner.stop().is_err());
    assert!(owner.custody.as_ref().unwrap().thread.is_none()); // Joined actual thread.
    drop(owner);
    assert!(INSTALLATION_ACTIVE.load(Ordering::Acquire));
    assert!(reserve_installation().is_err());
    assert_eq!(Arc::strong_count(&wake), 2); // Exact callback state is still retained.
    // Test owns no HWND/native endpoint. Release only this injected failed custody.
    let retained = FAILED_INSTALLATION.lock().unwrap().take().unwrap();
    assert!(retained.cleanup_failed && retained.thread.is_none());
    drop(retained);
    assert!(!INSTALLATION_ACTIVE.load(Ordering::Acquire));
    drop(reserve_installation().unwrap());
}

#[test]
fn private_startup_ownership_serializes_threads_and_releases_on_owner_thread() {
    let id = id();
    let guard = DesktopLaunchGuard::acquire(&id).unwrap();
    let other = id.clone();
    assert!(
        thread::spawn(move || { DesktopLaunchGuard::acquire_with_timeout(&other, 10).is_err() })
            .join()
            .unwrap()
    );
    drop(guard);
    assert!(
        thread::spawn(move || { DesktopLaunchGuard::acquire_with_timeout(&id, 10).is_ok() })
            .join()
            .unwrap()
    );
}

#[test]
fn mutex_security_is_validated_on_reuse_and_handle_is_not_inherited() {
    let principal = security::Principal::current().unwrap();
    let name = Names::new(&id(), principal.logon).unwrap().lifetime;
    let first = security::MutexHandle::create(&name, &principal).unwrap();
    let second = security::MutexHandle::create(&name, &principal).unwrap();
    for mutex in [&first, &second] {
        let mut flags = 0;
        // SAFETY: query only the owned named mutex handle's inheritance bit.
        assert_ne!(unsafe { GetHandleInformation(mutex.raw(), &mut flags) }, 0);
        assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
    }
    let broad_name = Names::new(&id(), principal.logon).unwrap().lifetime;
    let broad_name_wide = wide(&broad_name);
    // SAFETY: disposable test name, default unprotected DACL deliberately differs.
    let broad = unsafe {
        windows_sys::Win32::System::Threading::CreateMutexW(
            std::ptr::null(),
            0,
            broad_name_wide.as_ptr(),
        )
    };
    assert!(!broad.is_null());
    use std::os::windows::io::FromRawHandle;
    // SAFETY: this test owns the one successfully created native mutex handle.
    let _broad = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(broad.cast()) };
    assert!(security::MutexHandle::create(&broad_name, &principal).is_err());
}

#[test]
fn wake_payloads_are_closed_and_failed_callback_admission_is_never_acknowledged() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let state = WakeState {
        pending: AtomicBool::new(false),
        callback: Arc::new(move || {
            counted.fetch_add(1, Ordering::Relaxed);
            false
        }),
    };
    for (wparam, lparam) in [(1, 0), (0, 1), (usize::MAX, isize::MIN)] {
        assert_eq!(state.request(wparam, lparam), 0);
    }
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    state.pending.store(true, Ordering::Release);
    assert_eq!(state.request(0, 0), 0);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    state.pending.store(false, Ordering::Release);
    assert_eq!(state.request(0, 0), 0);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert!(!state.pending.load(Ordering::Acquire));
    let panics = WakeState {
        pending: AtomicBool::new(false),
        callback: Arc::new(|| panic!("test callback")),
    };
    assert_eq!(panics.request(0, 0), 0);
    assert!(!panics.pending.load(Ordering::Acquire));
}
