use super::*;
use crate::{ChatAuthentication, ChatConnectionKind, ChatCredentialResolver};
use colossus_ports::CredentialResolver;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
struct Vault {
    records: Mutex<BTreeMap<CredentialKey, Vec<u8>>>,
    fail: AtomicBool,
}
impl CredentialVault for Vault {
    fn read(&self, key: &CredentialKey) -> Result<Option<VaultRecord>, CredentialError> {
        self.records
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .map(VaultRecord::new)
            .transpose()
    }
    fn write(&self, key: &CredentialKey, record: &VaultRecord) -> Result<(), CredentialError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(CredentialError::Unavailable);
        }
        self.records
            .lock()
            .unwrap()
            .insert(key.clone(), record.expose().to_vec());
        Ok(())
    }
    fn delete(&self, key: &CredentialKey) -> Result<(), CredentialError> {
        self.records.lock().unwrap().remove(key);
        Ok(())
    }
}
fn binding(user: &str, generation: u64, endpoint: &str) -> ChatConnectionBinding {
    ChatConnectionBinding::new(
        "project",
        user,
        ChatConnectionKind::Provider,
        "api",
        generation,
        endpoint,
        ChatAuthentication::Personal,
    )
    .unwrap()
    .unwrap()
}
fn metadata(id: &str) -> StoredCredentialMetadata {
    StoredCredentialMetadata {
        id: id.into(),
        label: "My API key".into(),
        kind: StoredCredentialKind::ApiKey,
    }
}
fn secret(value: &str) -> HostSecret {
    HostSecret::new(value).unwrap()
}

#[test]
fn unused_credentials_survive_reopen_but_cannot_be_resolved_by_id() {
    let vault = Arc::new(Vault::default());
    let mut store = CredentialStore::load(vault.as_ref()).unwrap();
    store
        .create(metadata("key-a"), &secret("private-value"))
        .unwrap();
    store.commit(vault.as_ref()).unwrap();
    let reopened = CredentialStore::load(vault.as_ref()).unwrap();
    assert_eq!(reopened.revision(), 1);
    let public = serde_json::to_string(&reopened.metadata().collect::<Vec<_>>()).unwrap();
    assert!(public.contains("My API key"));
    assert!(!public.contains("private-value"));
    let bound = binding("alice", 1, "https://api.example/v1");
    let resolver = ChatCredentialResolver::new(
        "project",
        "alice",
        [(bound.clone(), vault.clone() as Arc<dyn CredentialVault>)],
    )
    .unwrap();
    assert!(resolver.resolve("key-a").is_err());
    assert!(
        resolver
            .resolve(&ChatCredentialResolver::reference(&bound))
            .is_err()
    );
}

#[test]
fn explicit_attachments_rotate_together_and_endpoint_changes_need_a_new_attachment() {
    let vault = Vault::default();
    let first = binding("alice", 1, "https://api.example/v1");
    let second = binding("alice", 2, "https://other.example/v1");
    let mut store = CredentialStore::load(&vault).unwrap();
    store
        .create(metadata("key-a"), &secret("original"))
        .unwrap();
    store.attach(&first, Some("key-a")).unwrap();
    store.commit(&vault).unwrap();
    assert!(
        CredentialStore::resolve_bound(&vault, &second)
            .unwrap()
            .is_none()
    );
    store.attach(&second, Some("key-a")).unwrap();
    store
        .update(metadata("key-a"), Some(&secret("rotated")))
        .unwrap();
    store.commit(&vault).unwrap();
    for destination in [&first, &second] {
        assert_eq!(
            CredentialStore::resolve_bound(&vault, destination)
                .unwrap()
                .unwrap()
                .expose(),
            b"rotated"
        );
    }
    let other_owner = Vault::default();
    assert!(
        CredentialStore::resolve_bound(&other_owner, &first)
            .unwrap()
            .is_none()
    );
    assert!(
        CredentialStore::resolve_bound(&vault, &binding("bob", 1, "https://api.example/v1"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn legacy_values_remain_available_until_explicit_adoption_and_never_reappear_after_removal() {
    let vault = Vault::default();
    let bound = binding("alice", 1, "https://api.example/v1");
    vault
        .write(
            &legacy_key(&bound).unwrap(),
            &VaultRecord::new(b"retained".to_vec()).unwrap(),
        )
        .unwrap();
    assert_eq!(
        CredentialStore::resolve_bound(&vault, &bound)
            .unwrap()
            .unwrap()
            .expose(),
        b"retained"
    );
    let mut store = CredentialStore::load(&vault).unwrap();
    let id = store
        .adopt_legacy(&vault, &bound, "Retained key", StoredCredentialKind::ApiKey)
        .unwrap();
    store
        .update(
            StoredCredentialMetadata {
                id: id.clone(),
                ..metadata("ignored")
            },
            Some(&secret("replacement")),
        )
        .unwrap();
    store.commit(&vault).unwrap();
    assert_eq!(
        CredentialStore::resolve_bound(&vault, &bound)
            .unwrap()
            .unwrap()
            .expose(),
        b"replacement"
    );
    assert!(vault.read(&legacy_key(&bound).unwrap()).unwrap().is_none());
    store.remove(&id).unwrap();
    store.commit(&vault).unwrap();
    // Even restored obsolete material cannot bypass the durable disconnect marker.
    vault
        .write(
            &legacy_key(&bound).unwrap(),
            &VaultRecord::new(b"obsolete".to_vec()).unwrap(),
        )
        .unwrap();
    assert!(
        CredentialStore::resolve_bound(&vault, &bound)
            .unwrap()
            .is_none()
    );
    assert!(
        CredentialStore::load(&vault)
            .unwrap()
            .metadata()
            .next()
            .is_none()
    );
}

#[test]
fn failed_or_oversized_commits_preserve_the_previous_secret_and_revision() {
    let vault = Vault::default();
    let bound = binding("alice", 1, "https://api.example/v1");
    let mut store = CredentialStore::load(&vault).unwrap();
    store.save_bound(&bound, &secret("original")).unwrap();
    store.commit(&vault).unwrap();
    store.save_bound(&bound, &secret("replacement")).unwrap();
    vault.fail.store(true, Ordering::SeqCst);
    assert!(store.commit(&vault).is_err());
    vault.fail.store(false, Ordering::SeqCst);
    assert_eq!(CredentialStore::load(&vault).unwrap().revision(), 1);
    assert_eq!(
        CredentialStore::resolve_bound(&vault, &bound)
            .unwrap()
            .unwrap()
            .expose(),
        b"original"
    );
    for index in 0..20 {
        store
            .create(
                metadata(&format!("large-{index}")),
                &secret(&"a".repeat(65536)),
            )
            .unwrap();
    }
    assert!(matches!(
        store.commit(&vault),
        Err(CredentialError::Oversized)
    ));
    assert_eq!(CredentialStore::load(&vault).unwrap().revision(), 1);
}

#[test]
fn corrupt_named_state_never_falls_back_to_a_legacy_value() {
    let vault = Vault::default();
    let bound = binding("alice", 1, "https://api.example/v1");
    vault
        .write(
            &legacy_key(&bound).unwrap(),
            &VaultRecord::new(b"retained".to_vec()).unwrap(),
        )
        .unwrap();
    vault
        .write(
            &store_key().unwrap(),
            &VaultRecord::new(b"{}".to_vec()).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        CredentialStore::resolve_bound(&vault, &bound),
        Err(CredentialError::Corrupt)
    ));
}
