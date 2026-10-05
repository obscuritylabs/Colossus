use crate::config::Config;
use axum::http::{HeaderMap, HeaderValue};
use colossus_cloud::{CloudCaller, CloudError, CloudResult};
use openidconnect::{
    AccessTokenHash, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet,
    EndpointNotSet, EndpointSet, IssuerUrl, Nonce, OAuth2TokenResponse, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, TokenResponse,
    core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;
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
struct Flow {
    verifier: PkceCodeVerifier,
    nonce: Nonce,
    expires: Instant,
}
struct Session {
    subject: String,
    expires: Instant,
}
pub(crate) struct Authentication {
    client: Client,
    client_secret: Option<ClientSecret>,
    http: reqwest::Client,
    config: Config,
    flows: Mutex<HashMap<String, Flow>>,
    sessions: Mutex<HashMap<String, Session>>,
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
    pub async fn new(config: Config) -> Result<Self, &'static str> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| "OIDC HTTP configuration failed")?;
        let metadata = CoreProviderMetadata::discover_async(
            IssuerUrl::new(config.oidc.issuer.clone()).map_err(|_| "invalid issuer")?,
            &http,
        )
        .await
        .map_err(|_| "OIDC discovery failed")?;
        let secret = config
            .oidc
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
            ClientId::new(config.oidc.client_id.clone()),
            secret.clone(),
        )
        .set_redirect_uri(
            RedirectUrl::new(format!(
                "{}/auth/callback",
                config.public_origin.trim_end_matches('/')
            ))
            .map_err(|_| "invalid callback")?,
        );
        Ok(Self {
            client,
            client_secret: secret,
            http,
            config,
            flows: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
        })
    }
    pub fn login(&self) -> CloudResult<(String, HeaderValue)> {
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (url, state, nonce) = self
            .client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .set_pkce_challenge(challenge)
            .url();
        let mut flows = self.flows.lock().map_err(|_| CloudError::Storage)?;
        flows.retain(|_, flow| flow.expires > Instant::now());
        if flows.len() >= 128 {
            return Err(CloudError::ResourceExhausted);
        }
        flows.insert(
            hash(state.secret()),
            Flow {
                verifier,
                nonce,
                expires: Instant::now() + Duration::from_secs(300),
            },
        );
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
        let flow = self
            .flows
            .lock()
            .map_err(|_| CloudError::Storage)?
            .remove(&hash(state))
            .filter(|flow| flow.expires > Instant::now())
            .ok_or(CloudError::PermissionDenied)?;
        let token = self
            .client
            .exchange_code(AuthorizationCode::new(code))
            .map_err(|_| CloudError::PermissionDenied)?
            .set_pkce_verifier(flow.verifier)
            .request_async(&self.http)
            .await
            .map_err(|_| CloudError::PermissionDenied)?;
        let id = token.id_token().ok_or(CloudError::PermissionDenied)?;
        // Refresh issuer-owned keys for each new session. An IdP key rotation
        // must neither require a host restart nor keep a retired key trusted.
        let metadata = CoreProviderMetadata::discover_async(
            IssuerUrl::new(self.config.oidc.issuer.clone())
                .map_err(|_| CloudError::PermissionDenied)?,
            &self.http,
        )
        .await
        .map_err(|_| CloudError::PermissionDenied)?;
        let verification_client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(self.config.oidc.client_id.clone()),
            self.client_secret.clone(),
        );
        let verifier = verification_client.id_token_verifier();
        let claims = id
            .claims(&verifier, &flow.nonce)
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
        let subject = claims.subject().as_str().to_owned();
        if !self
            .config
            .memberships
            .iter()
            .any(|member| member.subject == subject)
        {
            return Err(CloudError::PermissionDenied);
        }
        let seconds = (claims.expiration() - chrono_now())
            .num_seconds()
            .clamp(0, 8 * 60 * 60) as u64;
        if seconds == 0 {
            return Err(CloudError::PermissionDenied);
        }
        let session = Nonce::new_random().secret().to_owned();
        let mut sessions = self.sessions.lock().map_err(|_| CloudError::Storage)?;
        sessions.retain(|_, session| session.expires > Instant::now());
        if sessions.len() >= 1024 {
            return Err(CloudError::ResourceExhausted);
        }
        sessions.insert(
            hash(&session),
            Session {
                subject,
                expires: Instant::now() + Duration::from_secs(seconds),
            },
        );
        self.set_cookie("colossus_session", &session, seconds)
    }
    pub fn caller(
        &self,
        headers: &HeaderMap,
        project: &str,
        mutation: bool,
    ) -> CloudResult<CloudCaller> {
        if mutation
            && (headers.get("origin").and_then(|value| value.to_str().ok())
                != Some(self.config.public_origin.trim_end_matches('/'))
                || headers
                    .get("x-colossus-csrf")
                    .and_then(|value| value.to_str().ok())
                    != Some("1"))
        {
            return Err(CloudError::PermissionDenied);
        }
        let token = cookie(headers, "colossus_session").ok_or(CloudError::PermissionDenied)?;
        let sessions = self.sessions.lock().map_err(|_| CloudError::Storage)?;
        let session = sessions
            .get(&hash(&token))
            .filter(|session| session.expires > Instant::now())
            .ok_or(CloudError::PermissionDenied)?;
        let member = self
            .config
            .memberships
            .iter()
            .find(|member| member.subject == session.subject && member.project_id == project)
            .ok_or(CloudError::PermissionDenied)?;
        CloudCaller::new(
            member.subject.clone(),
            member.project_id.clone(),
            member.permissions.clone(),
        )
    }
    pub fn memberships(&self, headers: &HeaderMap) -> CloudResult<Vec<crate::config::Membership>> {
        let token = cookie(headers, "colossus_session").ok_or(CloudError::PermissionDenied)?;
        let sessions = self.sessions.lock().map_err(|_| CloudError::Storage)?;
        let session = sessions
            .get(&hash(&token))
            .filter(|session| session.expires > Instant::now())
            .ok_or(CloudError::PermissionDenied)?;
        Ok(self
            .config
            .memberships
            .iter()
            .filter(|member| member.subject == session.subject)
            .cloned()
            .collect())
    }
    pub fn logout(&self, headers: &HeaderMap) -> CloudResult<HeaderValue> {
        if headers.get("origin").and_then(|value| value.to_str().ok())
            != Some(self.config.public_origin.trim_end_matches('/'))
        {
            return Err(CloudError::PermissionDenied);
        }
        if let Some(token) = cookie(headers, "colossus_session") {
            self.sessions
                .lock()
                .map_err(|_| CloudError::Storage)?
                .remove(&hash(&token));
        }
        self.set_cookie("colossus_session", "", 0)
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
