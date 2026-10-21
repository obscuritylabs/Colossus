//! A compiler-sealed functional preview is distinct from production acceptance.
use serde::Deserialize;

/// The only target supported by this explicit preview edition.
pub const TARGET: &str = "x86_64-pc-windows-msvc";
/// Fixed purpose; ordinary installed-browser descriptors cannot select this owner.
pub const PURPOSE: &str = "colossus-windows-browser-functional-preview";
/// Fixed installation-relative resource location, never a runtime configuration path.
pub const COMPONENT_PATH: &str = "browser/component";
/// Exact pinned native engine version.
pub const CEF_VERSION: &str = "154.0.34+g14c5a08+chromium-154.0.8037.98";
/// Exact pinned source archive, before publisher signing transforms its binaries.
pub const ARCHIVE_SHA256: &str = "c81c34d048d87276f4a18f0b000206710a1709bb67dd658948b87100e1a538da";
/// Compiler input and runtime decoding share this ceiling.
pub const MAX_BYTES: usize = 4096;

/// Nonsecret publisher selection embedded in the subsequently signed sidecar.
/// The signed Desktop bundle independently binds that sidecar. The adjacent
/// component cannot authorize itself by replacing its own inventory.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seal {
    schema_version: u32,
    purpose: String,
    target: String,
    component_path: String,
    /// Digest of the final inventory after every executable/DLL signing mutation.
    pub component_manifest_sha256: String,
    cef_version: String,
    source_archive_sha256: String,
    production_acceptance: bool,
    capabilities: Capabilities,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Capabilities {
    embedded: bool,
    headless: bool,
    restrictive_egress: bool,
    private_ca_trust: bool,
    client_identities: bool,
}

impl Seal {
    /// Decode only the exact preview purpose, relative path, pin and truthful
    /// capability ceiling. Successful parsing grants no native authority.
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err("functional-preview seal is absent or oversized");
        }
        let seal: Self =
            serde_json::from_slice(bytes).map_err(|_| "functional-preview seal is malformed")?;
        let cap = &seal.capabilities;
        let digest = &seal.component_manifest_sha256;
        if seal.schema_version != 1
            || seal.purpose != PURPOSE
            || seal.target != TARGET
            || seal.component_path != COMPONENT_PATH
            || seal.cef_version != CEF_VERSION
            || seal.source_archive_sha256 != ARCHIVE_SHA256
            || seal.production_acceptance
            || !cap.embedded
            || cap.headless
            || cap.restrictive_egress
            || cap.private_ca_trust
            || !cap.client_identities
            || digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || digest.bytes().all(|byte| byte == b'0')
        {
            return Err("functional-preview seal has an unaccepted ceiling or location");
        }
        Ok(seal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1, "purpose": PURPOSE, "target": TARGET,
            "component_path": COMPONENT_PATH, "component_manifest_sha256": "a".repeat(64),
            "cef_version": CEF_VERSION, "source_archive_sha256": ARCHIVE_SHA256,
            "production_acceptance": false,
            "capabilities": { "embedded": true, "headless": false,
                "restrictive_egress": false, "private_ca_trust": false, "client_identities": true }
        })
    }

    #[test]
    fn only_the_relative_publisher_selected_preview_is_accepted() {
        assert!(Seal::parse(&serde_json::to_vec(&valid()).unwrap()).is_ok());
        for path in [
            "C:/machine/component",
            "../component",
            "browser/../component",
            "component",
        ] {
            let mut value = valid();
            value["component_path"] = path.into();
            assert!(Seal::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        let mut value = valid();
        value["state_root"] = "C:/machine/state".into();
        assert!(Seal::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }

    #[test]
    fn false_production_network_or_certificate_claims_cannot_be_promoted() {
        for key in ["headless", "restrictive_egress", "private_ca_trust"] {
            let mut value = valid();
            value["capabilities"][key] = true.into();
            assert!(Seal::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        let mut value = valid();
        value["capabilities"]["client_identities"] = false.into();
        assert!(Seal::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        for (key, replacement) in [
            ("production_acceptance", serde_json::json!(true)),
            ("target", serde_json::json!("x86_64-unknown-linux-gnu")),
            (
                "component_manifest_sha256",
                serde_json::json!("0".repeat(64)),
            ),
            (
                "component_manifest_sha256",
                serde_json::json!("A".repeat(64)),
            ),
        ] {
            let mut value = valid();
            value[key] = replacement;
            assert!(Seal::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        assert!(Seal::parse(&[]).is_err());
        assert!(Seal::parse(&vec![b' '; MAX_BYTES + 1]).is_err());
    }
}
