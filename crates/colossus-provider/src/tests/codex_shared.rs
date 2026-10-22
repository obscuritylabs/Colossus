use super::*;
use colossus_codex_auth::{CodexAuthRecord, CodexAuthStorage};

struct MemoryStorage(std::sync::Mutex<CodexAuthRecord>);

impl CodexAuthStorage for MemoryStorage {
    fn read(&self) -> Result<CodexAuthRecord, CodexAuthError> {
        let record = self.0.lock().unwrap();
        Ok(CodexAuthRecord {
            revision: record.revision,
            bytes: record.bytes.clone(),
        })
    }

    fn compare_and_swap(
        &self,
        expected_revision: u64,
        bytes: &[u8],
    ) -> Result<u64, CodexAuthError> {
        let mut record = self.0.lock().unwrap();
        if record.revision != expected_revision {
            return Err(CodexAuthError::Storage("synthetic conflict".into()));
        }
        record.revision += 1;
        record.bytes = zeroize::Zeroizing::new(bytes.to_vec());
        Ok(record.revision)
    }
}

fn shared_store() -> (Arc<MemoryStorage>, CodexAuthStore) {
    let directory = codex_auth_tempdir();
    let path = directory.path().join("private").join("auth.json");
    write_test_codex_auth(&path, 1);
    let binding = "codex/project/account";
    let bytes = CodexAuthStore::at_path(path)
        .export_shared_record(binding)
        .unwrap();
    let storage = Arc::new(MemoryStorage(std::sync::Mutex::new(CodexAuthRecord {
        revision: 1,
        bytes,
    })));
    let store = CodexAuthStore::shared(storage.clone(), binding.into()).unwrap();
    (storage, store)
}

#[tokio::test]
async fn missing_refresh_permit_does_not_reserve_shared_rotating_token_authority() {
    let (storage, store) = shared_store();
    let mut profile = ProviderProfile::new(
        "codex",
        ProviderKind::OpenAiCodex,
        None,
        Some(CODEX_CREDENTIAL_REFERENCE.into()),
        5000,
    )
    .unwrap();
    profile.base_url = Some("http://127.0.0.1:9/v1".into());
    let origin = profile.network_origin().unwrap().unwrap();
    let executor = ProviderExecutor::new(profile.clone()).with_codex_auth_store(store.clone());
    let policy = BuiltInPolicy::offline_default()
        .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
        .with_network_destination(origin);
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [48; 32],
    );
    let error = gateway
        .execute(provider_request(&profile), &executor)
        .await
        .unwrap_err();
    assert!(format!("{error:?}").contains("provider origin is absent from permit obligations"));
    assert_eq!(storage.0.lock().unwrap().revision, 1);
    assert!(
        store.load().is_ok(),
        "preflight rejection cannot poison shared custody"
    );
}

#[tokio::test]
async fn interrupted_shared_refresh_never_replays_http_on_another_executor() {
    let (storage, store) = shared_store();
    let expected = store.load().unwrap();
    drop(store.begin_refresh(&expected).unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut profile = ProviderProfile::new(
        "codex",
        ProviderKind::OpenAiCodex,
        None,
        Some(CODEX_CREDENTIAL_REFERENCE.into()),
        5000,
    )
    .unwrap();
    profile.base_url = Some(format!("http://{}/v1", listener.local_addr().unwrap()));
    let origin = profile.network_origin().unwrap().unwrap();
    let executor = ProviderExecutor::new(profile.clone()).with_codex_auth_store(
        CodexAuthStore::shared(storage.clone(), "codex/project/account".into()).unwrap(),
    );
    let policy = BuiltInPolicy::offline_default()
        .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
        .with_network_destination(origin)
        .with_network_destination(CODEX_AUTH_ORIGIN);
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [49; 32],
    );
    for _ in 0..2 {
        let error = gateway
            .execute(provider_request(&profile), &executor)
            .await
            .unwrap_err();
        assert!(format!("{error:?}").contains("pending or interrupted"));
    }
    assert_eq!(storage.0.lock().unwrap().revision, 2);
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err(),
        "no model or refresh request may follow ambiguous shared custody"
    );
}
