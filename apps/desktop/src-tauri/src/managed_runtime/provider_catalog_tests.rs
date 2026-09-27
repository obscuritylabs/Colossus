//! Native acceptance: real verified sidecar, authenticated IPC, and provider gateway.

use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    time::{Duration, Instant},
};

use super::*;

const MODEL_CARD: &str = r#"{"data":[{"id":"fixture-model","name":"Fixture model","context_length":32768,"top_provider":{"max_completion_tokens":4096},"supported_parameters":["tools"]}]}"#;
const RETRY_CARD: &str = r#"{"data":[{"id":"fixture-model-retry"}]}"#;
const ROOT_PREFIX: &str = "ColossusCatalogTest-";
const RUNTIME_KEY_SERVICE: &str = "com.obscuritylabs.colossus.managed-runtime";

struct CatalogTestHome {
    path: PathBuf,
    parent: PathBuf,
    runtime_instance: Option<uuid::Uuid>,
    #[cfg(not(windows))]
    _temporary_parent: tempfile::TempDir,
}

impl CatalogTestHome {
    fn new() -> Self {
        #[cfg(windows)]
        let parent = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .expect("absolute LocalAppData test parent");
        #[cfg(not(windows))]
        let temporary_parent = tempfile::tempdir().expect("test parent");
        #[cfg(not(windows))]
        let parent = temporary_parent.path().to_owned();
        let parent = std::fs::canonicalize(parent).expect("canonical test parent");
        Self {
            path: parent.join(format!("{ROOT_PREFIX}{}", uuid::Uuid::now_v7())),
            parent,
            runtime_instance: None,
            #[cfg(not(windows))]
            _temporary_parent: temporary_parent,
        }
    }

    fn open_store(&mut self) -> SettingsStore {
        let store = SettingsStore::open_test_home(self.path.clone()).expect("isolated test home");
        let storage = store.self_test_storage().expect("isolated runtime storage");
        self.runtime_instance = Some(
            self_test_instance_id(&storage.instance_dir)
                .expect("isolated runtime identity")
                .as_uuid(),
        );
        store
    }

    fn cleanup(&self) {
        assert_eq!(self.path.parent(), Some(self.parent.as_path()));
        assert!(
            self.path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(ROOT_PREFIX))
        );
        if let Some(instance) = self.runtime_instance {
            // These are the exact accounts of this generated runtime, never a
            // service-wide search or keys from a user-selected Desktop home.
            for account in [
                format!("journal-key:journal-{instance}"),
                format!("signing-key:checkpoint-{instance}"),
                format!("journal-anchor:journal-{instance}"),
            ] {
                let entry = keyring::Entry::new(RUNTIME_KEY_SERVICE, &account)
                    .expect("isolated runtime key");
                if let Err(error) = entry.delete_credential() {
                    assert!(
                        matches!(error, keyring::Error::NoEntry),
                        "cannot remove isolated catalog runtime key"
                    );
                }
                assert!(
                    matches!(entry.get_secret(), Err(keyring::Error::NoEntry)),
                    "isolated catalog runtime key must be absent after cleanup"
                );
            }
        }
        if self.path.exists() {
            assert_eq!(
                std::fs::canonicalize(&self.path).expect("canonical test root"),
                self.path
            );
            #[cfg(unix)]
            crate::managed_runtime::test_directory_cleanup::prepare_removal(&self.path)
                .expect("prepare generated catalog directories");
            std::fs::remove_dir_all(&self.path).expect("remove generated catalog test home");
        }
    }
}

impl Drop for CatalogTestHome {
    fn drop(&mut self) {
        if let Err(panic) = std::panic::catch_unwind(|| self.cleanup()) {
            if std::thread::panicking() {
                eprintln!("catalog cleanup incomplete; generated test resources retained");
            } else {
                std::panic::resume_unwind(panic);
            }
        }
    }
}

struct CatalogResponse {
    status: u16,
    body: &'static str,
    bearer: Option<&'static str>,
}

fn catalog_fixture(
    responses: Vec<CatalogResponse>,
) -> (ProviderSetting, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
    listener.set_nonblocking(true).expect("bounded listener");
    let provider = ProviderSetting {
        profile: "setup-provider".into(),
        kind: ProviderKindSetting::Compatible,
        base_url: format!("http://{}/v1", listener.local_addr().expect("address")),
        credential_id: None,
        timeout_ms: None,
    };
    let server = std::thread::spawn(move || {
        for CatalogResponse {
            status,
            body,
            bearer,
        } in responses
        {
            let deadline = Instant::now() + Duration::from_secs(45);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "catalog request deadline");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                }
            };
            // Winsock can carry the listener's nonblocking mode onto accepted
            // streams. Read headers with the bounded blocking timeout below.
            stream
                .set_nonblocking(false)
                .expect("blocking fixture stream");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("read deadline");
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).expect("request header");
                request.push(byte[0]);
                assert!(request.len() <= 16 * 1024, "bounded request headers");
            }
            assert!(request.starts_with(b"GET /v1/models HTTP/1.1\r\n"));
            let request = String::from_utf8_lossy(&request);
            let authorization = request.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("authorization")
                    .then(|| value.trim())
            });
            assert_eq!(
                authorization.map(str::to_owned),
                bearer.map(|value| format!("Bearer {value}"))
            );
            write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).expect("fixture response");
        }
    });
    (provider, server)
}

