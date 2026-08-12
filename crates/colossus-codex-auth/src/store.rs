use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use directories::BaseDirs;
#[cfg(windows)]
use fs4::fs_std::FileExt as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(unix)]
use std::io::Write as _;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read as _,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
#[cfg(any(unix, windows))]
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
    /// response switches ChatGPT accounts. Concurrent Colossus processes serialize on a
    /// cross-process advisory lock, and the stored credentials are re-read immediately
    /// before persistence so an external writer such as the official Codex CLI cannot be
    /// overwritten with a stale snapshot.
    pub fn apply_refresh(
        &self,
        expected: &CodexAuthorization,
        response: &[u8],
    ) -> Result<CodexAuthorization, CodexAuthError> {
        let _guard = self
            .update_lock
            .lock()
            .map_err(|_| CodexAuthError::Storage("credential update lock was poisoned".into()))?;
        let _file_guard = AuthUpdateLock::acquire(&self.path)?;
        let mut stored = read_stored_auth(&self.path)?;
        let witness = AuthWitness::capture(&stored)?;
        let tokens = stored.tokens.as_mut().ok_or_else(missing_tokens)?;
        if tokens.refresh_token != expected.refresh_token() {
            return Err(credentials_changed());
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
        if !witness.matches(&read_stored_auth(&self.path)?) {
            return Err(credentials_changed());
        }
        write_stored_auth(&self.path, &stored)?;
        authorization_from_file(&stored)
    }
}

/// Credential fields that must stay untouched between the refresh check and the write.
struct AuthWitness {
    access_token: Zeroizing<String>,
    refresh_token: Zeroizing<String>,
    id_token: Zeroizing<String>,
    last_refresh: Option<String>,
}

impl AuthWitness {
    fn capture(stored: &StoredAuth) -> Result<Self, CodexAuthError> {
        let tokens = stored.tokens.as_ref().ok_or_else(missing_tokens)?;
        Ok(Self {
            access_token: Zeroizing::new(tokens.access_token.clone()),
            refresh_token: Zeroizing::new(tokens.refresh_token.clone()),
            id_token: Zeroizing::new(tokens.id_token.clone()),
            last_refresh: stored.last_refresh.clone(),
        })
    }

    fn matches(&self, stored: &StoredAuth) -> bool {
        stored.last_refresh == self.last_refresh
            && stored.tokens.as_ref().is_some_and(|tokens| {
                tokens.access_token == *self.access_token
                    && tokens.refresh_token == *self.refresh_token
                    && tokens.id_token == *self.id_token
            })
    }
}

/// Cross-process advisory lock over one Codex credential file.
///
/// The lock lives on a sibling `<name>.lock` file so the credential file itself is only
/// ever replaced atomically, and it is released when the guard closes its descriptor.
struct AuthUpdateLock {
    #[cfg(any(unix, windows))]
    _file: File,
}

