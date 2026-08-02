use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tempfile::NamedTempFile;
use thiserror::Error;
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
use zeroize::{Zeroize as _, Zeroizing};

const MAX_AUTH_FILE_BYTES: u64 = 256 * 1024;
const REFRESH_WINDOW: Duration = Duration::minutes(5);

/// Failure to locate, read, validate, update, or operate a Codex sign-in.
#[derive(Debug, Error)]
pub enum CodexAuthError {
    /// The configured credential file could not be safely accessed.
    #[error("Codex credential storage error: {0}")]
    Storage(String),
    /// The credential file did not contain a usable ChatGPT sign-in.
    #[error("Codex sign-in is unavailable: {0}")]
    Unavailable(String),
    /// The official Codex CLI could not complete an operator-requested action.
    #[error("Codex CLI error: {0}")]
    Cli(String),
}

/// A validated, zeroizing authorization snapshot loaded from Codex-managed storage.
pub struct CodexAuthorization {
    access_token: Zeroizing<String>,
    refresh_token: Zeroizing<String>,
    account_id: Zeroizing<String>,
    expires_at: Option<OffsetDateTime>,
    fedramp: bool,
}

impl CodexAuthorization {
    /// Bearer token for the ChatGPT Codex backend.
    pub fn access_token(&self) -> &str {
        self.access_token.as_str()
    }

    /// Refresh token sent only to the fixed OpenAI token endpoint.
    pub fn refresh_token(&self) -> &str {
        self.refresh_token.as_str()
    }

    /// ChatGPT account identifier required by the Codex backend.
    pub fn account_id(&self) -> &str {
        self.account_id.as_str()
    }

    /// Whether the sign-in belongs to the OpenAI FedRAMP environment.
    pub fn is_fedramp(&self) -> bool {
        self.fedramp
    }

    /// Whether the access token is expired or enters its proactive refresh window.
    pub fn requires_refresh(&self, now: OffsetDateTime) -> bool {
        self.expires_at
            .is_some_and(|expires_at| expires_at <= now + REFRESH_WINDOW)
    }
}

/// JSON body accepted by the fixed OpenAI refresh endpoint.
#[derive(Serialize)]
pub struct CodexRefreshRequest<'a> {
    client_id: &'static str,
    grant_type: &'static str,
    refresh_token: &'a str,
}

impl<'a> CodexRefreshRequest<'a> {
    /// Build a refresh request for one validated authorization snapshot.
    pub fn new(authorization: &'a CodexAuthorization) -> Self {
        Self {
            client_id: "app_EMoamEEZ73f0CkXaXp7hrann",
            grant_type: "refresh_token",
            refresh_token: authorization.refresh_token(),
        }
    }
}

/// Location and process-local synchronization for Codex-managed credentials.
#[derive(Clone)]
pub struct CodexAuthStore {
    path: PathBuf,
    update_lock: Arc<Mutex<()>>,
}

impl CodexAuthStore {
    /// Resolve `$CODEX_HOME/auth.json`, falling back to `~/.codex/auth.json`.
    pub fn from_environment() -> Result<Self, CodexAuthError> {
        let home = match std::env::var_os("CODEX_HOME") {
            Some(path) if !path.is_empty() => PathBuf::from(path),
            _ => BaseDirs::new()
                .map(|directories| directories.home_dir().join(".codex"))
                .ok_or_else(|| {
                    CodexAuthError::Storage("unable to resolve the user home directory".into())
                })?,
        };
        if !home.is_absolute() {
            return Err(CodexAuthError::Storage(
                "CODEX_HOME must be an absolute path".into(),
            ));
        }
        Ok(Self::at_path(home.join("auth.json")))
    }

