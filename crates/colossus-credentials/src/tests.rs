mod support;

use crate::{
    PlatformCredentialVault, PlatformKeyStore, SystemKeyStore,
    database::{DATABASE_FILE, METADATA, Metadata, RECORDS},
    vault::record_id,
};
use colossus_contracts::{CredentialError, MAX_VAULT_RECORD_BYTES, VaultRecord};
use colossus_ports::{CredentialKey, CredentialVault};
use redb::{ReadableDatabase as _, ReadableTable as _};
use std::{path::Path, sync::Arc};
use support::{Fault, Fixture, MemoryKeys};
use zeroize::Zeroizing;

fn key() -> CredentialKey {
    CredentialKey::new("manual-token", "test-token").unwrap()
}
fn record(size: usize) -> VaultRecord {
    VaultRecord::new((0..size).map(|n| (n % 251) as u8).collect::<Vec<_>>()).unwrap()
}

#[test]
fn master_key_envelope_is_bounded_canonical_base64url_without_plaintext_reallocation() {
    let master = Zeroizing::new([255_u8; 32]);
    let vault_id = "1".repeat(32);
    let key_id = "2".repeat(32);
    let encoded = crate::crypto::encode_key(&vault_id, &key_id, &master).unwrap();
    assert!(encoded.len() <= crate::platform::MAX_KEY_ENVELOPE_BYTES);
    assert_eq!(encoded.capacity(), crate::platform::MAX_KEY_ENVELOPE_BYTES);
    assert!(
        crate::crypto::decode_key(&encoded, &vault_id, &key_id)
            .unwrap()
            .as_ref()
            == master.as_ref()
    );
    let text = std::str::from_utf8(&encoded).unwrap();
    assert!(!text.contains(['+', '/', '=']));
    // Both padding and escaped JSON spellings are intentionally unsupported.
    let padded = text.replace("\"key\":\"", "\"key\":\"=");
    let escaped = text.replace("\"key\":\"_", "\"key\":\"\\u005f");
    for invalid in [padded.as_bytes(), escaped.as_bytes()] {
        assert!(matches!(
            crate::crypto::decode_key(invalid, &vault_id, &key_id),
            Err(CredentialError::Corrupt)
        ));
    }
    assert!(matches!(
        crate::crypto::encode_key(&"x".repeat(257), &key_id, &master),
        Err(CredentialError::Oversized)
    ));
}

#[test]
fn absent_reads_contains_and_delete_do_not_create_files_or_platform_entries() {
    let fixture = Fixture::new();
    let keys = Arc::new(MemoryKeys::default());
    let vault = fixture.vault(keys.clone());
    assert!(vault.read(&key()).unwrap().is_none());
    assert!(!vault.contains(&key()).unwrap());
    vault.delete(&key()).unwrap();
    assert_eq!(std::fs::read_dir(fixture.root.path()).unwrap().count(), 0);
    let state = keys.state.lock().unwrap();
    assert_eq!((state.reads, state.writes), (0, 0));
}

#[test]
fn binary_records_roundtrip_beyond_platform_limits_and_survive_reopen() {
    let fixture = Fixture::new();
    let keys = Arc::new(MemoryKeys::default());
    for length in [1, 761, 762, 2560, 2561, 8192, 65536, MAX_VAULT_RECORD_BYTES] {
        let expected = record(length);
        {
            let vault = fixture.vault(keys.clone());
            vault.write(&key(), &expected).unwrap();
            assert!(vault.contains(&key()).unwrap());
        }
        let vault = fixture.vault(keys.clone());
        let actual = vault.read(&key()).unwrap().unwrap();
        assert!(
            actual.expose() == expected.expose(),
            "roundtrip bytes differ at length {length}"
        );
    }
    let state = keys.state.lock().unwrap();
    assert_eq!(state.writes, 1);
    let envelope = state.values.values().next().unwrap();
    assert!(envelope.is_ascii());
    assert!(envelope.len() <= crate::platform::MAX_KEY_ENVELOPE_BYTES);
}