impl AuthUpdateLock {
    #[cfg(unix)]
    fn acquire(path: &Path) -> Result<Self, CodexAuthError> {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| CodexAuthError::Storage("Codex auth path has no file name".into()))?;
        let parent = path.parent().ok_or_else(|| {
            CodexAuthError::Storage("Codex auth path has no parent directory".into())
        })?;
        let lock_path = parent.join(format!("{file_name}.lock"));
        let file = rustix::fs::open(
            &lock_path,
            rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .map(File::from)
        .map_err(|error| {
            CodexAuthError::Storage(format!("{} is not lockable ({error})", lock_path.display()))
        })?;
        let metadata = file
            .metadata()
            .map_err(|error| CodexAuthError::Storage(error.to_string()))?;
        if !metadata.file_type().is_file() {
            return Err(CodexAuthError::Storage(
                "Codex credential lock path must be a regular non-symlink file".into(),
            ));
        }
        validate_private_permissions(&metadata)?;
        rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive).map_err(|error| {
            CodexAuthError::Storage(format!("Codex credential lock is unavailable ({error})"))
        })?;
        Ok(Self { _file: file })
    }

    #[cfg(windows)]
    fn acquire(path: &Path) -> Result<Self, CodexAuthError> {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| CodexAuthError::Storage("Codex auth path has no file name".into()))?;
        let parent = path.parent().ok_or_else(|| {
            CodexAuthError::Storage("Codex auth path has no parent directory".into())
        })?;
        let lock_path = parent.join(format!("{file_name}.lock"));
        let parent_binding = colossus_windows_native::BoundPath::open_directory(parent)
            .and_then(|binding| {
                binding.validate_ancestor_namespace_authority()?;
                binding.validate_private_owner_dacl()?;
                binding.revalidate()?;
                Ok(binding)
            })
            .map_err(|_| {
                CodexAuthError::Storage(
                    "the Codex credential directory is not owner-private".into(),
                )
            })?;

        let binding = match open_private_windows_lock(&lock_path) {
            Ok(binding) => binding,
            Err(error) if windows_native_not_found(&error) => {
                // Another Colossus process may win this exclusive creation. Ignore
                // only a creation error that is followed by a fully validated reopen.
                if let Err(create_error) =
                    colossus_windows_native::create_private_file(&lock_path, &[])
                    && windows_native_not_found(&create_error)
                {
                    return Err(CodexAuthError::Storage(
                        "the Codex credential lock could not be created safely".into(),
                    ));
                }
                open_private_windows_lock(&lock_path).map_err(|_| {
                    CodexAuthError::Storage(
                        "the Codex credential lock could not be opened safely".into(),
                    )
                })?
            }
            Err(_) => {
                return Err(CodexAuthError::Storage(
                    "the Codex credential lock could not be opened safely".into(),
                ));
            }
        };
        let file = binding.try_clone_file().map_err(|_| {
            CodexAuthError::Storage("the Codex credential lock could not be opened safely".into())
        })?;
        file.lock_exclusive().map_err(|_| {
            CodexAuthError::Storage("the Codex credential lock is unavailable".into())
        })?;
        binding.revalidate().map_err(|_| {
            CodexAuthError::Storage("the Codex credential lock changed while opening".into())
        })?;
        parent_binding.revalidate().map_err(|_| {
            CodexAuthError::Storage("the Codex credential directory changed while opening".into())
        })?;
        Ok(Self { _file: file })
    }

    #[cfg(not(any(unix, windows)))]
    fn acquire(_path: &Path) -> Result<Self, CodexAuthError> {
        Err(CodexAuthError::Storage(
            "safe Codex credential updates are unsupported on this platform".into(),
        ))
    }
}

#[cfg(windows)]
fn open_private_windows_lock(
    path: &Path,
) -> Result<colossus_windows_native::BoundPath, colossus_windows_native::WindowsNativeError> {
    let binding = colossus_windows_native::BoundPath::open_file_read_write(path)?;
    binding.validate_ancestor_namespace_authority()?;
    binding.validate_private_owner_dacl()?;
    if binding.link_count()? != 1 {
        return Err(colossus_windows_native::WindowsNativeError::UnsafePermissions);
    }
    binding.revalidate()?;
    Ok(binding)
}

#[cfg(windows)]
fn windows_native_not_found(error: &colossus_windows_native::WindowsNativeError) -> bool {
    matches!(
        error,
        colossus_windows_native::WindowsNativeError::Io { source, .. }
            if source.kind() == std::io::ErrorKind::NotFound
    )
}

