//! Closed authenticated Core selection; only public opaque IDs/fingerprint.
//! No container, password, private key, path, descriptor or signer can be encoded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PkiSelection {
    /// One-way correlation of Core's complete trusted runtime/application scope.
    pub scope_binding: [u8; 32],
    /// Independently registered native public-CA references.
    pub ca_ids: Vec<[u8; 32]>,
    /// Reviewed native client reference and exact public leaf fingerprint.
    pub client: Option<ReviewedClientIdentity>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewedClientIdentity {
    pub identity_id: [u8; 32],
    pub leaf_sha256: [u8; 32],
}
// The wire codec belongs to the Unix control transport; its pure roundtrip
// tests remain portable and execute on Windows too.
#[cfg(any(unix, test))]
impl PkiSelection {
    pub(crate) fn encode(&self) -> Result<Vec<u8>, ()> {
        if self.scope_binding == [0; 32]
            || self.ca_ids.len() > 8
            || self.ca_ids.is_empty() && self.client.is_none()
            || self
                .ca_ids
                .iter()
                .enumerate()
                .any(|(index, id)| *id == [0; 32] || self.ca_ids[..index].contains(id))
            || self.client.as_ref().is_some_and(|client| {
                client.identity_id == [0; 32] || client.leaf_sha256 == [0; 32]
            })
        {
            return Err(());
        }
        let mut bytes = Vec::with_capacity(34 + self.ca_ids.len() * 32 + 64);
        bytes.extend_from_slice(&self.scope_binding);
        bytes.push(self.ca_ids.len() as u8);
        for id in &self.ca_ids {
            bytes.extend_from_slice(id);
        }
        bytes.push(u8::from(self.client.is_some()));
        if let Some(client) = &self.client {
            bytes.extend_from_slice(&client.identity_id);
            bytes.extend_from_slice(&client.leaf_sha256);
        }
        Ok(bytes)
    }
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, ()> {
        if !(34..=354).contains(&bytes.len()) {
            return Err(());
        }
        let scope_binding = bytes[..32].try_into().map_err(|_| ())?;
        let count = usize::from(bytes[32]);
        if count > 8 || bytes.len() < 34 + count * 32 {
            return Err(());
        }
        let ca_ids = bytes[33..33 + count * 32]
            .chunks_exact(32)
            .map(|id| id.try_into().map_err(|_| ()))
            .collect::<Result<Vec<_>, _>>()?;
        let offset = 33 + count * 32;
        let client = match bytes[offset] {
            0 if bytes.len() == offset + 1 => None,
            1 if bytes.len() == offset + 65 => Some(ReviewedClientIdentity {
                identity_id: bytes[offset + 1..offset + 33].try_into().map_err(|_| ())?,
                leaf_sha256: bytes[offset + 33..].try_into().map_err(|_| ())?,
            }),
            _ => return Err(()),
        };
        let selection = Self {
            scope_binding,
            ca_ids,
            client,
        };
        selection.encode()?;
        Ok(selection)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn selection() -> PkiSelection {
        PkiSelection {
            scope_binding: [1; 32],
            ca_ids: vec![[2; 32]],
            client: Some(ReviewedClientIdentity {
                identity_id: [3; 32],
                leaf_sha256: [4; 32],
            }),
        }
    }
    #[test]
    fn bounded_public_selection_roundtrips_without_secret_or_nomination_fields() {
        let value = selection();
        let bytes = value.encode().unwrap();
        assert_eq!(PkiSelection::decode(&bytes).unwrap(), value);
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(PkiSelection::decode(&trailing).is_err());
        let mut unknown = bytes.clone();
        unknown[65] = 2;
        assert!(PkiSelection::decode(&unknown).is_err());
        for size in 0..bytes.len() {
            assert!(PkiSelection::decode(&bytes[..size]).is_err());
        }
    }
    #[test]
    fn zero_scope_unknown_empty_or_duplicate_references_fail_closed() {
        let mut value = selection();
        value.scope_binding = [0; 32];
        assert!(value.encode().is_err());
        let mut value = selection();
        value.ca_ids.push([2; 32]);
        assert!(value.encode().is_err());
        let mut value = selection();
        value.ca_ids.clear();
        value.client = None;
        assert!(value.encode().is_err());
        let mut value = selection();
        value.client.as_mut().unwrap().leaf_sha256 = [0; 32];
        assert!(value.encode().is_err());
    }
}
