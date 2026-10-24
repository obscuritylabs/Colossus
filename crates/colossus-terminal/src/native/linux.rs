//! Best-effort ordinary job cleanup without signaling a reused numeric PID.
use colossus_api::ApiResult;
use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};
use std::collections::BTreeMap;

pub(super) fn require_pidfds() -> ApiResult<()> {
    let raw = i32::try_from(std::process::id()).map_err(|_| super::failure())?;
    let pid = Pid::from_raw(raw).ok_or_else(super::failure)?;
    let _pidfd = pidfd_open(pid, PidfdFlags::empty()).map_err(|_| super::failure())?;
    Ok(())
}

/// The direct child remains unreaped, pinning its SID throughout both scans.
/// New process sessions deliberately created by the human remain outside this boundary.
pub(super) fn close_session(session: i32) {
    let expected = nix::unistd::Pid::from_raw(session);
    let mut members = BTreeMap::new();
    for _ in 0..2 {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            break;
        };
        for entry in entries.take(65536).flatten() {
            if members.len() >= 4096 {
                break;
            }
            let Some(raw) = entry
                .file_name()
                .to_str()
                .and_then(|value| value.parse::<i32>().ok())
                .filter(|pid| *pid > 0)
            else {
                continue;
            };
            if members.contains_key(&raw) {
                continue;
            }
            let pid = nix::unistd::Pid::from_raw(raw);
            if nix::unistd::getsid(Some(pid)) != Ok(expected) {
                continue;
            }
            let Some(rustix_pid) = Pid::from_raw(raw) else {
                continue;
            };
            let Ok(bound) = pidfd_open(rustix_pid, PidfdFlags::empty()) else {
                continue;
            };
            // A process may have exited/reused its number before pidfd_open. Recheck
            // after binding; signals then use the retained object rather than that number.
            if nix::unistd::getsid(Some(pid)) != Ok(expected) {
                continue;
            }
            if pidfd_send_signal(&bound, Signal::STOP).is_ok() {
                members.insert(raw, bound);
            }
        }
    }
    for bound in members.values() {
        let _ = pidfd_send_signal(bound, Signal::KILL);
    }
}
