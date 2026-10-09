use crate::config::Config;
use axum::http::{HeaderMap, HeaderValue};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use colossus_cloud::storage::{
    AuthSession, CloudStore, CloudTransaction, EntityKey, EntityKind, EntityMutation,
};
use colossus_cloud::{CloudCaller, CloudError, CloudResult};
use openidconnect::{
    AccessTokenHash, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet,
    EndpointNotSet, EndpointSet, IssuerUrl, Nonce, OAuth2TokenResponse, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, TokenResponse,
    core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata},
};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use zeroize::Zeroizing;
mod accounts;
mod bootstrap;
mod local;
#[cfg(test)]
mod tests;

type Client = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Flow {
    verifier: String,
    nonce: String,
    expires_at: u64,
}
pub(crate) struct Authentication {
    client: Option<Client>,
    client_secret: Option<ClientSecret>,
    http: reqwest::Client,
    config: Config,
    store: Arc<dyn CloudStore>,
    flow_key: Option<Zeroizing<[u8; 32]>>,
    password_permits: Arc<tokio::sync::Semaphore>,
    dummy_password_hash: String,
}
fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| {
            part.trim()
                .strip_prefix(&format!("{name}="))
                .map(str::to_owned)
        })
}
impl Authentication {
    #[cfg(test)]
    pub async fn new(config: Config) -> Result<Self, &'static str> {
        Self::with_store(
            config,
            Arc::new(colossus_cloud::storage::MemoryCloudStore::default()),
        )
        .await
    }
    pub async fn with_store(
        config: Config,
        store: Arc<dyn CloudStore>,
    ) -> Result<Self, &'static str> {
        let flow_key = config
            .auth_key_variable
            .as_ref()
            .map(|variable| {
                let encoded = Zeroizing::new(
                    std::env::var(variable).map_err(|_| "OIDC flow key unavailable")?,
                );
                let mut key = Zeroizing::new([0u8; 32]);
                hex::decode_to_slice(encoded.trim(), key.as_mut())
                    .map_err(|_| "OIDC flow key invalid")?;
                Ok::<_, &'static str>(key)
            })
            .transpose()?;
        if !config.local_development && config.oidc.is_some() && flow_key.is_none() {
            return Err("production requires encrypted OIDC flows");
        }
        bootstrap::seed_accounts(&store, &config)
            .await
            .map_err(|_| "cloud identity initialization failed")?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| "OIDC HTTP configuration failed")?;
        let (client, secret) = if let Some(oidc) = &config.oidc {
            let metadata = CoreProviderMetadata::discover_async(
                IssuerUrl::new(oidc.issuer.clone()).map_err(|_| "invalid issuer")?,
                &http,
            )
            .await
            .map_err(|_| "OIDC discovery failed")?;
            let secret = oidc
                .client_secret_file
                .as_ref()
                .map(|path| {
                    std::fs::read_to_string(path)
                        .map(Zeroizing::new)
                        .map_err(|_| "cannot load OIDC secret")
                })
                .transpose()?
                .map(|secret| ClientSecret::new(secret.trim().to_owned()));
            let client = CoreClient::from_provider_metadata(
                metadata,
                ClientId::new(oidc.client_id.clone()),
                secret.clone(),
            )
            .set_redirect_uri(
                RedirectUrl::new(format!(
                    "{}/auth/callback",
                    config.public_origin.trim_end_matches('/')
                ))
                .map_err(|_| "invalid callback")?,
            );
            (Some(client), secret)
        } else {
            (None, None)
        };
        let password_permits = Arc::new(tokio::sync::Semaphore::new(4));
        let dummy_password_hash = if config.local_auth.is_some() {
            local::hash_password(
                password_permits.clone(),
                Zeroizing::new(Nonce::new_random().secret().to_owned()),
            )
            .await
            .map_err(|_| "local authentication initialization failed")?
        } else {
            String::new()
        };
        Ok(Self {
            client,
            client_secret: secret,
            http,
            config,
            store,
            flow_key,
            password_permits,
            dummy_password_hash,
        })
    }
    pub async fn login(&self) -> CloudResult<(String, HeaderValue)> {
        let client = self.client.as_ref().ok_or(CloudError::PermissionDenied)?;
        let bucket = (crate::http::now() / 60).to_string();
        self.consume_auth_budget(
            &colossus_cloud::hash_identity(&["oidc-login-global", &bucket]),
            120,
        )
        .await?;
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (url, state, nonce) = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .set_pkce_challenge(challenge)
            .url();
        let id = self.authority_hash(state.secret());
        let flow = Flow {
            verifier: verifier.secret().into(),
            nonce: nonce.secret().into(),
            expires_at: crate::http::now() + 300,
        };
        let value = self.seal_flow(&id, &flow)?;
        self.store
            .commit(CloudTransaction {
                entities: vec![EntityMutation {
                    key: flow_key(&id),
                    expected_revision: 0,
                    value: colossus_cloud::storage::EntityValue::AuthFlow(value),
                    actor: "oidc".into(),
                    operation: "cloud.auth.flow-created.v2".into(),
                }],
                ..Default::default()
            })
            .await?;
        let header = self.set_cookie("colossus_flow", state.secret(), 300)?;
        Ok((url.to_string(), header))
    }
    pub async fn callback(
        &self,
        headers: &HeaderMap,
        state: &str,
        code: String,
    ) -> CloudResult<HeaderValue> {
        if state.len() > 256
            || code.len() > 4096
            || cookie(headers, "colossus_flow").as_deref().map(hash) != Some(hash(state))
        {
            return Err(CloudError::PermissionDenied);
        }
        let key = flow_key(&self.authority_hash(state));
        let record = self
            .store
            .read(&key)
            .await
            .map_err(|_| CloudError::PermissionDenied)?;
        let flow = self.open_flow(&key.id, record.value.auth_flow()?)?;
        if flow.expires_at <= crate::http::now() {
            return Err(CloudError::PermissionDenied);
        }
        self.store
            .delete_entity(&key, record.revision)
            .await
            .map_err(|_| CloudError::PermissionDenied)?;
        let client = self.client.as_ref().ok_or(CloudError::PermissionDenied)?;
        let oidc = self
            .config
            .oidc
            .as_ref()
            .ok_or(CloudError::PermissionDenied)?;
        let token = client
            .exchange_code(AuthorizationCode::new(code))
            .map_err(|_| CloudError::PermissionDenied)?
            .set_pkce_verifier(PkceCodeVerifier::new(flow.verifier))
            .request_async(&self.http)
            .await
            .map_err(|_| CloudError::PermissionDenied)?;
        let id = token.id_token().ok_or(CloudError::PermissionDenied)?;
        // Refresh issuer-owned keys for each new session. An IdP key rotation
        // must neither require a host restart nor keep a retired key trusted.
        let metadata = CoreProviderMetadata::discover_async(
            IssuerUrl::new(oidc.issuer.clone()).map_err(|_| CloudError::PermissionDenied)?,
            &self.http,
        )
        .await
        .map_err(|_| CloudError::PermissionDenied)?;
        let verification_client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(oidc.client_id.clone()),
            self.client_secret.clone(),
        );
        let verifier = verification_client.id_token_verifier();
        let claims = id
            .claims(&verifier, &Nonce::new(flow.nonce))
            .map_err(|_| CloudError::PermissionDenied)?;
        if let Some(expected) = claims.access_token_hash() {
            let actual = AccessTokenHash::from_token(
                token.access_token(),
                id.signing_alg().map_err(|_| CloudError::PermissionDenied)?,
                id.signing_key(&verifier)
                    .map_err(|_| CloudError::PermissionDenied)?,
            )
            .map_err(|_| CloudError::PermissionDenied)?;
            if actual != *expected {
                return Err(CloudError::PermissionDenied);
            }
        }
        let binding_key = colossus_cloud::identity_key(
            EntityKind::OidcIdentity,
            &colossus_cloud::hash_identity(&[&oidc.issuer, claims.subject().as_str()]),
        );
        let binding = colossus_cloud::OidcIdentity::try_from(
            self.store
                .read(&binding_key)
                .await
                .map_err(|_| CloudError::PermissionDenied)?
                .value,
        )
        .map_err(|_| CloudError::PermissionDenied)?;
        if binding.issuer != oidc.issuer || binding.subject != claims.subject().as_str() {
            return Err(CloudError::PermissionDenied);
        }
        let account = self
            .repository()?
            .account(&binding.user_id)
            .await
            .map_err(|_| CloudError::PermissionDenied)?;
        if !account.user.active {
            return Err(CloudError::PermissionDenied);
        }
        let subject = account.user.id;
        let seconds = (claims.expiration() - chrono_now())
            .num_seconds()
            .clamp(0, 8 * 60 * 60) as u64;
        if seconds == 0 {
            return Err(CloudError::PermissionDenied);
        }
        let session = Nonce::new_random().secret().to_owned();
        let now = crate::http::now();
        self.store
            .put_session(AuthSession {
                session_hash: self.authority_hash(&session),
                security_epoch: account.security_epoch,
                subject,
                csrf_hash: String::new(),
                created_at: now,
                expires_at: now + seconds,
            })
            .await?;
        self.set_cookie("colossus_session", &session, seconds)
    }
    fn authority_hash(&self, token: &str) -> String {
        if let Some(oidc) = &self.config.oidc {
            hash(&format!("{}\0{}\0{}", oidc.issuer, oidc.client_id, token))
        } else {
            self.local_session_hash(token)
        }
    }
    fn local_session_hash(&self, token: &str) -> String {
        hash(&format!("local\0{}\0{}", self.config.public_origin, token))
    }
    pub async fn logout(&self, headers: &HeaderMap) -> CloudResult<HeaderValue> {
        if headers.get("origin").and_then(|value| value.to_str().ok())
            != Some(self.config.public_origin.trim_end_matches('/'))
        {
            return Err(CloudError::PermissionDenied);
        }
        if let Some(token) = cookie(headers, "colossus_session") {
            self.store
                .delete_session(&self.authority_hash(&token))
                .await?;
            self.store
                .delete_session(&self.local_session_hash(&token))
                .await?;
        }
        self.set_cookie("colossus_session", "", 0)
    }
    fn seal_flow(&self, id: &str, flow: &Flow) -> CloudResult<serde_json::Value> {
        let plaintext = Zeroizing::new(serde_json::to_vec(flow).map_err(|_| CloudError::Storage)?);
        let Some(key) = &self.flow_key else {
            return serde_json::to_value(flow).map_err(|_| CloudError::Storage);
        };
        let mut nonce = [0u8; 24];
        getrandom::fill(&mut nonce).map_err(|_| CloudError::Storage)?;
        let cipher =
            XChaCha20Poly1305::new_from_slice(key.as_ref()).map_err(|_| CloudError::Storage)?;
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: id.as_bytes(),
                },
            )
            .map_err(|_| CloudError::Storage)?;
        Ok(
            serde_json::json!({"nonce":hex::encode(nonce),"ciphertext":hex::encode(ciphertext),"expires_at":flow.expires_at}),
        )
    }
    fn open_flow(&self, id: &str, value: &serde_json::Value) -> CloudResult<Flow> {
        let Some(key) = &self.flow_key else {
            return serde_json::from_value(value.clone()).map_err(|_| CloudError::PermissionDenied);
        };
        let nonce = hex::decode(
            value
                .get("nonce")
                .and_then(serde_json::Value::as_str)
                .ok_or(CloudError::PermissionDenied)?,
        )
        .map_err(|_| CloudError::PermissionDenied)?;
        let encoded = value
            .get("ciphertext")
            .and_then(serde_json::Value::as_str)
            .filter(|value| value.len() <= 16_384)
            .ok_or(CloudError::PermissionDenied)?;
        if nonce.len() != 24 {
            return Err(CloudError::PermissionDenied);
        }
        let ciphertext = hex::decode(encoded).map_err(|_| CloudError::PermissionDenied)?;
        let cipher =
            XChaCha20Poly1305::new_from_slice(key.as_ref()).map_err(|_| CloudError::Storage)?;
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: &ciphertext,
                        aad: id.as_bytes(),
                    },
                )
                .map_err(|_| CloudError::PermissionDenied)?,
        );
        serde_json::from_slice(&plaintext).map_err(|_| CloudError::PermissionDenied)
    }
    fn set_cookie(&self, name: &str, value: &str, seconds: u64) -> CloudResult<HeaderValue> {
        HeaderValue::from_str(&format!(
            "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={seconds}{}",
            if self.config.local_development {
                ""
            } else {
                "; Secure"
            }
        ))
        .map_err(|_| CloudError::Storage)
    }
}
fn chrono_now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}

fn flow_key(id: &str) -> EntityKey {
    EntityKey {
        kind: EntityKind::AuthFlow,
        project_id: "__auth".into(),
        parent_id: None,
        id: id.into(),
    }
}
