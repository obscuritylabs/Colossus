use super::*;

const CONTENT: &[u8] = include_bytes!("../tests/fixtures/sigstore/cosign-v3.txt");
const BUNDLE: &[u8] = include_bytes!("../tests/fixtures/sigstore/cosign-v3.sigstore.json");
const PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEFodOSridzGjgIAIl3/2N+eP4dMBE\nM0oMNJnbWLPBnASGSdjtYr8KvEoxYXTqc47tu22hKYyfnNPkADR1Q9FXeA==\n-----END PUBLIC KEY-----\n";

#[test]
fn cosign_key_and_keyless_evidence_verify_offline_against_local_roots() {
    let root = tempfile::tempdir().expect("fixture");
    let trust_root_path = root.path().join("trusted-root.json");
    fs::write(
        &trust_root_path,
        include_bytes!("../tests/fixtures/sigstore/trusted-root.json"),
    )
    .expect("local trust roots");
    let public_key_path = root.path().join("key.pem");
    fs::write(&public_key_path, PUBLIC_KEY).expect("public key");
    let mut profile = PluginTrustProfile {
        public_keys: vec![public_key_path],
        trust_root_path: Some(trust_root_path),
        ..PluginTrustProfile::default()
    };
    let result = verify_plugin_trust("offline-key", &profile, CONTENT, &[BUNDLE.to_vec()])
        .expect("Cosign public key with transparency proof");
    assert!(result.trusted);
    assert_eq!(result.method, "sigstore-key");
    profile.public_keys.clear();
    profile.identities.push(SigstoreIdentity {
        issuer: "https://github.com/login/oauth".into(),
        subject: "w.vollprecht@gmail.com".into(),
    });
    let result = verify_plugin_trust("offline-keyless", &profile, CONTENT, &[BUNDLE.to_vec()])
        .expect("Cosign keyless with local Fulcio/Rekor/TSA roots");
    assert!(result.trusted);
    assert_eq!(result.method, "sigstore-keyless");
    assert!(
        verify_plugin_trust(
            "wrong-artifact",
            &profile,
            b"another OCI manifest",
            &[BUNDLE.to_vec()]
        )
        .is_err()
    );
    profile.identities[0].subject = "another@example.test".into();
    assert!(verify_plugin_trust("wrong-identity", &profile, CONTENT, &[BUNDLE.to_vec()]).is_err());
}

#[test]
fn forged_or_incomplete_transparency_evidence_never_satisfies_required_trust() {
    let root = tempfile::tempdir().expect("fixture");
    let public_key_path = root.path().join("key.pem");
    fs::write(&public_key_path, PUBLIC_KEY).expect("public key");
    let profile = PluginTrustProfile {
        public_keys: vec![public_key_path],
        ..PluginTrustProfile::default()
    };
    for mutation in [
        "missing-log",
        "missing-checkpoint",
        "forged-checkpoint",
        "forged-signature",
        "wrong-digest",
    ] {
        let mut bundle: Value = serde_json::from_slice(BUNDLE).expect("standard bundle");
        match mutation {
            "missing-log" => bundle["verificationMaterial"]["tlogEntries"] = json!([]),
            "missing-checkpoint" => {
                bundle["verificationMaterial"]["tlogEntries"][0]["inclusionProof"]
                    .as_object_mut()
                    .expect("proof")
                    .remove("checkpoint");
            }
            "forged-checkpoint" => {
                bundle["verificationMaterial"]["tlogEntries"][0]["inclusionProof"]["checkpoint"]["envelope"] =
                    json!("forged")
            }
            "forged-signature" => bundle["messageSignature"]["signature"] = json!("Zm9yZ2Vk"),
            "wrong-digest" => {
                bundle["messageSignature"]["messageDigest"]["digest"] =
                    json!("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")
            }
            _ => unreachable!(),
        }
        let bytes = serde_json::to_vec(&bundle).expect("mutated bundle");
        assert!(
            verify_plugin_trust(mutation, &profile, CONTENT, std::slice::from_ref(&bytes)).is_err(),
            "reject {mutation}"
        );
        let optional = PluginTrustProfile {
            mode: PluginTrustMode::Optional,
            ..profile.clone()
        };
        assert!(
            !verify_plugin_trust(mutation, &optional, CONTENT, &[bytes])
                .expect("untrusted optional candidate")
                .trusted
        );
    }
}
