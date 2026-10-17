use super::*;
use serde_json::{Value, json};
fn metadata() -> Value {
    json!({"schema_version":1,"purpose":"colossus-native-browser-reviewed-pki-registry",
        "cas":[{"id":hex::encode([2;32]),"slot":0,"sha256":hex::encode([3;32]),
            "origins":["https://fixture.test"]}],
        "identities":[{"id":hex::encode([4;32]),"slot":0,"leaf_sha256":hex::encode([5;32]),
            "pkcs12_sha256":hex::encode([6;32]),"password_sha256":hex::encode([7;32]),
            "ca_ids":[hex::encode([2;32])],"origins":["https://fixture.test"]}],
        "reviews":[{"workspace_id":"workspace","application_id":"application",
            "scope":{"kind":"future_conversations"},"origins":["https://fixture.test"],
            "ca_ids":[hex::encode([2;32])],"client_id":hex::encode([4;32])}]})
}
fn parses(value: &Value) -> bool {
    RegistryMetadata::parse(&serde_json::to_vec(value).unwrap()).is_ok()
}
#[test]
fn exact_public_catalog_and_review_parse_without_secret_or_path_fields() {
    assert!(parses(&metadata()));
    let mut ca_only = metadata();
    ca_only["identities"] = json!([]);
    ca_only["reviews"][0]["client_id"] = Value::Null;
    assert!(parses(&ca_only));
    let mut extra = metadata();
    extra["password"] = json!("received-secret");
    assert!(!parses(&extra));
    let mut extra = metadata();
    extra["identities"][0]["path"] = json!("/received-path");
    assert!(!parses(&extra));
}
#[test]
fn slot_digest_origin_issuer_and_scope_substitutions_are_rejected() {
    for field in ["id", "sha256"] {
        let mut changed = metadata();
        changed["cas"][0][field] = json!("0".repeat(64));
        assert!(!parses(&changed));
    }
    for origin in [
        "https://user@fixture.test",
        "https://fixture.test/extra",
        "http://fixture.test",
        "https://fixture.test:443",
        "https://foreign.test",
    ] {
        let mut changed = metadata();
        changed["reviews"][0]["origins"] = json!([origin]);
        assert!(!parses(&changed));
    }
    let mut changed = metadata();
    changed["cas"][0]["slot"] = json!(8);
    assert!(!parses(&changed));
    let mut changed = metadata();
    let mut second = changed["cas"][0].clone();
    second["id"] = json!(hex::encode([8; 32]));
    changed["cas"].as_array_mut().unwrap().push(second);
    assert!(!parses(&changed));
    let mut changed = metadata();
    changed["reviews"][0]["ca_ids"] = json!([]);
    assert!(!parses(&changed));
    let mut changed = metadata();
    changed["identities"][0]["ca_ids"] = json!([]);
    assert!(!parses(&changed));
    let mut changed = metadata();
    changed["reviews"][0]["scope"] = json!({"kind":"conversation","id":""});
    assert!(!parses(&changed));
}