#[test]
fn replacing_and_deleting_records_preserves_separate_purposes() {
    let fixture = Fixture::new();
    let vault = fixture.vault(Arc::new(MemoryKeys::default()));
    let oauth = CredentialKey::new("mcp-oauth", key().id()).unwrap();
    vault.write(&key(), &record(8192)).unwrap();
    vault.write(&oauth, &record(65536)).unwrap();
    vault.write(&key(), &record(762)).unwrap();
    assert_eq!(vault.read(&key()).unwrap().unwrap().expose().len(), 762);
    vault.delete(&key()).unwrap();
    vault.delete(&key()).unwrap();
    assert!(!vault.contains(&key()).unwrap());
    assert_eq!(vault.read(&oauth).unwrap().unwrap().expose().len(), 65536);
}

#[test]
fn lease_prevents_competing_instances_and_releases_when_owner_drops() {
    let fixture = Fixture::new();
    let keys = Arc::new(MemoryKeys::default());
    let first = fixture.vault(keys.clone());
    let second = fixture.vault(keys);
    first.write(&key(), &record(1)).unwrap();
    assert_eq!(second.read(&key()).unwrap_err(), CredentialError::Busy);
    assert_eq!(
        second.write(&key(), &record(2)).unwrap_err(),
        CredentialError::Busy
    );
    drop(first);
    assert!(second.contains(&key()).unwrap());
}

#[test]
fn lease_prevents_a_second_process_from_accessing_the_same_vault() {
    let fixture = Fixture::new();
    let vault = fixture.vault(Arc::new(MemoryKeys::default()));
    vault.write(&key(), &record(8192)).unwrap();
    run_child(&fixture, "busy");
}

#[test]
fn verified_master_key_lives_only_in_its_open_vault_instance() {
    let fixture = Fixture::new();
    let keys = Arc::new(MemoryKeys::default());
    {
        let vault = fixture.vault(keys.clone());
        vault.write(&key(), &record(8192)).unwrap();
        assert_eq!(keys.state.lock().unwrap().reads, 2);
        for _ in 0..3 {
            assert!(vault.contains(&key()).unwrap());
        }
        assert_eq!(keys.state.lock().unwrap().reads, 2);
    }
    let vault = fixture.vault(keys.clone());
    assert!(vault.contains(&key()).unwrap());
    assert_eq!(keys.state.lock().unwrap().reads, 3);
}

