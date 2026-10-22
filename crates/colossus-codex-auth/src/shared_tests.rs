use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};

struct MemoryStorage {
    record: Mutex<CodexAuthRecord>,
    authorized: AtomicBool,
}

impl CodexAuthStorage for MemoryStorage {
    fn read(&self) -> Result<CodexAuthRecord, CodexAuthError> {
        let record = self.record.lock().unwrap();
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
        let mut record = self.record.lock().unwrap();
        if !self.authorized.load(Ordering::SeqCst) || record.revision != expected_revision {
            return Err(credentials_changed());
        }
        record.revision += 1;
        record.bytes = Zeroizing::new(bytes.to_vec());
        Ok(record.revision)
    }
}

fn jwt(value: Value) -> String {
    format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap())
    )
}

fn auth(account: &str, refresh: &str) -> StoredAuth {
    serde_json::from_value(json!({
        "auth_mode": "chatgpt",
        "tokens": {
            "id_token": jwt(json!({"https://api.openai.com/auth":{"chatgpt_account_id":account}})),
            "access_token": jwt(json!({"exp":1})),
            "refresh_token": refresh,
        },
        "metadata": {"retained":true},
    }))
    .unwrap()
}

fn response(account: &str, refresh: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id_token":jwt(json!({"https://api.openai.com/auth":{"chatgpt_account_id":account}})),
        "access_token":jwt(json!({"exp":OffsetDateTime::now_utc().unix_timestamp()+3600})),
        "refresh_token":refresh,
    }))
    .unwrap()
}

fn fixture() -> (Arc<MemoryStorage>, CodexAuthStore) {
    let binding = "codex/project-a/account";
    let storage = Arc::new(MemoryStorage {
        record: Mutex::new(CodexAuthRecord {
            revision: 1,
            bytes: export_document(auth("account-a", "refresh-one"), binding).unwrap(),
        }),
        authorized: AtomicBool::new(true),
    });
    let store = CodexAuthStore::shared(storage.clone(), binding.into()).unwrap();
    (storage, store)
}

#[test]
fn independent_replicas_claim_one_refresh_before_any_http_exchange() {
    let (storage, first) = fixture();
    let second = CodexAuthStore::shared(storage.clone(), "codex/project-a/account".into()).unwrap();
    let first_auth = first.load().unwrap();
    let second_auth = second.load().unwrap();
    let outcomes = std::thread::scope(|scope| {
        let left = scope.spawn(|| first.begin_refresh(&first_auth));
        let right = scope.spawn(|| second.begin_refresh(&second_auth));
        (left.join().unwrap(), right.join().unwrap())
    });
    let (store, expected, claim) = match outcomes {
        (Ok(claim), Err(_)) => (first, first_auth, claim),
        (Err(_), Ok(claim)) => (second, second_auth, claim),
        _ => panic!("exactly one replica must obtain pre-exchange authority"),
    };
    assert!(
        store.load().is_err(),
        "pending custody cannot start another exchange"
    );
    let refreshed = store
        .apply_claimed_refresh(&expected, claim, &response("account-a", "refresh-two"))
        .unwrap();
    assert_eq!(refreshed.refresh_token(), "refresh-two");
    assert!(!refreshed.requires_refresh(OffsetDateTime::now_utc()));
    assert_eq!(storage.record.lock().unwrap().revision, 3);
    assert!(
        store.begin_refresh(&expected).is_err(),
        "old snapshots cannot claim again"
    );
    assert_eq!(store.load().unwrap().refresh_token(), "refresh-two");
}

#[test]
fn interrupted_or_ambiguous_refresh_remains_pending_across_reconstruction() {
    let (storage, store) = fixture();
    let expected = store.load().unwrap();
    let claim = store.begin_refresh(&expected).unwrap();
    drop(claim); // Includes cancellation before a response or after an ambiguous HTTP result.
    let restarted =
        CodexAuthStore::shared(storage.clone(), "codex/project-a/account".into()).unwrap();
    assert!(restarted.load().is_err());
    assert!(restarted.begin_refresh(&expected).is_err());
    assert!(
        restarted
            .export_shared_record("codex/project-a/account")
            .is_err()
    );
    assert_eq!(storage.record.lock().unwrap().revision, 2);
}

#[test]
fn wrong_scope_and_cross_account_refresh_never_replace_a_selected_authorization() {
    let (storage, store) = fixture();
    let wrong = CodexAuthStore::shared(storage.clone(), "codex/project-b/account".into()).unwrap();
    assert!(
        wrong.load().is_err(),
        "envelopes retain exact project/purpose binding"
    );
    let expected = store.load().unwrap();
    assert!(wrong.begin_refresh(&expected).is_err());
    assert!(
        store
            .apply_refresh(&expected, &response("account-a", "refresh-two"))
            .is_err(),
        "shared refresh cannot bypass the pre-exchange claim"
    );
    let claim = store.begin_refresh(&expected).unwrap();
    assert!(
        store
            .apply_claimed_refresh(&expected, claim, &response("account-b", "foreign-refresh"))
            .is_err()
    );
    assert_eq!(storage.record.lock().unwrap().revision, 2);
    assert!(store.load().is_err());
}

#[test]
fn losing_runtime_authority_after_exchange_refuses_the_token_commit() {
    let (storage, store) = fixture();
    let expected = store.load().unwrap();
    let claim = store.begin_refresh(&expected).unwrap();
    storage.authorized.store(false, Ordering::SeqCst);
    assert!(
        store
            .apply_claimed_refresh(&expected, claim, &response("account-a", "refresh-two"))
            .is_err()
    );
    assert_eq!(storage.record.lock().unwrap().revision, 2);
    storage.authorized.store(true, Ordering::SeqCst);
    assert!(
        store.load().is_err(),
        "new lease cannot replay an interrupted rotating token"
    );
}

#[test]
fn deliberate_reauthentication_supersedes_an_old_refresh_claim() {
    let (storage, store) = fixture();
    let expected = store.load().unwrap();
    let claim = store.begin_refresh(&expected).unwrap();
    let new = export_document(auth("account-a", "fresh-login"), "codex/project-a/account").unwrap();
    storage.compare_and_swap(2, &new).unwrap();
    assert!(
        store
            .apply_claimed_refresh(&expected, claim, &response("account-a", "refresh-two"))
            .is_err()
    );
    assert_eq!(store.load().unwrap().refresh_token(), "fresh-login");
}

#[test]
fn corrupt_missing_oversized_or_inconsistent_records_are_unavailable() {
    let (storage, store) = fixture();
    for bytes in [
        b"{}".to_vec(),
        b"not-json".to_vec(),
        vec![0; MAX_SHARED_BYTES + 1],
    ] {
        storage.record.lock().unwrap().bytes = Zeroizing::new(bytes);
        assert!(store.load().is_err());
    }
    storage.record.lock().unwrap().revision = 0;
    assert!(store.load().is_err());
    let mut inconsistent = auth("account-a", "refresh-one");
    inconsistent.tokens.as_mut().unwrap().account_id = Some("account-b".into());
    assert!(export_document(inconsistent, "codex/project-a/account").is_err());
    assert!(CodexAuthStore::shared(storage, "bad binding".into()).is_err());
}
