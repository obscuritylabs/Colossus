//! Fail-closed macOS composition gate for a separately owned Chromium host.
//!
//! The human CEF preview and direct-child Darwin process ownership do not establish
//! the whole-process network and cleanup boundary required by `BrowserHostFactory`.
//! This type gives trusted composition one explicit Mac owner to retain while making
//! the missing native acceptance visible. It cannot be constructed in an available
//! state and never allocates a process, profile, channel, or network lease.

use async_trait::async_trait;
use colossus_browser_bridge::BrowserHostFactory;
use colossus_contracts::BrowserCapabilities;
use colossus_ports::{
    BrowserDriver, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
};
use std::sync::Arc;

#[cfg(target_os = "macos")]
mod process_cleanup {
    use colossus_darwin_process::{
        DarwinAuditSessionEnd, DarwinProcessIdentity, DarwinProcessSignal, DarwinProcessSnapshot,
        EmptyProcessDomainEvidence, ReceivedDarwinAuditObservation,
    };
    use std::{io, time::Duration};

    /// Bounded termination of one genuine audit-session observation.
    ///
    /// The observer is enrolled in a different audit session and retains the
    /// producer's session reference across the producer's death. This owner can
    /// terminate exact observed process incarnations and wait for a kernel
    /// zero-process event. It does not fence admission or prevent a process from
    /// joining another audit session. Callers must retain it on every error.
    pub struct MacosBrowserProcessKeeper {
        observation: ReceivedDarwinAuditObservation,
        uid: u32,
        maximum_processes: usize,
    }

    /// Kernel-backed end plus a subsequent empty bounded UID/ASID census.
    ///
    /// This is process cleanup evidence only. It cannot attest network revocation,
    /// native CEF shutdown, immutable membership, or the absence of an escaped
    /// process that changed audit session or credentials.
    pub struct MacosBrowserProcessClearance {
        end: DarwinAuditSessionEnd,
        empty: EmptyProcessDomainEvidence,
    }

    impl MacosBrowserProcessClearance {
        /// Matching kernel zero-process event for the retained session.
        pub fn session_end(&self) -> &DarwinAuditSessionEnd {
            &self.end
        }

        /// Empty kernel UID/ASID census taken after the end event.
        pub fn empty_census(&self) -> &EmptyProcessDomainEvidence {
            &self.empty
        }
    }

    impl MacosBrowserProcessKeeper {
        /// Adopt an authenticated outside-ASID observation for a fixed credential domain.
        ///
        /// A distinct OS identity and admission fence are required before this
        /// can serve as complete browser containment. Configuration is checked
        /// during termination so a mistaken UID cannot drop this retained owner.
        pub fn new(
            observation: ReceivedDarwinAuditObservation,
            uid: u32,
            maximum_processes: usize,
        ) -> Self {
            Self {
                observation,
                uid,
                maximum_processes,
            }
        }

