//! Shared strict publisher descriptors used by executable build seals and runtime discovery.
use colossus_bundles::{BundleService, BundleTrustStore};
use colossus_contracts::{BrowserCapabilities, BrowserLimits, BrowserMode, BundleManifest};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

/// Locked publisher manifest purpose for an installed Linux browser.
pub const MANIFEST_NAME: &str = "colossus-owned-browser-linux-release";
/// Only currently accepted installed package target.
pub const TARGET: &str = "x86_64-unknown-linux-gnu";
/// Exact bounded descriptor files signed by the publisher.
pub const FIXED_FILES: [&str; 3] = [
    "acceptance.json",
    "browser-release.json",
    "component/browser-component.json",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Publisher {
    publisher: String,
    purpose: String,
    algorithm: String,
    key_id: String,
    public_key: String,
}

/// Return only the independently compiled release publisher public key.
pub fn trust() -> Result<BundleTrustStore, &'static str> {
    let publisher: Publisher =
        serde_json::from_str(include_str!("../../../../release/bundle-publisher.json"))
            .map_err(|_| "compiled browser publisher trust is invalid")?;
    if publisher.publisher != "colossus"
        || publisher.purpose != "offline-bundle-manifest-signing"
        || publisher.algorithm != "ed25519"
    {
        return Err("compiled browser publisher trust is invalid");
    }
    Ok(BTreeMap::from([(
        publisher.publisher,
        BTreeMap::from([(publisher.key_id, publisher.public_key)]),
    )]))
}

/// Verify the exact manifest purpose, payload bindings and publisher signature.
pub fn signed(bytes: &[u8], trust: BundleTrustStore) -> Result<BundleManifest, &'static str> {
    if bytes.is_empty() || bytes.len() > 1024 * 1024 {
        return Err("browser publisher binding is absent or oversized");
    }
    let manifest: BundleManifest =
        serde_json::from_slice(bytes).map_err(|_| "browser publisher manifest is invalid")?;
    if manifest.name != MANIFEST_NAME
        || manifest.publisher != "colossus"
        || manifest.files.len() != FIXED_FILES.len()
        || manifest.files.iter().zip(FIXED_FILES).any(|(entry, path)| {
            entry.path != path
                || !digest(&entry.sha256)
                || entry
                    .size
                    .is_none_or(|size| size == 0 || size > 8 * 1024 * 1024)
        })
    {
        return Err("browser publisher manifest has invalid payload bindings");
    }
    BundleService::new(trust)
        .verify_manifest_signature(&manifest)
        .map_err(|_| "browser publisher signature is not trusted")?;
    Ok(manifest)
}

/// Accept only a canonical lowercase SHA-256 hexadecimal digest.
pub fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
/// Signed native package descriptor; validation grants no runtime availability.
pub struct Release {
    /// Strict descriptor schema version.
    pub schema_version: u32,
    /// Exact locked publisher manifest purpose.
    pub purpose: String,
    /// Exact native target accepted by this compiler.
    pub target: String,
    /// Immutable preloaded OCI image identity.
    pub image_id: String,
    /// Digest of the offline OCI archive distributed by the publisher.
    pub image_archive_sha256: String,
    /// Digest of the complete immutable native component inventory.
    pub component_manifest_sha256: String,
    /// Accepted native capability ceiling; runtime policy can only narrow it.
    pub capabilities: BrowserCapabilities,
}
impl Release {
    /// Require the exact supported target and bounded implemented capabilities.
    pub fn validate(&self) -> Result<(), &'static str> {
        let cap = &self.capabilities;
        let limits = &cap.limits;
        let hard = BrowserLimits::default();
        let dimensions = [
            (u32::from(limits.max_sessions), u32::from(hard.max_sessions)),
            (u32::from(limits.max_tabs), u32::from(hard.max_tabs)),
            (
                u32::from(limits.max_concurrent_actions),
                u32::from(hard.max_concurrent_actions),
            ),
            (
                u32::from(limits.max_snapshot_nodes),
                u32::from(hard.max_snapshot_nodes),
            ),
            (limits.max_observation_bytes, hard.max_observation_bytes),
            (limits.action_timeout_ms, hard.action_timeout_ms),
            (limits.navigation_timeout_ms, hard.navigation_timeout_ms),
            (limits.max_lease_ms, hard.max_lease_ms),
        ];
        if self.schema_version != 1
            || self.purpose != MANIFEST_NAME
            || self.target != TARGET
            || self
                .image_id
                .strip_prefix("sha256:")
                .is_none_or(|value| !digest(value))
            || !digest(&self.image_archive_sha256)
            || !digest(&self.component_manifest_sha256)
            || !cap.available
            || !cap.restrictive_egress
            || cap.modes != [BrowserMode::Headless]
            || cap.private_ca_trust
            || cap.client_identities
            || cap.engine_version.as_ref().is_none_or(|value| {
                value.is_empty() || value.len() > 128 || value.chars().any(char::is_control)
            })
            || cap.actions.is_empty()
            || cap
                .actions
                .iter()
                .map(|kind| format!("{kind:?}"))
                .collect::<BTreeSet<_>>()
                .len()
                != cap.actions.len()
            || dimensions
                .iter()
                .any(|(value, ceiling)| *value == 0 || value > ceiling)
        {
            return Err("browser release ceiling is unaccepted or invalid");
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
/// Publisher-bound production acceptance claims for the exact installed artifacts.
pub struct Acceptance {
    /// Strict descriptor schema version.
    pub schema_version: u32,
    /// Exact native target accepted by this compiler.
    pub target: String,
    /// Digest of the complete immutable native component inventory.
    pub component_manifest_sha256: String,
    /// Immutable preloaded OCI image identity.
    pub image_id: String,
    /// Digest of the offline OCI archive distributed by the publisher.
    pub image_archive_sha256: String,
    /// Native Chromium renderer sandbox acceptance for the exact artifact.
    pub chromium_sandbox_verified: bool,
    /// OS process-tree containment acceptance for the exact artifact.
    pub process_tree_containment_verified: bool,
    /// Observed OS denial of proxy-bypassing browser network traffic.
    pub egress_denial_verified: bool,
    /// Observed native execution of the typed browser action vocabulary.
    pub typed_actions_verified: bool,
    /// Positive native process, channel and profile cleanup acceptance.
    pub cleanup_verified: bool,
    /// Installed offline payload and pinned image acceptance.
    pub installed_browser_package_verified: bool,
    /// Explicit publisher release acceptance; development receipts remain false.
    pub production_acceptance: bool,
}
impl Acceptance {
    /// Require every native production claim to bind the same accepted release.
    pub fn validate(&self, release: &Release) -> Result<(), &'static str> {
        if self.schema_version != 1
            || self.target != release.target
            || self.component_manifest_sha256 != release.component_manifest_sha256
            || self.image_id != release.image_id
            || self.image_archive_sha256 != release.image_archive_sha256
            || ![
                self.chromium_sandbox_verified,
                self.process_tree_containment_verified,
                self.egress_denial_verified,
                self.typed_actions_verified,
                self.cleanup_verified,
                self.installed_browser_package_verified,
                self.production_acceptance,
            ]
            .into_iter()
            .all(|value| value)
        {
            return Err("browser installed production acceptance is absent or mismatched");
        }
        Ok(())
    }
}
