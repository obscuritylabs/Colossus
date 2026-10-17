//! Exact native peer enrollment, without path or PID-only authentication.

use std::{fmt, io};

use core_foundation::{base::TCFType as _, data::CFData};
use security_framework::os::macos::code_signing::{
    Flags, GuestAttributes, SecCode, SecRequirement,
};

use crate::DarwinProcessIdentity;

/// One genuine enrolled process token and a trusted manifest's exact cdhash.
///
/// The caller supplies the hash from its independently verified native manifest,
/// never from a peer's report or a pathname lookup. Enrollment and subsequent
/// checks use all eight genuine audit-token words as Security.framework's guest
/// selector. PID reuse, exec, credential or session changes fail closed. This
/// type owns no Mach right and grants no execution or session-joining authority.
#[derive(Clone)]
pub struct DarwinAuditPeer {
    identity: DarwinProcessIdentity,
    cdhash: [u8; 20],
}

impl DarwinAuditPeer {
    /// Enroll the exact live process against an independently trusted cdhash.
    pub fn enroll(identity: &DarwinProcessIdentity, expected_cdhash: [u8; 20]) -> io::Result<Self> {
        if expected_cdhash == [0; 20] || matches!(identity.audit_session_id(), 0 | u32::MAX) {
            return Err(invalid(
                "audit peer lacks an assigned identity or trusted code hash",
            ));
        }
        let peer = Self {
            identity: identity.clone(),
            cdhash: expected_cdhash,
        };
        peer.verify()?;
        Ok(peer)
    }

    /// Genuine kernel process identity captured by this enrollment.
    pub fn identity(&self) -> &DarwinProcessIdentity {
        &self.identity
    }

    /// Revalidate the exact live incarnation, exec generation and signed code.
    pub fn verify(&self) -> io::Result<()> {
        if DarwinProcessIdentity::bind(self.identity.pid())? != self.identity {
            return Err(invalid("enrolled Darwin audit peer changed identity"));
        }
        let mut bytes = [0_u8; 32];
        for (word, output) in self
            .identity
            .audit_token_words()
            .iter()
            .zip(bytes.chunks_exact_mut(4))
        {
            output.copy_from_slice(&word.to_ne_bytes());
        }
        let token = CFData::from_buffer(&bytes);
        let mut attributes = GuestAttributes::new();
        attributes.set_audit_token(token.as_concrete_TypeRef());
        let code = SecCode::copy_guest_with_attribues(None, &attributes, Flags::NONE)
            .map_err(|_| invalid("enrolled Darwin audit peer has no matching signed guest"))?;
        let mut requirement = String::from("cdhash H\"");
        for byte in self.cdhash {
            use std::fmt::Write as _;
            write!(&mut requirement, "{byte:02x}")
                .map_err(|_| io::Error::other("audit peer requirement encoding failed"))?;
        }
        requirement.push('"');
        let requirement: SecRequirement = requirement
            .parse()
            .map_err(|_| invalid("trusted Darwin audit peer requirement is invalid"))?;
        code.check_validity(
            Flags::STRICT_VALIDATE | Flags::NO_NETWORK_ACCESS,
            &requirement,
        )
        .map_err(|_| invalid("enrolled Darwin audit peer failed signed code validation"))?;
        if DarwinProcessIdentity::bind(self.identity.pid())? != self.identity {
            return Err(invalid(
                "enrolled Darwin audit peer changed during validation",
            ));
        }
        Ok(())
    }

    pub(crate) fn matches(&self, token: &[u32; 8]) -> bool {
        self.identity.matches_audit_token(token)
    }

    pub(crate) fn authenticate(&self, token: &[u32; 8]) -> io::Result<()> {
        if !self.matches(token) {
            return Err(invalid(
                "Mach audit trailer does not match the enrolled peer",
            ));
        }
        self.verify()
    }
}

impl fmt::Debug for DarwinAuditPeer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DarwinAuditPeer")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
