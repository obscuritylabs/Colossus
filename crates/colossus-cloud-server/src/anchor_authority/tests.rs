use super::*;
use colossus_contracts::{SecureAnchor, SecureAnchorStatus};
use sha2::{Digest as _, Sha256};
use std::{collections::BTreeMap, sync::Mutex};
use tower::ServiceExt as _;

const TOKEN: &str = "synthetic-independent-anchor-bearer";

#[test]
fn non_loopback_authority_requires_explicit_tls_ingress_and_secret_references() {
    let mut config:AnchorAuthorityConfig=serde_json::from_value(serde_json::json!({"bind":"0.0.0.0:8080","database":{"connectionVariable":"ANCHOR_DB","schema":"independent_anchors"},"bearerVariable":"ANCHOR_BEARER","storageKeyVariable":"ANCHOR_STORAGE_KEY"})).unwrap();
    assert!(config.validate().is_err());
    config.trusted_tls_ingress = true;
    config.validate().unwrap();
    config.bearer_variable = "literal-secret-with-dashes".into();
    assert!(config.validate().is_err());
}
#[derive(Default)]
struct MemoryRecords(Mutex<BTreeMap<String, (u64, Vec<u8>)>>);
impl records::AnchorRecords for MemoryRecords {
    fn read(&self, namespace: &str) -> CloudResult<ChatRecord> {
        let rows = self.0.lock().unwrap();
        Ok(rows.get(namespace).map_or(
            ChatRecord {
                revision: 0,
                ciphertext: None,
            },
            |(revision, bytes)| ChatRecord {
                revision: *revision,
                ciphertext: Some(bytes.clone()),
            },
        ))
    }
    fn write(&self, namespace: &str, expected: u64, bytes: &[u8]) -> CloudResult<()> {
        let mut rows = self.0.lock().unwrap();
        if rows.get(namespace).map_or(0, |row| row.0) != expected {
            return Err(CloudError::Conflict);
        }
        rows.insert(namespace.into(), (expected + 1, bytes.to_vec()));
        Ok(())
    }
}

fn state(records: Arc<dyn records::AnchorRecords>) -> Arc<AuthorityState> {
    Arc::new(AuthorityState {
        records,
        key: Zeroizing::new([7; 32]),
        bearer_hash: Sha256::digest(TOKEN.as_bytes()).into(),
        admission: Arc::new(tokio::sync::Semaphore::new(16)),
    })
}
fn update(sequence: u64, revision: u64) -> SharedAnchorUpdate {
    SharedAnchorUpdate {
        version: 1,
        expected_revision: revision,
        anchor: SecureAnchor {
            format_version: 2,
            sequence,
            hash: "a".repeat(64),
            verification_profile: Some("full-journal-v1".into()),
            status: SecureAnchorStatus::Verified,
        },
    }
}

#[test]
fn protected_anchor_records_bind_namespace_revision_and_key() {
    let snapshot = SharedAnchorSnapshot {
        version: 1,
        revision: 1,
        anchor: Some(update(20, 0).anchor),
    };
    let namespace = "a".repeat(64);
    let ciphertext = custody::encrypt(&[7; 32], &namespace, &snapshot).unwrap();
    assert!(!String::from_utf8_lossy(&ciphertext).contains("full-journal-v1"));
    assert_eq!(
        custody::decrypt(&[7; 32], &namespace, 1, &ciphertext).unwrap(),
        snapshot
    );
    assert!(custody::decrypt(&[7; 32], &"b".repeat(64), 1, &ciphertext).is_err());
    assert!(custody::decrypt(&[7; 32], &namespace, 2, &ciphertext).is_err());
    assert!(custody::decrypt(&[8; 32], &namespace, 1, &ciphertext).is_err());
}

#[test]
fn authority_rejects_stale_cas_rollback_divergence_and_ciphertext_corruption() {
    let records = Arc::new(MemoryRecords::default());
    let state = state(records.clone());
    let namespace = "a".repeat(64);
    assert_eq!(state.snapshot(&namespace).unwrap().revision, 0);
    assert_eq!(
        state.advance(&namespace, update(20, 0)).unwrap().revision,
        1
    );
    assert!(matches!(
        state.advance(&namespace, update(21, 0)),
        Err(CloudError::Conflict)
    ));
    assert!(matches!(
        state.advance(&namespace, update(19, 1)),
        Err(CloudError::Conflict)
    ));
    let mut divergent = update(20, 1);
    divergent.anchor.hash = "b".repeat(64);
    assert!(matches!(
        state.advance(&namespace, divergent),
        Err(CloudError::Conflict)
    ));
    assert_eq!(
        state.advance(&namespace, update(21, 1)).unwrap().revision,
        2
    );
    records.0.lock().unwrap().get_mut(&namespace).unwrap().1[30] ^= 1;
    assert!(matches!(
        state.snapshot(&namespace),
        Err(CloudError::Storage)
    ));
}