        /// Terminate observed members, force remaining members, and verify kernel end.
        ///
        /// Revoke network admission before invoking this method. Both deadlines
        /// must be positive and at most thirty seconds. A timeout, census error,
        /// event-stream failure, or signal error leaves the same keeper in place
        /// for independent reconciliation; none is interpreted as success.
        pub fn terminate_processes(
            &mut self,
            orderly: Duration,
            forced: Duration,
        ) -> io::Result<MacosBrowserProcessClearance> {
            let owner = self.observation.owner_identity();
            if self.uid == u32::MAX
                || self.uid != owner.real_uid()
                || self.uid != owner.effective_uid()
                || !(1..=4096).contains(&self.maximum_processes)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "browser cleanup needs the source's fixed UID and a finite census bound",
                ));
            }
            let end = terminate(&mut NativeDomain { keeper: self }, orderly, forced)?;
            let empty = self
                .snapshot()?
                .empty_evidence()
                .ok_or_else(|| unknown("browser session gained a member after kernel END"))?;
            Ok(MacosBrowserProcessClearance { end, empty })
        }

        fn snapshot(&self) -> io::Result<DarwinProcessSnapshot> {
            DarwinProcessSnapshot::for_uid(self.uid, self.maximum_processes)?
                .for_audit_session(self.observation.audit_session_id())
        }
    }

    trait ProcessDomain {
        type Member: Clone;
        type End;

        fn members(&mut self) -> io::Result<Vec<Self::Member>>;
        fn signal(&mut self, member: &Self::Member, signal: DarwinProcessSignal) -> io::Result<()>;
        fn wait_for_end(&mut self, timeout: Duration) -> io::Result<Option<Self::End>>;
    }

    struct NativeDomain<'a> {
        keeper: &'a mut MacosBrowserProcessKeeper,
    }

    impl ProcessDomain for NativeDomain<'_> {
        type Member = DarwinProcessIdentity;
        type End = DarwinAuditSessionEnd;

        fn members(&mut self) -> io::Result<Vec<Self::Member>> {
            Ok(self.keeper.snapshot()?.identities().to_vec())
        }

        fn signal(&mut self, member: &Self::Member, signal: DarwinProcessSignal) -> io::Result<()> {
            match member.signal(signal) {
                Ok(()) => Ok(()),
                // The observed incarnation may have exited or execed after the
                // census. Re-enumeration and kernel END still determine success.
                Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(()),
                Err(error) => Err(error),
            }
        }

        fn wait_for_end(&mut self, timeout: Duration) -> io::Result<Option<Self::End>> {
            match self.keeper.observation.wait_for_end(timeout) {
                Ok(end) => Ok(Some(end)),
                Err(error) if error.kind() == io::ErrorKind::TimedOut => Ok(None),
                Err(error) => Err(error),
            }
        }
    }

    fn terminate<D: ProcessDomain>(
        domain: &mut D,
        orderly: Duration,
        forced: Duration,
    ) -> io::Result<D::End> {
        for duration in [orderly, forced] {
            if duration.is_zero() || duration > Duration::from_secs(30) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "browser cleanup deadlines must be positive and at most thirty seconds",
                ));
            }
        }
        for member in domain.members()? {
            domain.signal(&member, DarwinProcessSignal::Terminate)?;
        }
        if let Some(end) = domain.wait_for_end(orderly)? {
            return Ok(end);
        }
        // Re-enumerate after an exec, fork, or orderly timeout. A stale token
        // cannot grant authority to signal a recycled PID.
        for member in domain.members()? {
            domain.signal(&member, DarwinProcessSignal::Kill)?;
        }
        domain
            .wait_for_end(forced)?
            .ok_or_else(|| unknown("browser session did not reach kernel END"))
    }

    fn unknown(message: &'static str) -> io::Error {
        io::Error::new(io::ErrorKind::TimedOut, message)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::collections::VecDeque;

        struct ScriptedDomain {
            snapshots: VecDeque<Vec<u32>>,
            ends: VecDeque<bool>,
            signaled: Vec<(u32, DarwinProcessSignal)>,
        }

        impl ProcessDomain for ScriptedDomain {
            type Member = u32;
            type End = ();

            fn members(&mut self) -> io::Result<Vec<Self::Member>> {
                self.snapshots
                    .pop_front()
                    .ok_or_else(|| io::Error::other("unexpected census"))
            }

            fn signal(
                &mut self,
                member: &Self::Member,
                signal: DarwinProcessSignal,
            ) -> io::Result<()> {
                self.signaled.push((*member, signal));
                Ok(())
            }

            fn wait_for_end(&mut self, _timeout: Duration) -> io::Result<Option<Self::End>> {
                self.ends
                    .pop_front()
                    .map(|ended| ended.then_some(()))
                    .ok_or_else(|| io::Error::other("unexpected kernel wait"))
            }
        }

        #[test]
        fn cancellation_reenumerates_forked_or_execed_members_before_force() {
            let mut domain = ScriptedDomain {
                snapshots: VecDeque::from([vec![11], vec![12, 13]]),
                ends: VecDeque::from([false, true]),
                signaled: Vec::new(),
            };
            terminate(
                &mut domain,
                Duration::from_millis(1),
                Duration::from_millis(1),
            )
            .unwrap();
            assert_eq!(
                domain.signaled,
                [
                    (11, DarwinProcessSignal::Terminate),
                    (12, DarwinProcessSignal::Kill),
                    (13, DarwinProcessSignal::Kill),
                ]
            );
        }

        #[test]
        fn direct_child_exit_does_not_replace_kernel_end() {
            let mut domain = ScriptedDomain {
                snapshots: VecDeque::from([vec![], vec![]]),
                ends: VecDeque::from([false, false]),
                signaled: Vec::new(),
            };
            let error = terminate(
                &mut domain,
                Duration::from_millis(1),
                Duration::from_millis(1),
            )
            .expect_err("an empty observation without kernel END is unknown");
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        }

        #[test]
        fn invalid_deadlines_never_signal_a_process() {
            let mut domain = ScriptedDomain {
                snapshots: VecDeque::from([vec![11]]),
                ends: VecDeque::new(),
                signaled: Vec::new(),
            };
            assert!(terminate(&mut domain, Duration::ZERO, Duration::from_secs(1)).is_err());
            assert!(domain.signaled.is_empty());
        }
    }
}