#[test]
fn one_shared_instance_serializes_writes_without_losing_records() {
    let fixture = Fixture::new();
    let vault = Arc::new(fixture.vault(Arc::new(MemoryKeys::default())));
    let threads = (0..8)
        .map(|index| {
            let vault = vault.clone();
            std::thread::spawn(move || {
                let key = CredentialKey::new("manual", &index.to_string()).unwrap();
                vault.write(&key, &record(8192 + index)).unwrap();
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
    for index in 0..8 {
        let key = CredentialKey::new("manual", &index.to_string()).unwrap();
        assert_eq!(
            vault.read(&key).unwrap().unwrap().expose().len(),
            8192 + index
        );
    }
}

#[test]
fn initialized_vault_never_replaces_a_missing_platform_key() {
    let fixture = Fixture::new();
    let keys = Arc::new(MemoryKeys::default());
    let vault = fixture.vault(keys.clone());
    vault.write(&key(), &record(8192)).unwrap();
    drop(vault);
    keys.state.lock().unwrap().values.clear();
    let vault = fixture.vault(keys.clone());
    assert_eq!(vault.read(&key()).unwrap_err(), CredentialError::MissingKey);
    assert_eq!(
        vault.write(&key(), &record(2)).unwrap_err(),
        CredentialError::MissingKey
    );
    assert_eq!(
        vault.delete(&key()).unwrap_err(),
        CredentialError::MissingKey
    );
    assert_eq!(keys.state.lock().unwrap().writes, 1);
}

#[test]
fn platform_errors_are_typed_and_no_replacement_or_plaintext_fallback_occurs() {
    let fixture = Fixture::new();
    let keys = Arc::new(MemoryKeys::default());
    let vault = fixture.vault(keys.clone());
    vault.write(&key(), &record(8192)).unwrap();
    drop(vault);
    for error in [
        CredentialError::Locked,
        CredentialError::Unavailable,
        CredentialError::Cancelled,
    ] {
        let vault = fixture.vault(keys.clone());
        keys.state.lock().unwrap().fault = Some(Fault::Read(error));
        assert_eq!(vault.read(&key()).unwrap_err(), error);
        keys.state.lock().unwrap().fault = Some(Fault::Read(error));
        assert_eq!(vault.write(&key(), &record(2)).unwrap_err(), error);
    }
    assert_eq!(keys.state.lock().unwrap().writes, 1);
    assert_eq!(
        fixture
            .vault(keys)
            .read(&key())
            .unwrap()
            .unwrap()
            .expose()
            .len(),
        8192
    );
}

#[test]
fn interrupted_first_key_creation_resumes_without_overwriting_an_existing_key() {
    for (fault, expected_writes) in [
        (Fault::WriteBefore(CredentialError::Unavailable), 2),
        (Fault::WriteAfter(CredentialError::Unavailable), 1),
        (Fault::Readback(CredentialError::Locked), 1),
    ] {
        let fixture = Fixture::new();
        let keys = Arc::new(MemoryKeys::default());
        keys.state.lock().unwrap().fault = Some(fault);
        {
            let vault = fixture.vault(keys.clone());
            assert!(vault.write(&key(), &record(8192)).is_err());
            let reads = keys.state.lock().unwrap().reads;
            assert!(vault.read(&key()).unwrap().is_none());
            vault.delete(&key()).unwrap();
            assert_eq!(
                keys.state.lock().unwrap().reads,
                reads,
                "pending read must not initialize or prompt"
            );
        }
        let envelope_before_recovery = keys.state.lock().unwrap().values.values().next().cloned();
        let vault = fixture.vault(keys.clone());
        vault.write(&key(), &record(8192)).unwrap();
        assert_eq!(vault.read(&key()).unwrap().unwrap().expose().len(), 8192);
        let state = keys.state.lock().unwrap();
        assert_eq!(state.writes, expected_writes);
        if let Some(before) = envelope_before_recovery {
            assert!(state.values.values().next().unwrap().as_slice() == before.as_slice());
        }
    }
}

#[test]
fn initialization_recovers_at_every_durable_boundary_without_rekeying_or_partial_records() {
    use crate::vault::InitializationCheckpoint::*;
    for checkpoint in [
        DatabaseOpened,
        PendingCommitted,
        PendingDirectorySynced,
        KeyStored,
        KeyReadBack,
        ReadyCommitted,
        BeforeRecordCommit,
        RecordCommitted,
    ] {
        let fixture = Fixture::new();
        let keys = Arc::new(MemoryKeys::default());
        {
            let vault = fixture.vault(keys.clone());
            vault.fail_at(checkpoint);
            assert_eq!(
                vault.write(&key(), &record(8192)).unwrap_err(),
                CredentialError::Io
            );
        }
        let writes_before_recovery = keys.state.lock().unwrap().writes;
        {
            let vault = fixture.vault(keys.clone());
            match checkpoint {
                DatabaseOpened => {
                    assert_eq!(vault.read(&key()).unwrap_err(), CredentialError::Corrupt)
                }
                RecordCommitted => {
                    assert!(vault.read(&key()).unwrap().unwrap().expose() == record(8192).expose())
                }
                _ => assert!(vault.read(&key()).unwrap().is_none()),
            }
            assert_eq!(
                keys.state.lock().unwrap().writes,
                writes_before_recovery,
                "read initialized the key at {checkpoint:?}"
            );
            vault.write(&key(), &record(8192)).unwrap();
            assert!(vault.read(&key()).unwrap().unwrap().expose() == record(8192).expose());
        }
        let state = keys.state.lock().unwrap();
        assert_eq!(state.writes, 1, "key was overwritten at {checkpoint:?}");
        assert_eq!(state.values.len(), 1);
    }
}

#[test]
fn corrupt_or_foreign_platform_envelopes_fail_closed() {
    let fixture = Fixture::new();
    let keys = Arc::new(MemoryKeys::default());
    let vault = fixture.vault(keys.clone());
    vault.write(&key(), &record(8192)).unwrap();
    drop(vault);
    let (account, original) = keys
        .state
        .lock()
        .unwrap()
        .values
        .iter()
        .next()
        .map(|(a, v)| (a.clone(), v.clone()))
        .unwrap();
    let mut invalid = vec![vec![b'x'; 257], b"not-json".to_vec()];
    for field in ["version", "vault_id", "key_id", "key"] {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        value[field] = if field == "version" {
            2.into()
        } else {
            "invalid".into()
        };
        invalid.push(serde_json::to_vec(&value).unwrap());
    }
    // A valid envelope with a different key must also fail authentication.
    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    use base64::Engine as _;
    value["key"] = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode([7_u8; 32])
        .into();
    invalid.push(serde_json::to_vec(&value).unwrap());
    for invalid in invalid {
        keys.state
            .lock()
            .unwrap()
            .values
            .insert(account.clone(), Zeroizing::new(invalid));
        let vault = fixture.vault(keys.clone());
        assert_eq!(vault.read(&key()).unwrap_err(), CredentialError::Corrupt);
        assert_eq!(
            vault.write(&key(), &record(2)).unwrap_err(),
            CredentialError::Corrupt
        );
    }
    assert_eq!(keys.state.lock().unwrap().writes, 1);
}

#[test]
fn scope_and_vault_identity_prevent_reusing_records_or_keys_across_owners() {
    let first = Fixture::new();
    let second = Fixture::new();
    let keys = Arc::new(MemoryKeys::default());
    {
        let vault = first.vault(keys.clone());
        vault.write(&key(), &record(8192)).unwrap();
    }
    {
        let wrong_scope = first.vault_with_scope(keys.clone(), "another-owner");
        assert_eq!(
            wrong_scope.read(&key()).unwrap_err(),
            CredentialError::Corrupt
        );
    }
    {
        let vault = second.vault(keys.clone());
        vault.write(&key(), &record(8192)).unwrap();
    }
    assert_eq!(keys.state.lock().unwrap().values.len(), 2);
    let encrypted = read_ciphertext(&first, &key());
    write_ciphertext(&second, &key(), &encrypted);
    assert_eq!(
        second.vault(keys).read(&key()).unwrap_err(),
        CredentialError::Corrupt
    );
}

#[test]
fn ciphertext_tamper_oversize_and_record_relocation_are_rejected() {
    let fixture = Fixture::new();
    let keys = Arc::new(MemoryKeys::default());
    {
        fixture
            .vault(keys.clone())
            .write(&key(), &record(8192))
            .unwrap();
    }
    let original = read_ciphertext(&fixture, &key());
    let mut damaged = original.clone();
    damaged[40] ^= 1;
    for invalid in [
        b"invalid".to_vec(),
        damaged,
        vec![0; crate::crypto::MAX_CIPHERTEXT_BYTES + 1],
    ] {
        write_ciphertext(&fixture, &key(), &invalid);
        assert_eq!(
            fixture.vault(keys.clone()).read(&key()).unwrap_err(),
            CredentialError::Corrupt
        );
    }
    let other = CredentialKey::new("manual-token", "another-token").unwrap();
    write_ciphertext(&fixture, &other, &original);
    assert_eq!(
        fixture.vault(keys).read(&other).unwrap_err(),
        CredentialError::Corrupt
    );
}

#[test]
fn database_contains_ciphertext_instead_of_tokens_after_replace_and_delete() {
    let fixture = Fixture::new();
    let token = b"synthetic-secret-marker-38412349".repeat(1024);
    {
        let vault = fixture.vault(Arc::new(MemoryKeys::default()));
        vault
            .write(&key(), &VaultRecord::new(token.clone()).unwrap())
            .unwrap();
        vault.write(&key(), &record(1)).unwrap();
        vault.delete(&key()).unwrap();
    }
    let bytes = std::fs::read(fixture.root.path().join(DATABASE_FILE)).unwrap();
    assert!(!bytes.windows(30).any(|window| window == &token[..30]));
}

#[test]
fn unsafe_database_and_lease_paths_are_rejected_before_platform_access() {
    for file in [DATABASE_FILE, crate::database::LEASE_FILE] {
        let fixture = Fixture::new();
        drop(fixture.root.open_file(Path::new(file)).unwrap());
        std::fs::hard_link(
            fixture.root.path().join(file),
            fixture.root.path().join("alias"),
        )
        .unwrap();
        let keys = Arc::new(MemoryKeys::default());
        let vault = fixture.vault(keys.clone());
        assert_eq!(
            vault.write(&key(), &record(8192)).unwrap_err(),
            CredentialError::Corrupt
        );
        assert_eq!(keys.state.lock().unwrap().reads, 0);
    }
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.root.path().join(DATABASE_FILE)).unwrap();
    let vault = fixture.vault(Arc::new(MemoryKeys::default()));
    assert!(vault.read(&key()).is_err());
}

#[test]
fn cached_vault_rejects_missing_and_replaced_database_and_lease_files() {
    for leaf in [DATABASE_FILE, crate::database::LEASE_FILE] {
        let fixture = Fixture::new();
        let keys = Arc::new(MemoryKeys::default());
        let vault = fixture.vault(keys.clone());
        vault.write(&key(), &record(8192)).unwrap();
        let path = fixture.root.path().join(leaf);
        std::fs::rename(&path, fixture.root.path().join("displaced-file")).unwrap();
        assert_invalidated_vault(&vault);
        // Even a new private regular leaf cannot stand in for the retained object.
        drop(fixture.root.open_file(Path::new(leaf)).unwrap());
        assert_invalidated_vault(&vault);
        if leaf == crate::database::LEASE_FILE {
            let competing = fixture.vault(keys);
            assert_eq!(
                competing.write(&key(), &record(1)).unwrap_err(),
                CredentialError::Busy
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn cached_vault_rejects_unlinked_database_and_lease_files() {
    for leaf in [DATABASE_FILE, crate::database::LEASE_FILE] {
        let fixture = Fixture::new();
        let vault = fixture.vault(Arc::new(MemoryKeys::default()));
        vault.write(&key(), &record(8192)).unwrap();
        std::fs::remove_file(fixture.root.path().join(leaf)).unwrap();
        assert_invalidated_vault(&vault);
    }
}

fn assert_invalidated_vault(vault: &PlatformCredentialVault) {
    assert_eq!(vault.read(&key()).unwrap_err(), CredentialError::Corrupt);
    assert_eq!(
        vault.contains(&key()).unwrap_err(),
        CredentialError::Corrupt
    );
    assert_eq!(
        vault.write(&key(), &record(1)).unwrap_err(),
        CredentialError::Corrupt
    );
    assert_eq!(vault.delete(&key()).unwrap_err(), CredentialError::Corrupt);
}

#[test]
fn malformed_metadata_and_pending_state_with_records_never_reinitialize() {
    for invalid in [b"invalid".to_vec(), vec![b' '; 2049], b"{}".to_vec()] {
        let fixture = Fixture::new();
        let keys = Arc::new(MemoryKeys::default());
        fixture
            .vault(keys.clone())
            .write(&key(), &record(8192))
            .unwrap();
        {
            let database = fixture.database();
            let write = database.begin_write().unwrap();
            write
                .open_table(METADATA)
                .unwrap()
                .insert("state", invalid.as_slice())
                .unwrap();
            write.commit().unwrap();
        }
        let vault = fixture.vault(keys.clone());
        assert_eq!(vault.read(&key()).unwrap_err(), CredentialError::Corrupt);
        assert_eq!(
            vault.write(&key(), &record(2)).unwrap_err(),
            CredentialError::Corrupt
        );
        assert_eq!(keys.state.lock().unwrap().writes, 1);
    }
    let fixture = Fixture::new();
    let keys = Arc::new(MemoryKeys::default());
    fixture
        .vault(keys.clone())
        .write(&key(), &record(8192))
        .unwrap();
    {
        let database = fixture.database();
        let write = database.begin_write().unwrap();
        {
            let mut table = write.open_table(METADATA).unwrap();
            let mut metadata: Metadata =
                serde_json::from_slice(table.get("state").unwrap().unwrap().value()).unwrap();
            metadata.state = crate::database::Initialization::PendingKey;
            metadata.verification = None;
            table
                .insert("state", serde_json::to_vec(&metadata).unwrap().as_slice())
                .unwrap();
        }
        write.commit().unwrap();
    }
    assert_eq!(
        fixture
            .vault(keys.clone())
            .write(&key(), &record(2))
            .unwrap_err(),
        CredentialError::Corrupt
    );
    assert_eq!(keys.state.lock().unwrap().writes, 1);
}

#[test]
fn empty_ready_checkpoint_remains_initialized_without_replacing_its_master_key() {
    let fixture = Fixture::new();
    let keys = Arc::new(MemoryKeys::default());
    {
        let vault = fixture.vault(keys.clone());
        vault.write(&key(), &record(1)).unwrap();
        vault.delete(&key()).unwrap();
    }
    // This is also the durable state after marking Ready but before the first record commit.
    let vault = fixture.vault(keys.clone());
    assert!(vault.read(&key()).unwrap().is_none());
    vault.write(&key(), &record(8192)).unwrap();
    assert_eq!(keys.state.lock().unwrap().writes, 1);
}

fn read_ciphertext(fixture: &Fixture, key: &CredentialKey) -> Vec<u8> {
    let database = fixture.database();
    let read = database.begin_read().unwrap();
    read.open_table(RECORDS)
        .unwrap()
        .get(record_id(key).as_str())
        .unwrap()
        .unwrap()
        .value()
        .to_vec()
}

fn write_ciphertext(fixture: &Fixture, key: &CredentialKey, value: &[u8]) {
    let database = fixture.database();
    let write = database.begin_write().unwrap();
    write
        .open_table(RECORDS)
        .unwrap()
        .insert(record_id(key).as_str(), value)
        .unwrap();
    write.commit().unwrap();
}

/// Real persistent native key storage requires an unlocked interactive OS credential store.
#[test]
#[ignore = "requires an unlocked Windows/macOS/Linux platform credential store; run in the native credential CI lane"]
fn platform_master_key_survives_vault_reopen() {
    let fixture = Fixture::new();
    {
        let vault =
            PlatformCredentialVault::new(fixture.root.clone(), "native-platform-acceptance")
                .unwrap();
        vault.write(&key(), &record(8192)).unwrap();
    }
    let account = {
        let database = fixture.database();
        let read = database.begin_read().unwrap();
        let table = read.open_table(METADATA).unwrap();
        let value = table.get("state").unwrap().unwrap();
        let metadata: Metadata = serde_json::from_slice(value.value()).unwrap();
        metadata.account()
    };
    let cleanup = PlatformKeyCleanup(account);
    run_child(&fixture, "platform-reopen");
    {
        let vault =
            PlatformCredentialVault::new(fixture.root.clone(), "native-platform-acceptance")
                .unwrap();
        assert_eq!(vault.read(&key()).unwrap().unwrap().expose().len(), 65536);
    }
    {
        let vault =
            PlatformCredentialVault::new(fixture.root.clone(), "native-platform-acceptance")
                .unwrap();
        let expected = record(65536);
        assert!(vault.read(&key()).unwrap().unwrap().expose() == expected.expose());
        assert!(SystemKeyStore.read(&cleanup.0).unwrap().unwrap().len() <= 256);
    }
    drop(cleanup);
    assert!(
        fixture
            .root
            .open_existing_file(Path::new(DATABASE_FILE))
            .is_ok()
    );
}

fn run_child(fixture: &Fixture, mode: &str) {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::vault_process_child", "--ignored"])
        .env("COLOSSUS_CREDENTIAL_TEST_ROOT", fixture.root.path())
        .env("COLOSSUS_CREDENTIAL_TEST_MODE", mode)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "credential subprocess failed: {:?}",
                status.code()
            );
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("credential subprocess exceeded its deadline");
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

#[test]
#[ignore = "subprocess helper; invoked by the vault persistence and lease tests"]
fn vault_process_child() {
    let Some(path) = std::env::var_os("COLOSSUS_CREDENTIAL_TEST_ROOT") else {
        return;
    };
    let root = colossus_home::ConfinedRoot::bind(std::path::PathBuf::from(path)).unwrap();
    match std::env::var("COLOSSUS_CREDENTIAL_TEST_MODE")
        .unwrap()
        .as_str()
    {
        "busy" => {
            let vault = PlatformCredentialVault::with_key_store(
                root,
                "test-owner",
                Arc::new(MemoryKeys::default()),
            )
            .unwrap();
            assert_eq!(vault.read(&key()).unwrap_err(), CredentialError::Busy);
        }
        "platform-reopen" => {
            let vault = PlatformCredentialVault::new(root, "native-platform-acceptance").unwrap();
            assert!(vault.read(&key()).unwrap().unwrap().expose() == record(8192).expose());
            vault.write(&key(), &record(65536)).unwrap();
        }
        _ => panic!("unknown credential subprocess mode"),
    }
}

struct PlatformKeyCleanup(String);

impl Drop for PlatformKeyCleanup {
    fn drop(&mut self) {
        crate::platform::delete_test_key(&self.0).expect("remove isolated platform test key");
    }
}
