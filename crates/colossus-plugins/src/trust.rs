use super::*;

#[cfg(test)]
#[path = "trust_tests.rs"]
mod tests;
use sigstore_verify::{
    VerificationPolicy, trust_root::SIGSTORE_PRODUCTION_TRUSTED_ROOT, trust_root::TrustedRoot,
    types::Bundle, types::DerPublicKey, verify, verify_with_key,
};

/// Apply one reusable Sigstore trust profile to an OCI manifest and bundled evidence.
///
/// `bundles` are standard Sigstore bundle JSON documents, normally downloaded from
/// OCI 1.1 Cosign referrers or carried alongside an air-gap layout. No network access
/// is performed by this function.
pub fn verify_plugin_trust(
    profile_name: &str,
    profile: &PluginTrustProfile,
    manifest: &[u8],
    bundles: &[Vec<u8>],
) -> Result<PluginTrustEvidence, StoreError> {
    if profile_name.is_empty() {
        return Err(StoreError::Adapter("trust profile name is required".into()));
    }
    if profile.mode == PluginTrustMode::Disabled {
        return Ok(PluginTrustEvidence {
            trusted: false,
            profile: Some(profile_name.into()),
            signer: None,
            method: "digest-only".into(),
        });
    }
    let trusted_root = load_trusted_root(profile)?;
    for bytes in bundles {
        if u64::try_from(bytes.len()).map_err(adapter)? > MAX_MANIFEST_BYTES {
            continue;
        }
        let Ok(json) = std::str::from_utf8(bytes) else {
            continue;
        };
        let Ok(bundle) = Bundle::from_json(json) else {
            continue;
        };
        for key_path in &profile.public_keys {
            let Ok(key_pem) = read_bounded(key_path, MAX_MANIFEST_BYTES)
                .and_then(|bytes| String::from_utf8(bytes).map_err(adapter))
            else {
                continue;
            };
            let Ok(key) = DerPublicKey::from_pem(&key_pem) else {
                continue;
            };
            if verify_with_key(manifest, &bundle, &key, &trusted_root).is_ok() {
                return Ok(PluginTrustEvidence {
                    trusted: true,
                    profile: Some(profile_name.into()),
                    signer: Some(public_key_fingerprint(&key_pem)),
                    method: "sigstore-key".into(),
                });
            }
        }
        for identity in &profile.identities {
            let policy = VerificationPolicy::default()
                .require_issuer(identity.issuer.clone())
                .require_identity(identity.subject.clone());
            if verify(manifest, &bundle, &policy, &trusted_root).is_ok() {
                return Ok(PluginTrustEvidence {
                    trusted: true,
                    profile: Some(profile_name.into()),
                    signer: Some(format!("{}|{}", identity.issuer, identity.subject)),
                    method: "sigstore-keyless".into(),
                });
            }
        }
    }
    if profile.mode == PluginTrustMode::Required {
        Err(StoreError::Verification(format!(
            "Agent Plugin signature does not match required trust profile {profile_name}"
        )))
    } else {
        Ok(PluginTrustEvidence {
            trusted: false,
            profile: Some(profile_name.into()),
            signer: None,
            method: "sigstore-unmatched".into(),
        })
    }
}

fn load_trusted_root(profile: &PluginTrustProfile) -> Result<TrustedRoot, StoreError> {
    if let Some(path) = &profile.trust_root_path {
        let bytes = read_bounded(path, MAX_MANIFEST_BYTES)?;
        let json = String::from_utf8(bytes).map_err(adapter)?;
        TrustedRoot::from_json(&json).map_err(adapter)
    } else {
        TrustedRoot::from_json(SIGSTORE_PRODUCTION_TRUSTED_ROOT).map_err(adapter)
    }
}

fn public_key_fingerprint(pem: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(pem.as_bytes())))
}