#[cfg(target_os = "macos")]
pub use process_cleanup::{MacosBrowserProcessClearance, MacosBrowserProcessKeeper};

const CURRENT_BLOCKERS: [MacosBrowserRequirement; 6] = [
    MacosBrowserRequirement::WholeProcessTreeOwner,
    MacosBrowserRequirement::InheritedNetworkContainment,
    MacosBrowserRequirement::RequestOriginEnforcement,
    MacosBrowserRequirement::OwnedCertificateCustody,
    MacosBrowserRequirement::NativePresentationAcceptance,
    MacosBrowserRequirement::SignedDistributionAcceptance,
];

/// Independently accepted platform requirements that remain before Mac launch.
///
/// A direct-child PID, process group, BSD session, CEF callback, proxy, human
/// preview, or Debug signature cannot satisfy any of these requirements alone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MacosBrowserRequirement {
    /// Kernel-backed ownership and acknowledged exit of the browser and every helper.
    WholeProcessTreeOwner,
    /// Positively enforced direct TCP, UDP, QUIC, WebRTC, loopback and DNS denial.
    InheritedNetworkContainment,
    /// Exact HTTPS request-origin enforcement, including coalesced connections.
    RequestOriginEnforcement,
    /// Isolated private CA and reviewed client-identity import, use and removal.
    OwnedCertificateCustody,
    /// Authenticated compositor, input, IME, handoff and viewer lifecycle acceptance.
    NativePresentationAcceptance,
    /// Publisher-reviewed component inventory, signatures and installed-host evidence.
    SignedDistributionAcceptance,
}

/// Current native readiness of the Mac browser host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MacosBrowserReadiness {
    blockers: &'static [MacosBrowserRequirement],
}

impl MacosBrowserReadiness {
    /// Requirements for which no independently accepted receipt exists.
    #[must_use]
    pub fn blockers(self) -> &'static [MacosBrowserRequirement] {
        self.blockers
    }

    /// Whether native composition may advertise or allocate the browser host.
    #[must_use]
    pub fn is_available(self) -> bool {
        self.blockers.is_empty()
    }
}

/// Retained Mac browser owner that deliberately cannot launch before acceptance.
///
/// There is intentionally no constructor accepting booleans, paths, process IDs,
/// audit-session IDs, or self-reported child evidence. A future accepted native
/// owner must introduce an unforgeable receipt and replace this gate as one reviewed
/// change. Until then Core can retain this value without accidentally enabling Mac
/// automation.
#[derive(Clone, Copy, Debug)]
pub struct MacosBrowserSupervisor;

impl MacosBrowserSupervisor {
    /// Construct the current fail-closed native owner.
    #[must_use]
    pub fn unavailable() -> Self {
        Self
    }

    /// Return the exact independently unaccepted requirements.
    #[must_use]
    pub fn readiness(self) -> MacosBrowserReadiness {
        MacosBrowserReadiness {
            blockers: &CURRENT_BLOCKERS,
        }
    }
}

#[async_trait]
impl BrowserHostFactory for MacosBrowserSupervisor {
    fn capabilities(&self) -> BrowserCapabilities {
        BrowserCapabilities::unavailable()
    }

    async fn launch(
        &self,
        _request: &BrowserDriverOpenRequest,
        _control: &BrowserDriverControl,
    ) -> Result<Arc<dyn BrowserDriver>, BrowserDriverError> {
        Err(BrowserDriverError::Unavailable)
    }

    async fn reap_failed_launch(
        &self,
        _request: &BrowserDriverOpenRequest,
    ) -> Result<(), BrowserDriverError> {
        // `launch` performs no allocation before returning unavailable. There is no
        // uncertain native obligation to reconcile through this gate.
        Ok(())
    }
}