fn credentials_changed() -> CodexAuthError {
    CodexAuthError::Storage("Codex credentials changed while a token refresh was in flight".into())
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

struct OpenAuthFile {
    file: File,
    #[cfg(windows)]
    binding: colossus_windows_native::BoundPath,
}

impl OpenAuthFile {
    fn revalidate_after_read(&self) -> Result<(), CodexAuthError> {
        #[cfg(windows)]
        {
            self.binding
                .validate_ancestor_namespace_authority()
                .and_then(|()| self.binding.validate_private_owner_dacl())
                .and_then(|()| self.binding.revalidate())
                .map_err(|_| {
                    CodexAuthError::Storage(
                        "the Codex auth file changed while credentials were being read".into(),
                    )
                })?;
            if self.binding.link_count().ok() != Some(1) {
                return Err(CodexAuthError::Storage(
                    "the Codex auth file changed while credentials were being read".into(),
                ));
            }
        }
        Ok(())
    }
}

fn read_stored_auth(path: &Path) -> Result<StoredAuth, CodexAuthError> {
    let mut file = open_auth_file(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            CodexAuthError::Unavailable(
                "no file-backed ChatGPT credential was found; run `colossus codex login`".into(),
            )
        } else {
            CodexAuthError::Storage(
                "the Codex auth file could not be opened safely as a regular non-symlink file"
                    .into(),
            )
        }
    })?;
    let metadata = file
        .file
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
    std::io::Read::by_ref(&mut file.file)
        .take(MAX_AUTH_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| CodexAuthError::Storage(error.to_string()))?;
    file.revalidate_after_read()?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_AUTH_FILE_BYTES {
        return Err(CodexAuthError::Storage(
            "Codex auth file changed beyond the 256 KiB safety bound".into(),
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| CodexAuthError::Unavailable("Codex auth file is invalid JSON".into()))
}

#[cfg(unix)]
fn open_auth_file(path: &Path) -> std::io::Result<OpenAuthFile> {
    rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map(File::from)
    .map(|file| OpenAuthFile { file })
    .map_err(Into::into)
}

#[cfg(windows)]
fn open_auth_file(path: &Path) -> std::io::Result<OpenAuthFile> {
    let binding =
        colossus_windows_native::BoundPath::open_file(path).map_err(windows_native_open_error)?;
    binding
        .validate_ancestor_namespace_authority()
        .and_then(|()| binding.validate_private_owner_dacl())
        .map_err(windows_native_open_error)?;
    if binding.link_count().map_err(windows_native_open_error)? != 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Codex auth file has multiple filesystem names",
        ));
    }
    binding.revalidate().map_err(windows_native_open_error)?;
    let file = binding
        .try_clone_file()
        .map_err(windows_native_open_error)?;
    Ok(OpenAuthFile { file, binding })
}

#[cfg(windows)]
fn windows_native_open_error(error: colossus_windows_native::WindowsNativeError) -> std::io::Error {
    match error {
        colossus_windows_native::WindowsNativeError::Io { source, .. } => source,
        _ => std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "unsafe Codex credential path",
        ),
    }
}

#[cfg(not(any(unix, windows)))]
fn open_auth_file(_path: &Path) -> std::io::Result<OpenAuthFile> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "safe Codex credential access is unsupported on this platform",
    ))
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

#[cfg(windows)]
fn validate_private_permissions(_metadata: &fs::Metadata) -> Result<(), CodexAuthError> {
    // Windows owner, DACL, reparse-point, and link-count validation happens while
    // opening through `BoundPath`, before the retained handle is cloned.
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn validate_private_permissions(_metadata: &fs::Metadata) -> Result<(), CodexAuthError> {
    Err(CodexAuthError::Storage(
        "safe Codex credential access is unsupported on this platform".into(),
    ))
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
    let bytes = Zeroizing::new(
        serde_json::to_vec_pretty(stored)
            .map_err(|error| CodexAuthError::Storage(error.to_string()))?,
    );
    if bytes.len() > usize::try_from(MAX_AUTH_FILE_BYTES).unwrap_or(usize::MAX) {
        return Err(CodexAuthError::Storage(
            "updated Codex auth file exceeds the safety bound".into(),
        ));
    }
    write_auth_bytes(path, &bytes)
}

#[cfg(unix)]
fn write_auth_bytes(path: &Path, bytes: &[u8]) -> Result<(), CodexAuthError> {
    let parent = path
        .parent()
        .ok_or_else(|| CodexAuthError::Storage("Codex auth path has no parent directory".into()))?;
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|error| CodexAuthError::Storage(error.to_string()))?;
    set_private_permissions(temporary.as_file())?;
    temporary
        .write_all(bytes)
        .and_then(|()| temporary.as_file_mut().sync_all())
        .map_err(|error| CodexAuthError::Storage(error.to_string()))?;
    temporary
        .persist(path)
        .map_err(|error| CodexAuthError::Storage(error.error.to_string()))?;
    Ok(())
}

