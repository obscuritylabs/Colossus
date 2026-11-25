use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use colossus_contracts::{
    ENCRYPTED_PAYLOAD_ALGORITHM, EncryptedPayload, EventEnvelope, PLAINTEXT_PAYLOAD_ALGORITHM,
};
use colossus_ports::{JournalPayloadProtection, KeyProvider, StoreError};
use serde_json::Value;

use crate::envelope::{PersistedEnvelope, sha256_hex};

pub(super) fn protect_payload(
    protection: JournalPayloadProtection,
    keys: &dyn KeyProvider,
    plaintext: &[u8],
    aad: &[u8],
) -> Result<EncryptedPayload, StoreError> {
    let plaintext_hash = sha256_hex(plaintext);
    if protection == JournalPayloadProtection::Plaintext {
        return Ok(EncryptedPayload {
            key_id: "none".into(),
            algorithm: PLAINTEXT_PAYLOAD_ALGORITHM.into(),
            nonce: String::new(),
            ciphertext: hex::encode(plaintext),
            plaintext_hash,
        });
    }
    let (key_id, key) = keys.active_key()?;
    let mut nonce = [0_u8; 24];
    getrandom::fill(&mut nonce)
        .map_err(|_| StoreError::Adapter("cannot generate journal nonce".into()))?;
    let ciphertext = XChaCha20Poly1305::new((&key).into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| StoreError::Adapter("cannot encrypt journal payload".into()))?;
    Ok(EncryptedPayload {
        key_id,
        algorithm: ENCRYPTED_PAYLOAD_ALGORITHM.into(),
        nonce: hex::encode(nonce),
        ciphertext: hex::encode(ciphertext),
        plaintext_hash,
    })
}

/// Authenticate a stored record, validate its plaintext hash and decode the payload.
///
/// Both arguments must come from the same stored envelope.
pub fn verify_payload(
    protection: JournalPayloadProtection,
    keys: &dyn KeyProvider,
    envelope: &EventEnvelope,
    persisted: &PersistedEnvelope<'_>,
) -> Result<Value, StoreError> {
    if persisted.compute_record_hash()? != envelope.record_hash
        || persisted.record_hash != envelope.record_hash
    {
        return Err(StoreError::Verification(
            "event record hash mismatch".into(),
        ));
    }
    let plaintext = unprotect_payload(protection, keys, &envelope.payload, persisted)?;
    if sha256_hex(&plaintext) != envelope.payload.plaintext_hash {
        return Err(StoreError::Verification(
            "event plaintext hash mismatch".into(),
        ));
    }
    serde_json::from_slice(&plaintext)
        .map_err(|_| StoreError::Verification("invalid stored journal payload JSON".into()))
}

fn unprotect_payload(
    protection: JournalPayloadProtection,
    keys: &dyn KeyProvider,
    payload: &EncryptedPayload,
    persisted: &PersistedEnvelope<'_>,
) -> Result<Vec<u8>, StoreError> {
    if protection == JournalPayloadProtection::Plaintext {
        if payload.algorithm != PLAINTEXT_PAYLOAD_ALGORITHM
            || payload.key_id != "none"
            || !payload.nonce.is_empty()
        {
            return Err(StoreError::Verification(
                "event payload does not match plaintext journal protection".into(),
            ));
        }
        return hex::decode(&payload.ciphertext)
            .map_err(|_| StoreError::Verification("invalid plaintext payload encoding".into()));
    }
    if payload.algorithm != ENCRYPTED_PAYLOAD_ALGORITHM {
        return Err(StoreError::Verification(
            "event payload does not match encrypted journal protection".into(),
        ));
    }
    let key = keys.key_by_id(&payload.key_id)?;
    let mut nonce = [0_u8; 24];
    hex::decode_to_slice(&payload.nonce, &mut nonce).map_err(|_| {
        StoreError::Verification("invalid XChaCha20 nonce encoding or length".into())
    })?;
    let ciphertext = hex::decode(&payload.ciphertext)
        .map_err(|_| StoreError::Verification("invalid encrypted payload encoding".into()))?;
    let aad = persisted.associated_data_bytes()?;
    XChaCha20Poly1305::new((&key).into())
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| StoreError::Verification("event payload authentication failed".into()))
}
