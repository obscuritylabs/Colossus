//! Public signed registry metadata only; no secret data, caller paths or signers.
use serde::Deserialize;
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryMetadata {
    pub schema_version: u32,
    pub purpose: String,
    pub cas: Vec<RegisteredCa>,
    pub identities: Vec<RegisteredIdentity>,
    pub reviews: Vec<ReviewedScope>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisteredCa {
    pub id: String,
    pub slot: u8,
    pub sha256: String,
    pub origins: Vec<String>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisteredIdentity {
    pub id: String,
    pub slot: u8,
    pub leaf_sha256: String,
    pub pkcs12_sha256: String,
    pub password_sha256: String,
    pub ca_ids: Vec<String>,
    pub origins: Vec<String>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedScope {
    pub workspace_id: String,
    pub application_id: String,
    pub scope: ScopeConsent,
    pub origins: Vec<String>,
    pub ca_ids: Vec<String>,
    pub client_id: Option<String>,
}
#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScopeConsent {
    Conversation { id: String },
    Workflow { id: String },
    FutureConversations {},
    FutureWorkflows {},
    FutureConversationsAndWorkflows {},
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryError {
    Invalid,
}
impl RegistryMetadata {
    pub fn parse(bytes: &[u8]) -> Result<Self, RegistryError> {
        let invalid = RegistryError::Invalid;
        if bytes.is_empty() || bytes.len() > 128 * 1024 {
            return Err(invalid);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| invalid)?;
        if value.schema_version != 1
            || value.purpose != "colossus-native-browser-reviewed-pki-registry"
            || value.cas.len() > 8
            || value.identities.len() > 8
            || value.reviews.is_empty()
            || value.reviews.len() > 32
            || value.cas.is_empty()
        {
            return Err(invalid);
        }
        for (index, ca) in value.cas.iter().enumerate() {
            if !digest(&ca.id)
                || !digest(&ca.sha256)
                || ca.slot > 7
                || !origins(&ca.origins)
                || value.cas[..index]
                    .iter()
                    .any(|prior| prior.id == ca.id || prior.slot == ca.slot)
            {
                return Err(invalid);
            }
        }
        for (index, identity) in value.identities.iter().enumerate() {
            if !digest(&identity.id)
                || !digest(&identity.leaf_sha256)
                || !digest(&identity.pkcs12_sha256)
                || !digest(&identity.password_sha256)
                || identity.slot > 7
                || !origins(&identity.origins)
                || identity.ca_ids.len() > 8
                || identity.ca_ids.is_empty()
                || value.identities[..index]
                    .iter()
                    .any(|prior| prior.id == identity.id || prior.slot == identity.slot)
                || identity.ca_ids.iter().enumerate().any(|(n, id)| {
                    identity.ca_ids[..n].contains(id)
                        || !value.cas.iter().any(|ca| {
                            ca.id == *id
                                && identity
                                    .origins
                                    .iter()
                                    .all(|origin| ca.origins.contains(origin))
                        })
                })
            {
                return Err(invalid);
            }
        }
        for review in &value.reviews {
            if !id(&review.workspace_id)
                || !id(&review.application_id)
                || !origins(&review.origins)
                || review.ca_ids.len() > 8
                || review.ca_ids.is_empty()
            {
                return Err(invalid);
            }
            match &review.scope {
                ScopeConsent::Conversation { id: scope } | ScopeConsent::Workflow { id: scope }
                    if !id(scope) =>
                {
                    return Err(invalid);
                }
                _ => {}
            }
            if review.ca_ids.iter().enumerate().any(|(n, id)| {
                review.ca_ids[..n].contains(id)
                    || !value.cas.iter().any(|ca| {
                        ca.id == *id
                            && review
                                .origins
                                .iter()
                                .all(|origin| ca.origins.contains(origin))
                    })
            }) || review.client_id.as_ref().is_some_and(|id| {
                !value.identities.iter().any(|client| {
                    client.id == *id
                        && review
                            .origins
                            .iter()
                            .all(|origin| client.origins.contains(origin))
                        && client
                            .ca_ids
                            .iter()
                            .all(|issuer| review.ca_ids.contains(issuer))
                })
            }) {
                return Err(invalid);
            }
        }
        Ok(value)
    }
}

#[cfg(test)]
#[path = "registry_metadata/tests.rs"]
mod tests;
pub fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && !value.bytes().all(|b| b == b'0')
}
fn id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}
fn origins(values: &[String]) -> bool {
    !values.is_empty()
        && values.len() <= 8
        && values.iter().enumerate().all(|(n, value)| {
            value.len() <= 2048
                && !values[..n].contains(value)
                && url::Url::parse(value).is_ok_and(|url| {
                    url.scheme() == "https"
                        && url.origin().ascii_serialization() == *value
                        && url.path() == "/"
                        && url.query().is_none()
                        && url.fragment().is_none()
                        && url.username().is_empty()
                        && url.password().is_none()
                })
        })
}