#[cfg(windows)]
fn write_auth_bytes(path: &Path, bytes: &[u8]) -> Result<(), CodexAuthError> {
    let parent = path
        .parent()
        .ok_or_else(|| CodexAuthError::Storage("Codex auth path has no parent directory".into()))?;
    let parent_binding = colossus_windows_native::BoundPath::open_directory(parent)
        .and_then(|binding| {
            binding.validate_ancestor_namespace_authority()?;
            binding.validate_private_owner_dacl()?;
            binding.revalidate()?;
            Ok(binding)
        })
        .map_err(|_| {
            CodexAuthError::Storage("the Codex credential directory is not owner-private".into())
        })?;

    // `NamedTempFile` is used only to reserve an unpredictable same-directory name.
    // It is removed before any credential bytes exist; the actual staged file is then
    // created with an explicit protected DACL by the Windows native boundary.
    let reservation = NamedTempFile::new_in(parent).map_err(|_| {
        CodexAuthError::Storage("a private Codex credential update could not be staged".into())
    })?;
    let temporary_path = reservation.path().to_owned();
    reservation.close().map_err(|_| {
        CodexAuthError::Storage("a private Codex credential update could not be staged".into())
    })?;
    colossus_windows_native::create_private_file(&temporary_path, bytes).map_err(|_| {
        CodexAuthError::Storage("a private Codex credential update could not be staged".into())
    })?;

    let replace = (|| {
        let temporary = colossus_windows_native::BoundPath::open_file(&temporary_path)?;
        temporary.validate_ancestor_namespace_authority()?;
        temporary.validate_private_owner_dacl()?;
        if temporary.link_count()? != 1 {
            return Err(colossus_windows_native::WindowsNativeError::UnsafePermissions);
        }
        temporary.revalidate()?;
        parent_binding.revalidate()?;
        colossus_windows_native::replace_private_file(&temporary_path, path)?;
        parent_binding.revalidate()?;
        let committed = colossus_windows_native::BoundPath::open_file(path)?;
        committed.validate_ancestor_namespace_authority()?;
        committed.validate_private_owner_dacl()?;
        if committed.link_count()? != 1 {
            return Err(colossus_windows_native::WindowsNativeError::UnsafePermissions);
        }
        committed.revalidate()
    })();
    if replace.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    replace.map_err(|_| {
        CodexAuthError::Storage("the Codex credential update could not commit safely".into())
    })
}

#[cfg(not(any(unix, windows)))]
fn write_auth_bytes(_path: &Path, _bytes: &[u8]) -> Result<(), CodexAuthError> {
    Err(CodexAuthError::Storage(
        "safe Codex credential updates are unsupported on this platform".into(),
    ))
}

