//! The project projection reaches the real OAuth HTTP client, including token exchange.
use super::*;
use axum::{
    Json, Router,
    http::StatusCode,
    routing::{get, post},
};
use colossus_contracts::VaultRecord;
use colossus_ports::{CredentialError, CredentialKey, CredentialVault};
use colossus_runtime::{Runtime, RuntimeOpenOptions};
use serde_json::json;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct Vault(Mutex<BTreeMap<CredentialKey, VaultRecord>>);
impl CredentialVault for Vault {
    fn read(&self, key: &CredentialKey) -> Result<Option<VaultRecord>, CredentialError> {
        self.0
            .lock()
            .unwrap()
            .get(key)
            .map(|record| VaultRecord::new(record.expose().to_vec()))
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
}

#[tokio::test]
async fn restricted_oauth_checks_metadata_and_token_origins_before_network_use() {
    tokio::time::timeout(std::time::Duration::from_secs(15), oauth_policy())
        .await
        .unwrap();
}

async fn oauth_policy() {
    let mcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mcp_origin = format!("http://{}", mcp_listener.local_addr().unwrap());
    let issuer_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer_origin = format!("http://{}", issuer_listener.local_addr().unwrap());
    let token_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let token_origin = format!("http://{}", token_listener.local_addr().unwrap());
    let metadata_requests = Arc::new(AtomicUsize::new(0));
    let token_requests = Arc::new(AtomicUsize::new(0));
    let resource = format!("{mcp_origin}/mcp");
    let challenge = format!(
        "Bearer resource_metadata=\"{mcp_origin}/.well-known/oauth-protected-resource/mcp\""
    );
    let metadata = json!({"resource":resource,"authorization_servers":[issuer_origin],"scopes_supported":["read"]});
    let mcp = Router::new()
        .route(
            "/mcp",
            get(move || {
                let challenge = challenge.clone();
                async move {
                    (
                        StatusCode::UNAUTHORIZED,
                        [("www-authenticate", challenge)],
                        Json(json!({})),
                    )
                }
            }),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(move || {
                let metadata = metadata.clone();
                async move { Json(metadata) }
            }),
        );
    let issuer = {
        let requests = metadata_requests.clone();
        let metadata = json!({"issuer":issuer_origin,"authorization_endpoint":format!("{issuer_origin}/authorize"),"token_endpoint":format!("{token_origin}/token"),"response_types_supported":["code"],"code_challenge_methods_supported":["S256"]});
        Router::new().route(
            "/.well-known/oauth-authorization-server",
            get(move || {
                let requests = requests.clone();
                let metadata = metadata.clone();
                async move {
                    requests.fetch_add(1, Ordering::SeqCst);
                    Json(metadata)
                }
            }),
        )
    };
    let token = {
        let requests = token_requests.clone();
        Router::new().route("/token", post(move || { let requests = requests.clone(); async move { requests.fetch_add(1, Ordering::SeqCst); Json(json!({"access_token":"synthetic-test-token","token_type":"Bearer","expires_in":3600,"scope":"read"})) } }))
    };
    let servers = [
        tokio::spawn(async move {
            axum::serve(mcp_listener, mcp).await.unwrap();
        }),
        tokio::spawn(async move {
            axum::serve(issuer_listener, issuer).await.unwrap();
        }),
        tokio::spawn(async move {
            axum::serve(token_listener, token).await.unwrap();
        }),
    ];
    let mut full = ChatProjectConfig::new(
        colossus_runtime::RuntimeConfig::controlled_chat_template("/unused/oauth.redb"),
    );
    full.runtime.mcp.servers.insert("oauth".into(), serde_json::from_value(json!({"transport":"streamable_http","url":resource,"allowedTools":["read"],"oauth":{"clientId":"fixture","callbackPort":8787,"scopes":["read"]}})).unwrap());
    full.runtime.mcp.oauth_credential_store = colossus_mcp::McpOAuthCredentialStoreKind::Platform;
    // A retained operator auth origin cannot bypass a project restriction.
    full.runtime.sandbox.network_destinations = vec![issuer_origin.clone(), token_origin.clone()];
    full.connection_network_policy =
        serde_json::from_value(json!({"mode":"allow_list","allowed_origins":[mcp_origin]}))
            .unwrap();
    for step in 0..3 {
        if step == 1 {
            full.connection_network_policy
                .allowed_origins
                .insert(issuer_origin.clone());
        }
        if step == 2 {
            full.connection_network_policy
                .allowed_origins
                .insert(token_origin.clone());
        }
        let workspace = tempfile::tempdir().unwrap();
        let mut config = execution(&full).unwrap().runtime;
        config.use_ephemeral_storage();
        let vault = Arc::new(Vault::default());
        let runtime = Runtime::open_with_options(
            &config,
            Arc::new(colossus_policy::DenyApproval),
            None,
            RuntimeOpenOptions::for_controlled_chat(workspace.path())
                .unwrap()
                .with_oauth_credential_vault(vault.clone()),
        )
        .unwrap();
        let callback = "https://control.example/auth/mcp/callback";
        let login = runtime
            .mcp_oauth_browser_login_begin("oauth", "flow", "caller-binding", callback)
            .await;
        if step == 0 {
            assert!(login.is_err());
            assert_eq!(metadata_requests.load(Ordering::SeqCst), 0);
            assert_eq!(token_requests.load(Ordering::SeqCst), 0);
            continue;
        }
        let login = login.unwrap();
        let authorization = url::Url::parse(&login.authorization_url).unwrap();
        let state = authorization
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .into_owned();
        let mut redirect = url::Url::parse(callback).unwrap();
        redirect
            .query_pairs_mut()
            .append_pair("code", "synthetic-code")
            .append_pair("state", &state);
        let completed = runtime
            .mcp_oauth_browser_login_complete("oauth", "flow", "caller-binding", redirect.as_str())
            .await;
        if step == 1 {
            assert!(completed.is_err());
            assert_eq!(token_requests.load(Ordering::SeqCst), 0);
        } else {
            assert!(completed.unwrap().authenticated);
            assert_eq!(token_requests.load(Ordering::SeqCst), 1);
            assert!(!vault.0.lock().unwrap().is_empty());
        }
    }
    for server in servers {
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }
}
