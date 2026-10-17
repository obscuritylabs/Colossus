use super::*;

unsafe extern "C" {
    fn mach_port_get_refs(
        task: libc::mach_port_t,
        name: libc::mach_port_t,
        right: u32,
        references: *mut u32,
    ) -> libc::kern_return_t;
}

fn send_references(name: libc::mach_port_t) -> u32 {
    let mut references = 0;
    assert_eq!(
        // SAFETY: the test's retained send right owns this genuine Mach name. The
        // SDK's SEND-right selector is zero and references is valid writable u32.
        unsafe { mach_port_get_refs(mach_task_self_, name, 0, &mut references) },
        0
    );
    references
}

#[test]
fn current_session_retains_and_releases_one_real_kernel_send_reference() {
    let first = RetainedDarwinAuditSession::current().expect("current assigned audit session");
    let identity = DarwinProcessIdentity::bind(std::process::id()).unwrap();
    assert_eq!(first.owner_identity(), &identity);
    assert_eq!(first.audit_session_id(), identity.audit_session_id());
    let before = send_references(first._right.0);
    assert!(before > 0);
    let second = RetainedDarwinAuditSession::current().unwrap();
    assert_eq!(second._right.0, first._right.0);
    assert_eq!(send_references(first._right.0), before + 1);
    drop(second);
    assert_eq!(send_references(first._right.0), before);
}
