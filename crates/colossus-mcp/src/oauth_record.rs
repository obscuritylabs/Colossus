//! Bounded, zeroizing serialization for complete OAuth records, not single tokens.

use colossus_contracts::MAX_VAULT_RECORD_BYTES;
use rmcp::transport::auth::{AuthError, StoredCredentials};
use std::io::{self, Write};
use zeroize::Zeroizing;

pub(super) const MAX_STATE_RECORD_BYTES: usize = 3 * MAX_VAULT_RECORD_BYTES;
const PLAINTEXT_PREFIX: &[u8] = b"{\"schema_version\":1,\"credentials\":";
pub(super) const MAX_PLAINTEXT_STATE_RECORD_BYTES: usize =
    MAX_VAULT_RECORD_BYTES + PLAINTEXT_PREFIX.len() + 1;

struct RecordBuffer(Zeroizing<Vec<u8>>);

impl Write for RecordBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_VAULT_RECORD_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("OAuth record exceeds its bound"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn encode(credentials: &StoredCredentials) -> Result<Zeroizing<Vec<u8>>, AuthError> {
    // Never reallocate a buffer after it has contained credential plaintext.
    let mut buffer = RecordBuffer(Zeroizing::new(Vec::with_capacity(MAX_VAULT_RECORD_BYTES)));
    serde_json::to_writer(&mut buffer, credentials).map_err(|_| invalid_record())?;
    Ok(buffer.0)
}

pub(super) fn decode(bytes: &[u8]) -> Result<StoredCredentials, AuthError> {
    if bytes.is_empty() || bytes.len() > MAX_VAULT_RECORD_BYTES {
        return Err(invalid_record());
    }
    serde_json::from_slice(bytes).map_err(|_| invalid_record())
}

pub(super) fn plaintext_state_record(bytes: &[u8]) -> Zeroizing<Vec<u8>> {
    // The credential JSON has already passed encode's exact payload bound.
    let mut record = Zeroizing::new(Vec::with_capacity(PLAINTEXT_PREFIX.len() + bytes.len() + 1));
    record.extend_from_slice(PLAINTEXT_PREFIX);
    record.extend_from_slice(bytes);
    record.push(b'}');
    record
}

pub(super) fn invalid_record() -> AuthError {
    AuthError::InternalError("OAuth credential record is invalid or exceeds 1 MiB".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialized_oauth_bound_is_exact_and_accounts_for_escaping() {
        let mut credentials = StoredCredentials::new(String::new(), None, Vec::new(), None);
        let overhead = encode(&credentials).unwrap().len();
        credentials.client_id = "x".repeat(MAX_VAULT_RECORD_BYTES - overhead);
        let exact = encode(&credentials).unwrap();
        assert_eq!(exact.len(), MAX_VAULT_RECORD_BYTES);
        assert!(decode(&exact).is_ok());
        credentials.client_id.push('x');
        assert!(encode(&credentials).is_err());
        credentials.client_id = "\"".repeat(MAX_VAULT_RECORD_BYTES / 2);
        assert!(encode(&credentials).is_err());
        assert!(decode(&vec![b' '; MAX_VAULT_RECORD_BYTES + 1]).is_err());
    }
}
