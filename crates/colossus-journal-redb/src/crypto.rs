use super::*;

/// Checkpoint signer placeholder for journal modes where checkpoints are disabled.
#[derive(Default)]
pub struct DisabledCheckpointSigner;

impl CheckpointSigner for DisabledCheckpointSigner {
    fn key_id(&self) -> &str {
        "none"
    }

    fn sign(&self, _message: &[u8]) -> Result<Vec<u8>, StoreError> {
        Err(StoreError::Adapter(
            "checkpoint signing is disabled for plaintext storage".into(),
        ))
    }

    fn verify(&self, _message: &[u8], _signature: &[u8]) -> Result<(), StoreError> {
        Err(StoreError::Verification(
            "plaintext storage cannot contain signed checkpoints".into(),
        ))
    }
}

/// Ed25519 checkpoint signer created from explicit secret key bytes.
pub struct Ed25519CheckpointSigner {
    key_id: String,
    signing_key: SigningKey,
}

impl Ed25519CheckpointSigner {
    /// Construct a signer from a 32-byte Ed25519 secret key.
    pub fn new(key_id: impl Into<String>, secret: [u8; 32]) -> Self {
        Self {
            key_id: key_id.into(),
            signing_key: SigningKey::from_bytes(&secret),
        }
    }

    /// Public verification key bytes for external anchor verification.
    pub fn verifying_key(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }
}

impl CheckpointSigner for Ed25519CheckpointSigner {
    fn key_id(&self) -> &str {
        &self.key_id
    }

    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, StoreError> {
        Ok(self.signing_key.sign(message).to_bytes().to_vec())
    }

    fn verify(&self, message: &[u8], signature: &[u8]) -> Result<(), StoreError> {
        let signature = Signature::from_slice(signature)
            .map_err(|error| StoreError::Verification(error.to_string()))?;
        let key: VerifyingKey = self.signing_key.verifying_key();
        key.verify(message, &signature)
            .map_err(|error| StoreError::Verification(error.to_string()))
    }
}

#[cfg(test)]
pub(super) fn crash_at_test_fault(point: &str) {
    if std::env::var("COLOSSUS_REDB_TEST_CRASH_POINT").as_deref() == Ok(point) {
        std::process::abort();
    }
}
