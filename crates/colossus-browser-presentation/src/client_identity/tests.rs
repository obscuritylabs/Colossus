use super::*;
use crate::PresentationCommand;

fn challenge() -> ClientIdentityChallenge {
    ClientIdentityChallenge {
        key: ClientIdentityKey {
            tab: 7,
            generation: 2,
            request_id: 19,
            control_generation: 0,
        },
        origin: BrowserOrigin::parse("https://example.test:8443").unwrap(),
        candidates: vec![ClientIdentityCandidate {
            fingerprint_sha256: "a".repeat(64),
            label: "Client certificate".into(),
        }],
        remaining_ms: 45_000,
    }
}

#[test]
fn native_identity_metadata_is_bounded_and_defaults_to_review() {
    assert_eq!(
        ClientIdentityPolicy::default(),
        ClientIdentityPolicy::Review
    );
    let value = challenge();
    value.validate().unwrap();
    let bytes = serde_json::to_vec(&value).unwrap();
    let decoded: ClientIdentityChallenge = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, value);
    for invalid in ["", "a", &"A".repeat(64), &"a".repeat(65)] {
        assert!(validate_fingerprint(invalid).is_err());
    }
    for label in ["", " label", "label\n", "\u{202e}label", &"x".repeat(129)] {
        let mut value = challenge();
        value.candidates[0].label = label.into();
        assert!(value.validate().is_err());
    }
}

#[test]
fn native_identity_challenge_rejects_ambiguous_expired_and_non_https_data() {
    for cause in 0..7 {
        let mut value = challenge();
        match cause {
            0 => value.key.tab = 0,
            1 => value.key.generation = 0,
            2 => value.key.request_id = 0,
            3 => value.remaining_ms = 0,
            4 => value.remaining_ms = 45_001,
            5 => value.origin = BrowserOrigin::parse("http://example.test").unwrap(),
            _ => value.candidates.push(value.candidates[0].clone()),
        }
        assert!(value.validate().is_err(), "case {cause}");
    }
    let mut value = challenge();
    value.candidates.clear();
    assert!(value.validate().is_err());
    value.candidates = (0..65)
        .map(|index| ClientIdentityCandidate {
            fingerprint_sha256: format!("{index:064x}"),
            label: "Client certificate".into(),
        })
        .collect();
    assert!(value.validate().is_err());
    value.candidates.pop();
    value.validate().unwrap();
}

#[test]
fn selection_accepts_only_a_native_key_and_an_exact_fingerprint_shape() {
    let key = challenge().key;
    for fingerprint_sha256 in [None, Some("a".repeat(64))] {
        PresentationCommand::SelectClientIdentity {
            key,
            fingerprint_sha256,
        }
        .validate()
        .unwrap();
    }
    assert!(
        PresentationCommand::SelectClientIdentity {
            key,
            fingerprint_sha256: Some("a".repeat(63)),
        }
        .validate()
        .is_err()
    );
    let mut json = serde_json::to_value(challenge()).unwrap();
    json["key"]["private_key"] = "forbidden".into();
    assert!(serde_json::from_value::<ClientIdentityChallenge>(json).is_err());
}