#[tokio::test]
async fn authority_http_requires_bearer_and_bounds_namespaces() {
    use axum::{body::Body, http::Request};
    let authority = AnchorAuthority {
        bind: "127.0.0.1:0".parse().unwrap(),
        state: state(Arc::new(MemoryRecords::default())),
    };
    let path = format!("/v1/anchors/{}", "a".repeat(64));
    let response = authority
        .router()
        .oneshot(Request::builder().uri(&path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = authority
        .router()
        .oneshot(
            Request::builder()
                .uri(&path)
                .header("Authorization", "Bearer synthetic-wrong-anchor-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = authority
        .router()
        .oneshot(
            Request::builder()
                .uri(&path)
                .header("Authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = authority
        .router()
        .oneshot(
            Request::builder()
                .uri("/v1/anchors/foreign-namespace")
                .header("Authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = serde_json::to_vec(&update(12, 0)).unwrap();
    let request = || {
        Request::builder()
            .method("POST")
            .uri(&path)
            .header("Authorization", format!("Bearer {TOKEN}"))
            .header("Content-Type", "application/json")
            .body(Body::from(body.clone()))
            .unwrap()
    };
    assert_eq!(
        authority
            .router()
            .oneshot(request())
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        authority
            .router()
            .oneshot(request())
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    let oversized = Request::builder()
        .method("POST")
        .uri(&path)
        .header("Authorization", format!("Bearer {TOKEN}"))
        .header("Content-Type", "application/json")
        .body(Body::from(vec![b' '; 20 * 1024]))
        .unwrap();
    assert_eq!(
        authority
            .router()
            .oneshot(oversized)
            .await
            .unwrap()
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[tokio::test]
#[ignore = "requires the independent local anchor PostgreSQL fixture and explicit authority keys"]
async fn independent_postgres_authority_reopens_encrypted_cas_state() {
    use colossus_cloud_postgres::CloudDatabaseTls;
    let config = AnchorAuthorityConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        database: CloudDatabaseConfig {
            connection_variable: "COLOSSUS_ANCHOR_DATABASE".into(),
            schema: format!("anchor_acceptance_{}", uuid::Uuid::now_v7().simple()),
            tls: CloudDatabaseTls::Disabled,
            max_connections: 2,
            connection_timeout_ms: 5000,
            statement_timeout_ms: 15000,
        },
        bearer_variable: "COLOSSUS_ANCHOR_BEARER".into(),
        storage_key_variable: "COLOSSUS_ANCHOR_STORAGE_KEY".into(),
        trusted_tls_ingress: false,
    };
    let authority = AnchorAuthority::open(config.clone()).await.unwrap();
    let namespace = "a".repeat(64);
    let state = authority.state.clone();
    tokio::task::spawn_blocking(move || {
        assert_eq!(
            state.advance(&namespace, update(40, 0)).unwrap().revision,
            1
        );
        let record = state.records.read(&namespace).unwrap();
        assert_eq!(record.revision, 1);
        assert!(!String::from_utf8_lossy(&record.ciphertext.unwrap()).contains("full-journal-v1"));
    })
    .await
    .unwrap();
    drop(authority);
    let authority = AnchorAuthority::open(config).await.unwrap();
    tokio::task::spawn_blocking(move || {
        let namespace = "a".repeat(64);
        let snapshot = authority.state.snapshot(&namespace).unwrap();
        assert_eq!(snapshot.anchor.unwrap().sequence, 40);
        assert!(matches!(
            authority.state.advance(&namespace, update(39, 1)),
            Err(CloudError::Conflict)
        ));
        assert_eq!(
            authority
                .state
                .advance(&namespace, update(41, 1))
                .unwrap()
                .revision,
            2
        );
    })
    .await
    .unwrap();
}