    /// Use one explicit auth file, primarily for embedded hosts and tests.
    pub fn at_path(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            update_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Credential file path without reading it.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load and validate the current ChatGPT authorization without logging secrets.
    pub fn load(&self) -> Result<CodexAuthorization, CodexAuthError> {
        authorization_from_file(&read_stored_auth(&self.path)?)
    }

    /// Atomically merge a successful bounded refresh response into Codex storage.
    ///
    /// The update is refused if another process rotated the refresh token or if the
    /// response switches ChatGPT accounts.
    pub fn apply_refresh(
        &self,
        expected: &CodexAuthorization,
        response: &[u8],
    ) -> Result<CodexAuthorization, CodexAuthError> {
        let _guard = self
            .update_lock
            .lock()
            .map_err(|_| CodexAuthError::Storage("credential update lock was poisoned".into()))?;
        let mut stored = read_stored_auth(&self.path)?;
        let tokens = stored.tokens.as_mut().ok_or_else(missing_tokens)?;
        if tokens.refresh_token != expected.refresh_token() {
            return Err(CodexAuthError::Storage(
                "Codex credentials changed while a token refresh was in flight".into(),
            ));
        }
        let refresh: RefreshResponse = serde_json::from_slice(response).map_err(|_| {
            CodexAuthError::Unavailable(
                "OpenAI returned an invalid token refresh response; run `colossus codex login`"
                    .into(),
            )
        })?;
        if refresh.access_token.is_empty() {
            return Err(CodexAuthError::Unavailable(
                "OpenAI returned an empty access token; run `colossus codex login`".into(),
            ));
        }
        let candidate_id_token = refresh.id_token.as_deref().unwrap_or(&tokens.id_token);
        let candidate_account = token_metadata(candidate_id_token)?.account_id;
        if candidate_account.as_deref() != Some(expected.account_id()) {
            return Err(CodexAuthError::Unavailable(
                "refreshed Codex credentials changed ChatGPT accounts; run `colossus codex login`"
                    .into(),
            ));
        }
        tokens.access_token.clone_from(&refresh.access_token);
        if let Some(refresh_token) = refresh.refresh_token.as_deref() {
            if refresh_token.is_empty() {
                return Err(CodexAuthError::Unavailable(
                    "OpenAI returned an empty refresh token".into(),
                ));
            }
            tokens.refresh_token.clear();
            tokens.refresh_token.push_str(refresh_token);
        }
        if let Some(id_token) = refresh.id_token.as_deref() {
            tokens.id_token.clear();
            tokens.id_token.push_str(id_token);
        }
        stored.last_refresh = Some(
            OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .map_err(|error| CodexAuthError::Storage(error.to_string()))?,
        );
        write_stored_auth(&self.path, &stored)?;
        authorization_from_file(&stored)
    }
}

#[derive(Deserialize, Serialize)]
struct StoredAuth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_mode: Option<String>,
    #[serde(
        rename = "OPENAI_API_KEY",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    openai_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tokens: Option<StoredTokens>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_refresh: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Deserialize, Serialize)]
struct StoredTokens {
    id_token: String,
    access_token: String,
    refresh_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account_id: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

impl Drop for StoredAuth {
    fn drop(&mut self) {
        self.openai_api_key.zeroize();
        self.extra.values_mut().for_each(zeroize_json_value);
    }
}

impl Drop for StoredTokens {
    fn drop(&mut self) {
        self.id_token.zeroize();
        self.access_token.zeroize();
        self.refresh_token.zeroize();
        self.account_id.zeroize();
        self.extra.values_mut().for_each(zeroize_json_value);
    }
}

#[derive(Deserialize)]
struct RefreshResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

impl Drop for RefreshResponse {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
        self.id_token.zeroize();
    }
}

struct TokenMetadata {
    account_id: Option<String>,
    expires_at: Option<OffsetDateTime>,
    fedramp: bool,
}

