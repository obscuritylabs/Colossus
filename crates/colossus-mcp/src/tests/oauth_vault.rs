use super::*;
use colossus_contracts::{CredentialError, MAX_VAULT_RECORD_BYTES, VaultRecord};
use colossus_ports::{CredentialKey, CredentialVault};
use redb::ReadableTable as _;
use rmcp::transport::auth::{CredentialStore as _, StoredCredentials};

#[derive(Default)]
struct MemoryVault(Mutex<BTreeMap<CredentialKey, VaultRecord>>);

impl CredentialVault for MemoryVault {
    fn read(&self, key: &CredentialKey) -> Result<Option<VaultRecord>, CredentialError> {
        self.0
            .lock()
            .unwrap()
            .get(key)
            .map(|value| VaultRecord::new(value.expose().to_vec()))
            .transpose()
    }
    fn write(&self, key: &CredentialKey, record: &VaultRecord) -> Result<(), CredentialError> {
        self.0
            .lock()
            .unwrap()
            .insert(key.clone(), VaultRecord::new(record.expose().to_vec())?);
        Ok(())
    }
    fn delete(&self, key: &CredentialKey) -> Result<(), CredentialError> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
    fn contains(&self, key: &CredentialKey) -> Result<bool, CredentialError> {
        Ok(self.0.lock().unwrap().contains_key(key))
    }
}

fn credentials(size: usize) -> StoredCredentials {
    serde_json::from_value(json!({
        "client_id": "native-client", "granted_scopes": ["openid"], "token_received_at": 1,
        "token_response": {"access_token": "A".repeat(size), "refresh_token": "R".repeat(size), "token_type": "Bearer"}
    })).unwrap()
}

#[tokio::test]
async fn platform_oauth_uses_injected_vault_for_large_records_and_isolates_identity() {
    let vault = Arc::new(MemoryVault::default());
    let factory = OAuthStoreFactory::platform(vault.clone(), "repository".into());
    let store = factory.store("splunk", "https://splunk.example/mcp");
    assert!(store.load().await.unwrap().is_none());
    let original = credentials(64 * 1024);
    store.save(original.clone()).await.unwrap();
    let restored = factory
        .store("splunk", "https://splunk.example/mcp")
        .load()
        .await
        .unwrap()
        .unwrap();
    assert!(serde_json::to_value(restored).unwrap() == serde_json::to_value(original).unwrap());
    for (server, endpoint) in [
        ("other", "https://splunk.example/mcp"),
        ("splunk", "https://other.example/mcp"),
    ] {
        assert!(
            factory
                .store(server, endpoint)
                .load()
                .await
                .unwrap()
                .is_none()
        );
    }
    let other = OAuthStoreFactory::platform(vault.clone(), "other-repository".into());
    assert!(
        other
            .store("splunk", "https://splunk.example/mcp")
            .load()
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .save(credentials(MAX_VAULT_RECORD_BYTES))
            .await
            .is_err()
    );
    assert_eq!(vault.0.lock().unwrap().len(), 1);
    store.clear().await.unwrap();
    store.clear().await.unwrap();
    assert!(store.load().await.unwrap().is_none());
}

#[tokio::test]
async fn malformed_vault_oauth_payload_is_rejected_without_disclosing_content() {
    let vault = Arc::new(MemoryVault::default());
    let store = OAuthStoreFactory::platform(vault.clone(), "repo".into())
        .store("fixture", "https://example.com/mcp");
    store.save(credentials(8192)).await.unwrap();
    let key = vault.0.lock().unwrap().keys().next().unwrap().clone();
    vault
        .write(
            &key,
            &VaultRecord::new(b"private-malformed-json".to_vec()).unwrap(),
        )
        .unwrap();
    let error = store.load().await.unwrap_err().to_string();
    assert!(!error.contains("private-malformed-json"));
}

#[tokio::test]
async fn explicit_state_oauth_modes_keep_large_records_and_reject_oversized_payloads() {
    let directory = tempfile::tempdir().unwrap();
    let keys = Arc::new(RotatingTestKeys {
        active: Mutex::new("key".into()),
        keys: BTreeMap::from([("key".into(), [7; 32])]),
    });
    let factories = [
        OAuthStoreFactory::ephemeral_state("repo".into()).unwrap(),
        OAuthStoreFactory::plaintext_state(&directory.path().join("plain.redb"), "repo".into())
            .unwrap(),
        OAuthStoreFactory::encrypted_state(
            &directory.path().join("encrypted.redb"),
            keys,
            "repo".into(),
        )
        .unwrap(),
    ];
    for factory in factories {
        let store = factory.store("fixture", "https://example.com/mcp");
        store.save(credentials(64 * 1024)).await.unwrap();
        assert!(store.load().await.unwrap().is_some());
        assert!(
            store
                .save(credentials(MAX_VAULT_RECORD_BYTES))
                .await
                .is_err()
        );
        assert!(store.load().await.unwrap().is_some());
    }
}

#[tokio::test]
async fn plaintext_oauth_state_bounds_the_wrapper_before_parsing() {
    let factory = OAuthStoreFactory::ephemeral_state("repo".into()).unwrap();
    let store = factory.store("fixture", "https://example.com/mcp");
    let mut value = StoredCredentials::new(String::new(), None, Vec::new(), None);
    let overhead = crate::oauth_record::encode(&value).unwrap().len();
    value.client_id = "x".repeat(MAX_VAULT_RECORD_BYTES - overhead);
    store.save(value).await.unwrap();
    assert!(store.load().await.unwrap().is_some());
    let crate::oauth_store::OAuthCredentialStore::PlaintextState { database, identity } = &store
    else {
        panic!("expected plaintext test store");
    };
    let write = database.begin_write().unwrap();
    {
        let mut table = write.open_table(OAUTH_RECORDS).unwrap();
        let mut record = table
            .get(identity.as_str())
            .unwrap()
            .unwrap()
            .value()
            .to_vec();
        assert_eq!(
            record.len(),
            crate::oauth_record::MAX_PLAINTEXT_STATE_RECORD_BYTES
        );
        // Appended JSON whitespace keeps the document valid, but exceeds its bound.
        record.push(b' ');
        table.insert(identity.as_str(), record.as_slice()).unwrap();
    }
    write.commit().unwrap();
    assert!(store.load().await.is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn native_oauth_operations_run_off_the_async_executor() {
    struct ThreadCheckingVault(std::thread::ThreadId);
    impl CredentialVault for ThreadCheckingVault {
        fn read(&self, _: &CredentialKey) -> Result<Option<VaultRecord>, CredentialError> {
            assert_ne!(std::thread::current().id(), self.0);
            Ok(None)
        }
        fn write(&self, _: &CredentialKey, _: &VaultRecord) -> Result<(), CredentialError> {
            assert_ne!(std::thread::current().id(), self.0);
            Ok(())
        }
        fn delete(&self, _: &CredentialKey) -> Result<(), CredentialError> {
            assert_ne!(std::thread::current().id(), self.0);
            Ok(())
        }
    }
    let vault = Arc::new(ThreadCheckingVault(std::thread::current().id()));
    let store = OAuthStoreFactory::platform(vault, "repo".into())
        .store("fixture", "https://example.com/mcp");
    store.load().await.unwrap();
    store.save(credentials(8192)).await.unwrap();
    store.clear().await.unwrap();
}
