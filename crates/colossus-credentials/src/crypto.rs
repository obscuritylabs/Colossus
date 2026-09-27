//! Versioned AEAD records and strictly bounded platform-key envelopes.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{KeyInit as _, Tag, XChaCha20Poly1305, XNonce, aead::AeadInPlace as _};
use colossus_contracts::{CredentialError, MAX_VAULT_RECORD_BYTES};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use zeroize::Zeroizing;

use crate::platform::MAX_KEY_ENVELOPE_BYTES;

const PREFIX: &[u8; 4] = b"CCV1";
const NONCE_BYTES: usize = 24;
pub(crate) const MAX_CIPHERTEXT_BYTES: usize = MAX_VAULT_RECORD_BYTES + 4 + NONCE_BYTES + 16;
pub(crate) type MasterKey = Zeroizing<[u8; 32]>;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyEnvelope<'a> {
    version: u16,
    #[serde(borrow)]
    vault_id: &'a str,
    #[serde(borrow)]
    key_id: &'a str,
    #[serde(borrow)]
    key: &'a str,
}

pub(crate) fn random_id() -> Result<String, CredentialError> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| CredentialError::Unavailable)?;
    Ok(hex::encode(bytes))
}

pub(crate) fn new_key() -> Result<MasterKey, CredentialError> {
    let mut key = Zeroizing::new([0; 32]);
    getrandom::fill(key.as_mut()).map_err(|_| CredentialError::Unavailable)?;
    Ok(key)
}

pub(crate) fn encode_key(
    vault_id: &str,
    key_id: &str,
    key: &MasterKey,
) -> Result<Zeroizing<Vec<u8>>, CredentialError> {
    let encoded_key = Zeroizing::new(URL_SAFE_NO_PAD.encode(key.as_ref()));
    let envelope = KeyEnvelope {
        version: 1,
        vault_id,
        key_id,
        key: &encoded_key,
    };
    let mut encoded = Zeroizing::new(Vec::with_capacity(MAX_KEY_ENVELOPE_BYTES));
    serde_json::to_writer(BoundedEnvelope(&mut encoded), &envelope)
        .map_err(|_| CredentialError::Oversized)?;
    Ok(encoded)
}

// Never reallocate a plaintext master-key envelope, including rejected serialization.
struct BoundedEnvelope<'a>(&'a mut Zeroizing<Vec<u8>>);

impl Write for BoundedEnvelope<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_KEY_ENVELOPE_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn decode_key(
    bytes: &[u8],
    vault_id: &str,
    key_id: &str,
) -> Result<MasterKey, CredentialError> {
    if bytes.len() > MAX_KEY_ENVELOPE_BYTES || bytes.contains(&b'\\') {
        return Err(CredentialError::Corrupt);
    }
    // Generated IDs/base64url never need JSON escapes. Rejecting escapes lets the
    // decoder borrow every field from the zeroizing input instead of copying a key
    // into serde_json's unprotected string scratch buffer, even on malformed input.
    let envelope: KeyEnvelope<'_> =
        serde_json::from_slice(bytes).map_err(|_| CredentialError::Corrupt)?;
    if envelope.version != 1
        || envelope.vault_id != vault_id
        || envelope.key_id != key_id
        || envelope.key.len() != 43
    {
        return Err(CredentialError::Corrupt);
    }
    let mut key = Zeroizing::new([0; 32]);
    let length = URL_SAFE_NO_PAD
        .decode_slice(envelope.key.as_bytes(), key.as_mut())
        .map_err(|_| CredentialError::Corrupt)?;
    if length != 32 {
        return Err(CredentialError::Corrupt);
    }
    Ok(key)
}

pub(crate) fn encrypt(
    key: &MasterKey,
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, CredentialError> {
    if plaintext.len() > MAX_VAULT_RECORD_BYTES {
        return Err(CredentialError::Oversized);
    }
    let mut nonce = [0; NONCE_BYTES];
    getrandom::fill(&mut nonce).map_err(|_| CredentialError::Unavailable)?;
    let mut encoded = Zeroizing::new(Vec::with_capacity(4 + NONCE_BYTES + plaintext.len() + 16));
    encoded.extend_from_slice(PREFIX);
    encoded.extend_from_slice(&nonce);
    encoded.extend_from_slice(plaintext);
    let tag = XChaCha20Poly1305::new(key.as_ref().into())
        .encrypt_in_place_detached(
            XNonce::from_slice(&nonce),
            aad,
            &mut encoded[4 + NONCE_BYTES..],
        )
        .map_err(|_| CredentialError::Corrupt)?;
    encoded.extend_from_slice(&tag);
    // Only authenticated ciphertext leaves the zeroizing allocation owner.
    Ok(std::mem::take(&mut *encoded))
}

pub(crate) fn decrypt(
    key: &MasterKey,
    aad: &[u8],
    encoded: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CredentialError> {
    if encoded.len() < 4 + NONCE_BYTES + 16
        || encoded.len() > MAX_CIPHERTEXT_BYTES
        || !encoded.starts_with(PREFIX)
    {
        return Err(CredentialError::Corrupt);
    }
    let tag_start = encoded.len() - 16;
    let mut plaintext = Zeroizing::new(encoded[4 + NONCE_BYTES..tag_start].to_vec());
    XChaCha20Poly1305::new(key.as_ref().into())
        .decrypt_in_place_detached(
            XNonce::from_slice(&encoded[4..4 + NONCE_BYTES]),
            aad,
            &mut plaintext,
            Tag::from_slice(&encoded[tag_start..]),
        )
        .map_err(|_| CredentialError::Corrupt)?;
    Ok(plaintext)
}