fn read_stored_auth(path: &Path) -> Result<StoredAuth, CodexAuthError> {
    let mut file = open_auth_file(path).map_err(|error| {
        CodexAuthError::Unavailable(format!(
            "{} is not readable ({error}); run `colossus codex login`",
            path.display()
        ))
    })?;
    let metadata = file
        .metadata()
        .map_err(|error| CodexAuthError::Storage(error.to_string()))?;
    if !metadata.file_type().is_file() {
        return Err(CodexAuthError::Storage(
            "Codex auth path must be a regular non-symlink file".into(),
        ));
    }
    if metadata.len() > MAX_AUTH_FILE_BYTES {
        return Err(CodexAuthError::Storage(
            "Codex auth file exceeds the 256 KiB safety bound".into(),
        ));
    }
    validate_private_permissions(&metadata)?;
    let mut bytes = Zeroizing::new(Vec::with_capacity(
        usize::try_from(metadata.len()).unwrap_or(0),
    ));
    std::io::Read::by_ref(&mut file)
        .take(MAX_AUTH_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| CodexAuthError::Storage(error.to_string()))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_AUTH_FILE_BYTES {
        return Err(CodexAuthError::Storage(
            "Codex auth file changed beyond the 256 KiB safety bound".into(),
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| CodexAuthError::Unavailable("Codex auth file is invalid JSON".into()))
}

#[cfg(unix)]
fn open_auth_file(path: &Path) -> std::io::Result<File> {
    rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map(File::from)
    .map_err(Into::into)
}

#[cfg(not(unix))]
fn open_auth_file(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

#[cfg(unix)]
fn validate_private_permissions(metadata: &fs::Metadata) -> Result<(), CodexAuthError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    if metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(CodexAuthError::Storage(
            "Codex auth file must be owned by the current user and inaccessible by group or other users"
                .into(),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_permissions(_metadata: &fs::Metadata) -> Result<(), CodexAuthError> {
    Ok(())
}

fn authorization_from_file(stored: &StoredAuth) -> Result<CodexAuthorization, CodexAuthError> {
    if stored
        .auth_mode
        .as_deref()
        .is_some_and(|mode| !matches!(mode, "chatgpt" | "chatgpt_auth_tokens" | "Chatgpt"))
    {
        return Err(CodexAuthError::Unavailable(
            "Codex is not signed in with ChatGPT; run `colossus codex login`".into(),
        ));
    }
    let tokens = stored.tokens.as_ref().ok_or_else(missing_tokens)?;
    if tokens.access_token.is_empty()
        || tokens.refresh_token.is_empty()
        || tokens.id_token.is_empty()
    {
        return Err(missing_tokens());
    }
    let metadata = token_metadata(&tokens.id_token)?;
    let account_id = tokens
        .account_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .or(metadata.account_id.as_deref())
        .ok_or_else(|| {
            CodexAuthError::Unavailable(
                "Codex sign-in has no ChatGPT account identifier; sign in again".into(),
            )
        })?;
    let access_metadata = token_metadata(&tokens.access_token)?;
    Ok(CodexAuthorization {
        access_token: Zeroizing::new(tokens.access_token.clone()),
        refresh_token: Zeroizing::new(tokens.refresh_token.clone()),
        account_id: Zeroizing::new(account_id.to_owned()),
        expires_at: access_metadata.expires_at,
        fedramp: metadata.fedramp,
    })
}

fn missing_tokens() -> CodexAuthError {
    CodexAuthError::Unavailable(
        "no file-backed ChatGPT tokens were found; run `colossus codex login`".into(),
    )
}

fn token_metadata(token: &str) -> Result<TokenMetadata, CodexAuthError> {
    let payload = token.split('.').nth(1).ok_or_else(|| {
        CodexAuthError::Unavailable("Codex credential contains an invalid token".into())
    })?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).map_err(|_| {
        CodexAuthError::Unavailable("Codex credential contains an invalid token".into())
    })?;
    let claims: Value = serde_json::from_slice(&bytes).map_err(|_| {
        CodexAuthError::Unavailable("Codex credential contains invalid token claims".into())
    })?;
    let auth = claims
        .get("https://api.openai.com/auth")
        .and_then(Value::as_object);
    let account_id = auth
        .and_then(|claims| claims.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let fedramp = auth
        .and_then(|claims| claims.get("chatgpt_account_is_fedramp"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let expires_at = claims
        .get("exp")
        .and_then(Value::as_i64)
        .and_then(|timestamp| OffsetDateTime::from_unix_timestamp(timestamp).ok());
    Ok(TokenMetadata {
        account_id,
        expires_at,
        fedramp,
    })
}

fn zeroize_json_value(value: &mut Value) {
    match value {
        Value::String(value) => value.zeroize(),
        Value::Array(values) => values.iter_mut().for_each(zeroize_json_value),
        Value::Object(values) => values.values_mut().for_each(zeroize_json_value),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn write_stored_auth(path: &Path, stored: &StoredAuth) -> Result<(), CodexAuthError> {
    let parent = path
        .parent()
        .ok_or_else(|| CodexAuthError::Storage("Codex auth path has no parent directory".into()))?;
    let bytes = Zeroizing::new(
        serde_json::to_vec_pretty(stored)
            .map_err(|error| CodexAuthError::Storage(error.to_string()))?,
    );
    if bytes.len() > usize::try_from(MAX_AUTH_FILE_BYTES).unwrap_or(usize::MAX) {
        return Err(CodexAuthError::Storage(
            "updated Codex auth file exceeds the safety bound".into(),
        ));
    }
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|error| CodexAuthError::Storage(error.to_string()))?;
    set_private_permissions(temporary.as_file())?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file_mut().sync_all())
        .map_err(|error| CodexAuthError::Storage(error.to_string()))?;
    temporary
        .persist(path)
        .map_err(|error| CodexAuthError::Storage(error.error.to_string()))?;
    Ok(())
}

#[cfg(unix)]
fn set_private_permissions(file: &File) -> Result<(), CodexAuthError> {
    use std::os::unix::fs::PermissionsExt as _;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| CodexAuthError::Storage(error.to_string()))
}

#[cfg(not(unix))]
fn set_private_permissions(_file: &File) -> Result<(), CodexAuthError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn jwt(claims: Value) -> String {
        format!(
            "header.{}.signature",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("claims serialize"))
        )
    }

    fn write_auth(path: &Path, value: &Value) {
        fs::write(path, serde_json::to_vec(value).expect("auth serializes"))
            .expect("auth file writes");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .expect("permissions update");
        }
    }

    #[test]
    fn loads_chatgpt_tokens_and_account_claims() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("auth.json");
        let now = OffsetDateTime::now_utc().unix_timestamp();
        write_auth(
            &path,
            &json!({
                "auth_mode": "chatgpt",
                "tokens": {
                    "id_token": jwt(json!({"https://api.openai.com/auth": {
                        "chatgpt_account_id": "account-1",
                        "chatgpt_account_is_fedramp": true
                    }})),
                    "access_token": jwt(json!({"exp": now + 60})),
                    "refresh_token": "refresh-1"
                }
            }),
        );
        let authorization = CodexAuthStore::at_path(path).load().expect("auth loads");
        assert_eq!(authorization.access_token().split('.').count(), 3);
        assert_eq!(authorization.account_id(), "account-1");
        assert!(authorization.is_fedramp());
        assert!(authorization.requires_refresh(OffsetDateTime::now_utc()));
    }

    #[test]
    fn refresh_updates_tokens_without_switching_accounts() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("auth.json");
        let id_token = jwt(json!({"https://api.openai.com/auth": {
            "chatgpt_account_id": "account-1"
        }}));
        write_auth(
            &path,
            &json!({
                "auth_mode": "chatgpt",
                "tokens": {
                    "id_token": id_token,
                    "access_token": jwt(json!({"exp": 1})),
                    "refresh_token": "refresh-1"
                }
            }),
        );
        let store = CodexAuthStore::at_path(path);
        let before = store.load().expect("initial auth loads");
        let response = serde_json::to_vec(&json!({
            "access_token": jwt(json!({"exp": OffsetDateTime::now_utc().unix_timestamp() + 3600})),
            "refresh_token": "refresh-2",
            "id_token": id_token
        }))
        .expect("refresh serializes");
        let after = store
            .apply_refresh(&before, &response)
            .expect("refresh applies");
        assert_eq!(after.refresh_token(), "refresh-2");
        assert_eq!(after.account_id(), "account-1");
        assert!(!after.requires_refresh(OffsetDateTime::now_utc()));
    }

    #[test]
    fn rejects_group_readable_auth_files() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("auth.json");
        write_auth(&path, &json!({}));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o640))
                .expect("permissions update");
            assert!(matches!(
                CodexAuthStore::at_path(path).load(),
                Err(CodexAuthError::Storage(_))
            ));
        }
    }
}
