//! The concrete safe adapter calls the in-process nonexportable native owner.
use crate::{BrokerError, ClientIdentity, IdentityScheme, LocalHandshake, LocalIdentity};
use colossus_native_browser_mac_identity::{NativeHandshake, NativeIdentity, Scheme};
use std::{sync::Arc, time::Instant};
struct MacIdentity(Arc<NativeIdentity>);
struct MacHandshake(NativeHandshake);
impl ClientIdentity {
    /// Borrow an actually prepared native owner retained by this broker's local
    /// composition through startup errors and explicit cleanup. No key,
    /// filesystem path or generic signing operation is published to CEF or Core.
    pub fn from_mac_native(
        origin: String,
        generation: [u8; 32],
        reviewed_leaf: [u8; 32],
        owner: &Arc<NativeIdentity>,
    ) -> Result<Self, BrokerError> {
        Self::new(
            origin,
            generation,
            reviewed_leaf,
            Arc::new(MacIdentity(Arc::clone(owner))),
        )
    }
}
impl LocalIdentity for MacIdentity {
    fn public_leaf(&self) -> Result<(Vec<u8>, IdentityScheme), BrokerError> {
        let (leaf, scheme) = self.0.public_leaf();
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
        self.0
            .begin(generation, origin, leaf, deadline)
            .map(|handshake| Box::new(MacHandshake(handshake)) as Box<dyn LocalHandshake>)
            .map_err(|_| BrokerError::Unavailable)
    }
    fn revoke(&self) {
        self.0.revoke();
    }
    fn finish(&self) -> Result<(), BrokerError> {
        self.0.finish().map_err(|_| BrokerError::OutcomeUnknown)
    }
}
impl LocalHandshake for MacHandshake {
    fn certificate_verify(
        &mut self,
        scheme: IdentityScheme,
        transcript: &[u8; 32],
    ) -> Result<Vec<u8>, BrokerError> {
        self.0
            .certificate_verify(
                match scheme {
                    IdentityScheme::EcdsaP256Sha256 => Scheme::EcdsaP256Sha256,
                    IdentityScheme::RsaPssSha256 => Scheme::RsaPssSha256,
                },
                transcript,
            )
            .map_err(|_| BrokerError::Unavailable)
    }
}
