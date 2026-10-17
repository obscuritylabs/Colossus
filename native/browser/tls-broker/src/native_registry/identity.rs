//! Native signer retention and read-only default/search metadata checks.
use super::files::HeldFile;
use crate::{BrokerError, IdentityScheme, LocalHandshake, LocalIdentity};
use colossus_native_browser_mac_identity::{NativeHandshake, NativeIdentity, Scheme};
use std::{sync::Arc, time::Instant};

pub(super) struct PreparedIdentity {
    pub owner: NativeIdentity,
    pub inputs: Vec<HeldFile>,
    pub metadata: [u8; 32],
}
impl PreparedIdentity {
    pub fn admitted(&self) -> Result<(), BrokerError> {
        for input in &self.inputs {
            input.revalidate()?;
        }
        metadata_unchanged(&self.metadata)
    }
    pub fn finish(&self) -> Result<(), BrokerError> {
        self.owner
            .finish()
            .map_err(|_| BrokerError::OutcomeUnknown)?;
        metadata_unchanged(&self.metadata)
    }
}
pub(super) fn metadata_unchanged(expected: &[u8; 32]) -> Result<(), BrokerError> {
    let current = colossus_native_browser_mac_identity::keychain_metadata_fingerprint()
        .map_err(|_| BrokerError::OutcomeUnknown)?;
    if current == *expected {
        Ok(())
    } else {
        Err(BrokerError::OutcomeUnknown)
    }
}
pub(super) struct GuardedIdentity(pub Arc<PreparedIdentity>);
struct GuardedHandshake {
    native: NativeHandshake,
    retained: Arc<PreparedIdentity>,
}
impl LocalIdentity for GuardedIdentity {
    fn public_leaf(&self) -> Result<(Vec<u8>, IdentityScheme), BrokerError> {
        self.0.admitted()?;
        let (leaf, scheme) = self.0.owner.public_leaf();
        Ok((
            leaf.to_vec(),
            match scheme {
                Scheme::EcdsaP256Sha256 => IdentityScheme::EcdsaP256Sha256,
                Scheme::RsaPssSha256 => IdentityScheme::RsaPssSha256,
            },
        ))
    }
    fn begin(
        &self,
        generation: &[u8; 32],
        origin: &str,
        leaf: &[u8; 32],
        deadline: Instant,
    ) -> Result<Box<dyn LocalHandshake>, BrokerError> {
        self.0.admitted()?;
        let native = self
            .0
            .owner
            .begin(generation, origin, leaf, deadline)
            .map_err(|_| BrokerError::Unavailable)?;
        self.0.admitted()?;
        Ok(Box::new(GuardedHandshake {
            native,
            retained: Arc::clone(&self.0),
        }))
    }
    fn revoke(&self) {
        self.0.owner.revoke();
    }
    fn finish(&self) -> Result<(), BrokerError> {
        self.0.finish()
    }
}
impl LocalHandshake for GuardedHandshake {
    fn certificate_verify(
        &mut self,
        scheme: IdentityScheme,
        transcript: &[u8; 32],
    ) -> Result<Vec<u8>, BrokerError> {
        self.retained.admitted()?;
        let output = self
            .native
            .certificate_verify(
                match scheme {
                    IdentityScheme::EcdsaP256Sha256 => Scheme::EcdsaP256Sha256,
                    IdentityScheme::RsaPssSha256 => Scheme::RsaPssSha256,
                },
                transcript,
            )
            .map_err(|_| BrokerError::Unavailable)?;
        // The native boundary independently rechecks actual parent/package both
        // before signing and before output. These checks add exact input/default
        // metadata generation validation; they never replace native admission.
        self.retained.admitted()?;
        Ok(output)
    }
}
