use super::*;
use chacha20poly1305::{
    KeyInit as _, XChaCha20Poly1305, XNonce,
    aead::{Aead as _, Payload},
};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;

pub(super) fn bearer(variable: &str) -> CloudResult<[u8; 32]> {
    let value = Zeroizing::new(std::env::var(variable).map_err(|_| CloudError::Storage)?);
    if value.len() < 16 || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(CloudError::Storage);
    }
    Ok(Sha256::digest(value.as_bytes()).into())
}

pub(super) fn key(variable: &str) -> CloudResult<Zeroizing<[u8; 32]>> {
    let value = Zeroizing::new(std::env::var(variable).map_err(|_| CloudError::Storage)?);
    let decoded = Zeroizing::new(hex::decode(value.as_str()).map_err(|_| CloudError::Storage)?);
    let key: [u8; 32] = decoded
        .as_slice()
        .try_into()
        .map_err(|_| CloudError::Storage)?;
    Ok(Zeroizing::new(key))
}

pub(super) fn authorize(state: &AuthorityState, headers: &HeaderMap) -> Result<(), StatusCode> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if token.len() > 4096 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let actual: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    if !bool::from(state.bearer_hash.ct_eq(&actual)) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

fn associated_data(namespace: &str, revision: u64) -> CloudResult<Vec<u8>> {
    serde_json::to_vec(&("colossus-independent-anchor-v1", namespace, revision))
        .map_err(|_| CloudError::Storage)
}

pub(super) fn encrypt(
    key: &[u8; 32],
    namespace: &str,
    snapshot: &SharedAnchorSnapshot,
) -> CloudResult<Vec<u8>> {
    let mut nonce = [0_u8; 24];
    getrandom::fill(&mut nonce).map_err(|_| CloudError::Storage)?;
    let plaintext = Zeroizing::new(serde_json::to_vec(snapshot).map_err(|_| CloudError::Storage)?);
    let aad = associated_data(namespace, snapshot.revision)?;
    let ciphertext = XChaCha20Poly1305::new(key.into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| CloudError::Storage)?;
    let mut envelope = Vec::with_capacity(25 + ciphertext.len());
    envelope.push(1);
    envelope.extend_from_slice(&nonce);
    envelope.extend_from_slice(&ciphertext);
    Ok(envelope)
}

pub(super) fn decrypt(
    key: &[u8; 32],
    namespace: &str,
    revision: u64,
    envelope: &[u8],
) -> CloudResult<SharedAnchorSnapshot> {
    if envelope.len() < 41 || envelope.len() > 16 * 1024 || envelope[0] != 1 {
        return Err(CloudError::Storage);
    }
    let aad = associated_data(namespace, revision)?;
    let plaintext = Zeroizing::new(
        XChaCha20Poly1305::new(key.into())
            .decrypt(
                XNonce::from_slice(&envelope[1..25]),
                Payload {
                    msg: &envelope[25..],
                    aad: &aad,
                },
            )
            .map_err(|_| CloudError::Storage)?,
    );
    let snapshot: SharedAnchorSnapshot =
        serde_json::from_slice(&plaintext).map_err(|_| CloudError::Storage)?;
    if snapshot.version != 1 || snapshot.revision != revision || snapshot.anchor.is_none() {
        return Err(CloudError::Storage);
    }
    validate_shared_anchor_advance(None, snapshot.anchor.as_ref().ok_or(CloudError::Storage)?)
        .map_err(|_| CloudError::Storage)?;
    Ok(snapshot)
}