#[cfg(unix)]
fn set_private_permissions(file: &File) -> Result<(), CodexAuthError> {
    use std::os::unix::fs::PermissionsExt as _;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| CodexAuthError::Storage(error.to_string()))
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
        let contents = serde_json::to_vec(value).expect("auth serializes");
        #[cfg(windows)]
        colossus_windows_native::create_private_file(path, &contents)
            .expect("private auth file writes");
        #[cfg(not(windows))]
        fs::write(path, contents).expect("auth file writes");
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
    fn concurrent_refreshes_serialize_to_one_winner() {
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
        // Independent stores share no process-local mutex, so only the cross-process
        // lock plus the pre-write comparison can keep one stale snapshot from winning.
        let first = CodexAuthStore::at_path(&path);
        let second = CodexAuthStore::at_path(&path);
        let first_snapshot = first.load().expect("first snapshot loads");
        let second_snapshot = second.load().expect("second snapshot loads");
        let response = serde_json::to_vec(&json!({
            "access_token": jwt(json!({"exp": OffsetDateTime::now_utc().unix_timestamp() + 3600})),
            "refresh_token": "refresh-2",
            "id_token": id_token
        }))
        .expect("refresh serializes");
        let applied = std::thread::scope(|scope| {
            let left = scope.spawn(|| first.apply_refresh(&first_snapshot, &response).is_ok());
            let right = scope.spawn(|| second.apply_refresh(&second_snapshot, &response).is_ok());
            [
                left.join().expect("left thread joins"),
                right.join().expect("right thread joins"),
            ]
        });
        assert_eq!(applied.into_iter().filter(|applied| *applied).count(), 1);
        assert_eq!(
            CodexAuthStore::at_path(&path)
                .load()
                .expect("rotated auth loads")
                .refresh_token(),
            "refresh-2"
        );
        #[cfg(unix)]
        assert!(path.with_file_name("auth.json.lock").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_group_or_other_readable_auth_files() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("auth.json");
        write_auth(&path, &json!({}));
        use std::os::unix::fs::PermissionsExt as _;
        for mode in [0o640, 0o644] {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode))
                .expect("permissions update");
            assert!(matches!(
                CodexAuthStore::at_path(&path).load(),
                Err(CodexAuthError::Storage(_))
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_auth_fifo_promptly_without_waiting_for_a_writer() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("auth.json");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .expect("run mkfifo")
                .success()
        );

        let started = std::time::Instant::now();
        assert!(matches!(
            CodexAuthStore::at_path(path).load(),
            Err(CodexAuthError::Storage(_))
        ));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "auth FIFO validation must not wait for a writer"
        );
    }

    #[cfg(windows)]
    #[test]
    fn loads_owner_private_windows_auth_file() {
        let directory = private_windows_directory();
        let path = directory.path().join("auth.json");
        write_auth(&path, &valid_auth("refresh-1"));

        CodexAuthStore::at_path(path)
            .load()
            .expect("owner-private Windows auth loads");
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_auth_hard_link() {
        let directory = private_windows_directory();
        let path = directory.path().join("auth.json");
        write_auth(&path, &valid_auth("refresh-1"));
        std::fs::hard_link(&path, directory.path().join("auth-alias.json"))
            .expect("create auth hard link");

        assert!(matches!(
            CodexAuthStore::at_path(path).load(),
            Err(CodexAuthError::Storage(_))
        ));
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_auth_with_broad_dacl() {
        let directory = private_windows_directory();
        let path = directory.path().join("auth.json");
        write_auth(&path, &valid_auth("refresh-1"));
        grant_everyone(&path, "(R)");

        assert!(matches!(
            CodexAuthStore::at_path(path).load(),
            Err(CodexAuthError::Storage(_))
        ));
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_auth_beneath_junction() {
        let parent = private_windows_directory();
        let target = parent.path().join("target");
        colossus_windows_native::create_private_directory(&target)
            .expect("private target directory");
        write_auth(&target.join("auth.json"), &valid_auth("refresh-1"));
        let junction = parent.path().join("linked");
        let status = std::process::Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&target)
            .status()
            .expect("create junction");
        assert!(status.success(), "create auth-directory junction");

        assert!(matches!(
            CodexAuthStore::at_path(junction.join("auth.json")).load(),
            Err(CodexAuthError::Storage(_))
        ));
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_auth_beneath_replaceable_parent() {
        let outer = private_windows_directory();
        let directory = outer.path().join("codex");
        colossus_windows_native::create_private_directory(&directory)
            .expect("private Codex directory");
        let path = directory.join("auth.json");
        write_auth(&path, &valid_auth("refresh-1"));
        grant_everyone(outer.path(), "(DC)");

        assert!(matches!(
            CodexAuthStore::at_path(path).load(),
            Err(CodexAuthError::Storage(_))
        ));
    }

    #[cfg(windows)]
    #[test]
    fn concurrent_windows_refreshes_have_one_winner() {
        let directory = private_windows_directory();
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
        let first = CodexAuthStore::at_path(&path);
        let second = CodexAuthStore::at_path(&path);
        let first_snapshot = first.load().expect("first snapshot loads");
        let second_snapshot = second.load().expect("second snapshot loads");
        let response = serde_json::to_vec(&json!({
            "access_token": jwt(json!({"exp": OffsetDateTime::now_utc().unix_timestamp() + 3600})),
            "refresh_token": "refresh-2",
            "id_token": id_token
        }))
        .expect("refresh serializes");
        let applied = std::thread::scope(|scope| {
            let left = scope.spawn(|| first.apply_refresh(&first_snapshot, &response).is_ok());
            let right = scope.spawn(|| second.apply_refresh(&second_snapshot, &response).is_ok());
            [
                left.join().expect("left thread joins"),
                right.join().expect("right thread joins"),
            ]
        });
        assert_eq!(applied.into_iter().filter(|applied| *applied).count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn refresh_preserves_private_windows_file_and_rejects_unsafe_lock() {
        let directory = private_windows_directory();
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
        let store = CodexAuthStore::at_path(&path);
        let before = store.load().expect("initial auth loads");
        let response = serde_json::to_vec(&json!({
            "access_token": jwt(json!({"exp": OffsetDateTime::now_utc().unix_timestamp() + 3600})),
            "refresh_token": "refresh-2",
            "id_token": id_token
        }))
        .expect("refresh serializes");
        store
            .apply_refresh(&before, &response)
            .expect("Windows refresh applies");
        let committed =
            colossus_windows_native::BoundPath::open_file(&path).expect("bind refreshed auth");
        committed
            .validate_private_owner_dacl()
            .expect("refreshed auth remains private");
        assert_eq!(committed.link_count().expect("auth link count"), 1);

        let lock = path.with_file_name("auth.json.lock");
        grant_everyone(&lock, "(R)");
        let current = store.load().expect("refreshed auth loads");
        assert!(matches!(
            store.apply_refresh(&current, &response),
            Err(CodexAuthError::Storage(_))
        ));
    }

    #[cfg(windows)]
    struct WindowsTestDirectory {
        _outer: tempfile::TempDir,
        path: PathBuf,
    }

    #[cfg(windows)]
    impl WindowsTestDirectory {
        fn path(&self) -> &Path {
            &self.path
        }
    }

    #[cfg(windows)]
    fn private_windows_directory() -> WindowsTestDirectory {
        let outer = tempfile::tempdir().expect("temporary parent");
        let path = outer.path().join("private");
        colossus_windows_native::create_private_directory(&path).expect("private test directory");
        WindowsTestDirectory {
            _outer: outer,
            path,
        }
    }

    #[cfg(windows)]
    fn valid_auth(refresh_token: &str) -> Value {
        json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": jwt(json!({"https://api.openai.com/auth": {
                    "chatgpt_account_id": "account-1"
                }})),
                "access_token": jwt(json!({"exp": OffsetDateTime::now_utc().unix_timestamp() + 3600})),
                "refresh_token": refresh_token
            }
        })
    }

    #[cfg(windows)]
    fn grant_everyone(path: &Path, rights: &str) {
        let status = std::process::Command::new("icacls.exe")
            .arg(path)
            .args(["/grant", &format!("*S-1-1-0:{rights}")])
            .status()
            .expect("run Windows ACL editor");
        assert!(status.success(), "grant Everyone {rights}");
    }
}