#[test]
#[ignore = "requires prepared Desktop sidecar binaries, platform storage, and loopback access"]
fn native_catalog_first_setup_loads_cards_and_recovers_after_provider_failure() {
    let mut home = CatalogTestHome::new();
    let store = home.open_store();
    let state = AppState::default();
    let settings = DesktopSettings::default();
    assert!(!settings.managed_configured());
    let (mut provider, server) = catalog_fixture(
        [MODEL_CARD, "invalid JSON", RETRY_CARD]
            .into_iter()
            .map(|body| CatalogResponse {
                status: 200,
                body,
                bearer: None,
            })
            .collect(),
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("native runtime");
    runtime.block_on(async {
        let models = discover_provider_models(&state, &store, &settings, &provider)
            .await
            .expect("first-setup catalog");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "fixture-model");
        assert_eq!(models[0].context_window_tokens, Some(32_768));
        assert_eq!(models[0].max_output_tokens, Some(4_096));
        assert!(
            discover_provider_models(&state, &store, &settings, &provider)
                .await
                .is_err()
        );
        provider.kind = ProviderKindSetting::Responses;
        let retry = discover_provider_models(&state, &store, &settings, &provider)
            .await
            .expect("retry after malformed catalog");
        assert_eq!(retry[0].id, "fixture-model-retry");
    });
    server.join().expect("catalog fixture completed");
    assert!(!store.application_root().join("settings.json").exists());
    drop(state);
    drop(store);
    let generated_path = home.path.clone();
    drop(home);
    assert!(
        !generated_path.exists(),
        "catalog test home must be removed"
    );
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
#[ignore = "requires prepared Desktop sidecar binaries, native key store, and loopback access"]
fn native_catalog_forwards_vault_credentials_and_recovers_after_unauthorized() {
    use std::sync::Arc;

    const TEST_KEY: &str = "catalog-disposable-fixture-key";
    let mut home = CatalogTestHome::new();
    let store = home.open_store();
    let state = AppState::default();
    let keys = Arc::new(NativeTestKeyStore::default());
    let vault = DesktopCredentials::with_test_key_store(&store, keys.clone());
    *state.credential_vault.lock().expect("test vault slot") = Some(vault.clone());
    let settings = DesktopSettings::default();
    let (mut provider, server) = catalog_fixture(
        [401, 200]
            .into_iter()
            .map(|status| CatalogResponse {
                status,
                body: MODEL_CARD,
                bearer: Some(TEST_KEY),
            })
            .collect(),
    );
    provider.credential_id = Some("fixture-provider-key".into());
    provider.kind = ProviderKindSetting::Responses;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("native runtime");
    runtime.block_on(async {
        vault
            .write(
                "fixture-provider-key",
                colossus_contracts::HostSecret::new(TEST_KEY.to_owned()).expect("test secret"),
            )
            .await
            .expect("encrypted native vault write");
        assert!(
            discover_provider_models(&state, &store, &settings, &provider)
                .await
                .is_err()
        );
        let models = discover_provider_models(&state, &store, &settings, &provider)
            .await
            .expect("retry with saved credential");
        assert_eq!(models[0].id, "fixture-model");
        vault
            .delete("fixture-provider-key")
            .await
            .expect("delete fixture credential");
    });
    server.join().expect("catalog fixture completed");
    drop(vault);
    drop(state);
    drop(keys);
    drop(store);
    let generated_path = home.path.clone();
    drop(home);
    assert!(
        !generated_path.exists(),
        "catalog test home must be removed"
    );
}

#[cfg(any(windows, target_os = "macos"))]
#[derive(Default)]
struct NativeTestKeyStore {
    accounts: std::sync::Mutex<Vec<String>>,
}

#[cfg(any(windows, target_os = "macos"))]
impl colossus_credentials::PlatformKeyStore for NativeTestKeyStore {
    fn read(
        &self,
        account: &str,
    ) -> Result<Option<zeroize::Zeroizing<Vec<u8>>>, colossus_contracts::CredentialError> {
        colossus_credentials::SystemKeyStore.read(account)
    }

    fn write(
        &self,
        account: &str,
        envelope: &[u8],
    ) -> Result<(), colossus_contracts::CredentialError> {
        self.accounts
            .lock()
            .expect("test key accounts")
            .push(account.into());
        colossus_credentials::SystemKeyStore.write(account, envelope)
    }
}

#[cfg(any(windows, target_os = "macos"))]
impl Drop for NativeTestKeyStore {
    fn drop(&mut self) {
        use keyring_core::api::CredentialStoreApi as _;

        let mut accounts = self.accounts.lock().expect("test key accounts");
        accounts.sort();
        accounts.dedup();
        for account in accounts.iter() {
            #[cfg(windows)]
            let entry = windows_native_keyring_store::Store::new()
                .expect("native store")
                .build(
                    "com.obscuritylabs.colossus.credentials.v1",
                    account,
                    Some(&std::collections::HashMap::from([("persistence", "Local")])),
                )
                .expect("isolated test key");
            #[cfg(target_os = "macos")]
            let entry = apple_native_keyring_store::keychain::Store::new()
                .expect("native store")
                .build("com.obscuritylabs.colossus.credentials.v1", account, None)
                .expect("isolated test key");
            if let Err(error) = entry.delete_credential() {
                assert!(
                    matches!(error, keyring_core::Error::NoEntry),
                    "cannot remove isolated catalog test key"
                );
            }
        }
    }
}
