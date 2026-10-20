use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use colossus_ports::StoreError;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

pub(super) fn derive_key(
    variable: &str,
    purpose: &[u8],
    namespace: &str,
) -> Result<Zeroizing<[u8; 32]>, ChatError> {
    let encoded = Zeroizing::new(std::env::var(variable).map_err(|_| ChatError::StateUnavailable)?);
    let decoded = Zeroizing::new(
        hex::decode(encoded.as_str())
            .or_else(|_| STANDARD.decode(encoded.as_bytes()))
            .map_err(|_| ChatError::StateUnavailable)?,
    );
    if decoded.len() != 32 {
        return Err(ChatError::StateUnavailable);
    }
    derive(decoded.as_slice(), purpose, namespace)
}

pub(super) fn derive(
    master: &[u8],
    purpose: &[u8],
    namespace: &str,
) -> Result<Zeroizing<[u8; 32]>, ChatError> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(master).map_err(|_| ChatError::StateUnavailable)?;
    mac.update(b"colossus-shared-chat-key-v1\0");
    mac.update(purpose);
    mac.update(b"\0");
    mac.update(namespace.as_bytes());
    Ok(Zeroizing::new(mac.finalize().into_bytes().into()))
}

pub(super) struct SharedKeyProvider {
    id: String,
    key: Zeroizing<[u8; 32]>,
    namespace: String,
    authority: http::AnchorHttp,
}

impl SharedKeyProvider {
    pub(super) fn new(
        id: String,
        key: Zeroizing<[u8; 32]>,
        namespace: String,
        authority: http::AnchorHttp,
    ) -> Self {
        Self {
            id,
            key,
            namespace,
            authority,
        }
    }
}

impl KeyProvider for SharedKeyProvider {
    fn active_key(&self) -> Result<(String, [u8; 32]), StoreError> {
        Ok((self.id.clone(), *self.key))
    }

    fn key_by_id(&self, key_id: &str) -> Result<[u8; 32], StoreError> {
        if key_id != self.id {
            return Err(StoreError::KeyUnavailable(
                "shared journal key is unavailable".into(),
            ));
        }
        Ok(*self.key)
    }

    fn load_anchor(&self) -> Result<Option<SecureAnchor>, StoreError> {
        self.authority
            .read(&self.namespace)
            .map(|snapshot| snapshot.anchor)
            .map_err(|_| unavailable())
    }

    fn store_anchor(&self, anchor: &SecureAnchor) -> Result<(), StoreError> {
        for _ in 0..3 {
            let current = self
                .authority
                .read(&self.namespace)
                .map_err(|_| unavailable())?;
            validate_shared_anchor_advance(current.anchor.as_ref(), anchor).map_err(|_| {
                StoreError::Verification("shared anchor would regress or diverge".into())
            })?;
            if current.anchor.as_ref() == Some(anchor) {
                return Ok(());
            }
            let update = SharedAnchorUpdate {
                version: 1,
                expected_revision: current.revision,
                anchor: anchor.clone(),
            };
            match self.authority.write(&self.namespace, update) {
                Ok(_) => return Ok(()),
                Err(http::AuthorityError::Conflict) => continue,
                Err(http::AuthorityError::Unavailable) => return Err(unavailable()),
            }
        }
        Err(unavailable())
    }
}

fn unavailable() -> StoreError {
    StoreError::KeyUnavailable("independent anchor authority is unavailable".into())
}
