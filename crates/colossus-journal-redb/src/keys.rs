use super::*;

/// Keyless provider selecting hash-chained plaintext journal payloads.
#[derive(Default)]
pub struct PlaintextKeyProvider;

impl KeyProvider for PlaintextKeyProvider {
    fn payload_protection(&self) -> JournalPayloadProtection {
        JournalPayloadProtection::Plaintext
    }

    fn active_key(&self) -> Result<(String, [u8; 32]), StoreError> {
        Err(StoreError::KeyUnavailable(
            "plaintext journal has no encryption key".into(),
        ))
    }

    fn key_by_id(&self, _key_id: &str) -> Result<[u8; 32], StoreError> {
        Err(StoreError::KeyUnavailable(
            "plaintext journal has no encryption key".into(),
        ))
    }

    fn store_anchor(&self, _anchor: &SecureAnchor) -> Result<(), StoreError> {
        Err(StoreError::Adapter(
            "plaintext journal does not support secure anchors".into(),
        ))
    }

    fn load_anchor(&self) -> Result<Option<SecureAnchor>, StoreError> {
        Ok(None)
    }
}

/// Explicit in-memory key provider for tests and embedded applications.
pub struct StaticKeyProvider {
    active_id: Mutex<String>,
    keys: Mutex<BTreeMap<String, [u8; 32]>>,
    anchor: Mutex<Option<SecureAnchor>>,
}

impl StaticKeyProvider {
    /// Create a provider with one active key.
    pub fn new(key_id: impl Into<String>, key: [u8; 32]) -> Self {
        let active_id = key_id.into();
        let mut keys = BTreeMap::new();
        keys.insert(active_id.clone(), key);
        Self {
            active_id: Mutex::new(active_id),
            keys: Mutex::new(keys),
            anchor: Mutex::new(None),
        }
    }

    /// Add a new key and atomically make it active while retaining historical keys.
    pub fn rotate(&self, key_id: impl Into<String>, key: [u8; 32]) -> Result<(), StoreError> {
        let key_id = key_id.into();
        self.keys
            .lock()
            .map_err(adapter_error)?
            .insert(key_id.clone(), key);
        *self.active_id.lock().map_err(adapter_error)? = key_id;
        Ok(())
    }
}

impl KeyProvider for StaticKeyProvider {
    fn active_key(&self) -> Result<(String, [u8; 32]), StoreError> {
        let active_id = self.active_id.lock().map_err(adapter_error)?.clone();
        let key = self
            .keys
            .lock()
            .map_err(adapter_error)?
            .get(&active_id)
            .copied()
            .ok_or_else(|| StoreError::KeyUnavailable(active_id.clone()))?;
        Ok((active_id, key))
    }

    fn key_by_id(&self, key_id: &str) -> Result<[u8; 32], StoreError> {
        self.keys
            .lock()
            .map_err(adapter_error)?
            .get(key_id)
            .copied()
            .ok_or_else(|| StoreError::KeyUnavailable(key_id.to_owned()))
    }

    fn store_anchor(&self, anchor: &SecureAnchor) -> Result<(), StoreError> {
        *self.anchor.lock().map_err(adapter_error)? = Some(anchor.clone());
        Ok(())
    }

    fn load_anchor(&self) -> Result<Option<SecureAnchor>, StoreError> {
        Ok(self.anchor.lock().map_err(adapter_error)?.clone())
    }
}

/// Environment-backed encryption key with a separate local secure-anchor file.
pub struct EnvironmentKeyProvider {
    variable: String,
    key_id: String,
    anchor_path: PathBuf,
}

impl EnvironmentKeyProvider {
    /// Configure an explicit environment variable and anchor path.
    pub fn new(
        variable: impl Into<String>,
        key_id: impl Into<String>,
        anchor_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            variable: variable.into(),
            key_id: key_id.into(),
            anchor_path: anchor_path.into(),
        }
    }

    fn read_key(&self) -> Result<[u8; 32], StoreError> {
        let encoded = std::env::var(&self.variable).map_err(|_| {
            StoreError::KeyUnavailable(format!("environment variable {} is unset", self.variable))
        })?;
        let bytes = hex::decode(&encoded)
            .or_else(|_| BASE64.decode(&encoded))
            .map_err(|_| {
                StoreError::KeyUnavailable(format!(
                    "{} must contain 32 bytes encoded as hex or base64",
                    self.variable
                ))
            })?;
        bytes.try_into().map_err(|_| {
            StoreError::KeyUnavailable(format!("{} must decode to exactly 32 bytes", self.variable))
        })
    }
}

impl KeyProvider for EnvironmentKeyProvider {
    fn active_key(&self) -> Result<(String, [u8; 32]), StoreError> {
        Ok((self.key_id.clone(), self.read_key()?))
    }

    fn key_by_id(&self, key_id: &str) -> Result<[u8; 32], StoreError> {
        if key_id != self.key_id {
            return Err(StoreError::KeyUnavailable(format!(
                "historical key {key_id} is not configured"
            )));
        }
        self.read_key()
    }

    fn store_anchor(&self, anchor: &SecureAnchor) -> Result<(), StoreError> {
        if let Some(parent) = self.anchor_path.parent() {
            fs::create_dir_all(parent).map_err(adapter_error)?;
        }
        let temporary = self.anchor_path.with_extension("tmp");
        let body = serde_json::to_vec(anchor).map_err(adapter_error)?;
        fs::write(&temporary, body).map_err(adapter_error)?;
        fs::rename(temporary, &self.anchor_path).map_err(adapter_error)
    }

    fn load_anchor(&self) -> Result<Option<SecureAnchor>, StoreError> {
        if !self.anchor_path.exists() {
            return Ok(None);
        }
        let value: Value =
            serde_json::from_slice(&fs::read(&self.anchor_path).map_err(adapter_error)?)
                .map_err(adapter_error)?;
        decode_anchor(&value).map(Some)
    }
}

/// OS keychain/DPAPI/Secret Service key provider with a separately protected anchor.
pub struct PlatformKeyProvider {
    service: String,
    key_id: String,
}

type PlatformSecretCache = Mutex<BTreeMap<(String, String), [u8; 32]>>;

static PLATFORM_SECRET_CACHE: OnceLock<PlatformSecretCache> = OnceLock::new();

pub(super) fn cached_platform_secret(
    service: &str,
    account: &str,
    load: impl FnOnce() -> Result<[u8; 32], StoreError>,
) -> Result<[u8; 32], StoreError> {
    let cache = PLATFORM_SECRET_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut cache = cache.lock().map_err(adapter_error)?;
    let identity = (service.to_owned(), account.to_owned());
    if let Some(secret) = cache.get(&identity) {
        return Ok(*secret);
    }
    let secret = load()?;
    cache.insert(identity, secret);
    Ok(secret)
}

impl PlatformKeyProvider {
    /// Load or create the active journal key in the platform credential store.
    pub fn new(service: impl Into<String>, key_id: impl Into<String>) -> Result<Self, StoreError> {
        let provider = Self {
            service: service.into(),
            key_id: key_id.into(),
        };
        platform_secret(
            &provider.service,
            &format!("journal-key:{}", provider.key_id),
        )?;
        Ok(provider)
    }

    fn key_account(&self, key_id: &str) -> String {
        format!("journal-key:{key_id}")
    }

    fn anchor_account(&self) -> String {
        format!("journal-anchor:{}", self.key_id)
    }
}

/// Load or create exactly 32 random bytes in the configured platform credential store.
///
/// Material is cached by service and account for this process so replaying a journal does not
/// repeatedly reopen the same protected credential.
pub fn platform_secret(service: &str, account: &str) -> Result<[u8; 32], StoreError> {
    cached_platform_secret(service, account, || {
        let entry = keyring::Entry::new(service, account).map_err(adapter_error)?;
        let secret = match entry.get_secret() {
            Ok(secret) => secret,
            Err(keyring::Error::NoEntry) => {
                let mut secret = [0_u8; 32];
                getrandom::fill(&mut secret).map_err(adapter_error)?;
                entry.set_secret(&secret).map_err(adapter_error)?;
                secret.to_vec()
            }
            Err(error) => return Err(adapter_error(error)),
        };
        secret.try_into().map_err(|_| {
            StoreError::KeyUnavailable(format!(
                "platform credential {service}/{account} is not 32 bytes"
            ))
        })
    })
}

fn platform_existing_secret(service: &str, account: &str) -> Result<[u8; 32], StoreError> {
    cached_platform_secret(service, account, || {
        let entry = keyring::Entry::new(service, account).map_err(adapter_error)?;
        let secret = entry.get_secret().map_err(|error| match error {
            keyring::Error::NoEntry => StoreError::KeyUnavailable(format!(
                "platform credential {service}/{account} is absent"
            )),
            other => adapter_error(other),
        })?;
        secret.try_into().map_err(|_| {
            StoreError::KeyUnavailable(format!(
                "platform credential {service}/{account} is not 32 bytes"
            ))
        })
    })
}

impl KeyProvider for PlatformKeyProvider {
    fn active_key(&self) -> Result<(String, [u8; 32]), StoreError> {
        Ok((
            self.key_id.clone(),
            platform_secret(&self.service, &self.key_account(&self.key_id))?,
        ))
    }

    fn key_by_id(&self, key_id: &str) -> Result<[u8; 32], StoreError> {
        platform_existing_secret(&self.service, &self.key_account(key_id))
    }

    fn store_anchor(&self, anchor: &SecureAnchor) -> Result<(), StoreError> {
        let entry =
            keyring::Entry::new(&self.service, &self.anchor_account()).map_err(adapter_error)?;
        let body = serde_json::to_vec(anchor).map_err(adapter_error)?;
        entry.set_secret(&body).map_err(adapter_error)
    }

    fn load_anchor(&self) -> Result<Option<SecureAnchor>, StoreError> {
        let entry =
            keyring::Entry::new(&self.service, &self.anchor_account()).map_err(adapter_error)?;
        let body = match entry.get_secret() {
            Ok(body) => body,
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(error) => return Err(adapter_error(error)),
        };
        let value: Value = serde_json::from_slice(&body).map_err(adapter_error)?;
        decode_anchor(&value).map(Some)
    }
}

fn decode_anchor(value: &Value) -> Result<SecureAnchor, StoreError> {
    let sequence = value
        .get("sequence")
        .and_then(Value::as_u64)
        .ok_or_else(|| StoreError::Verification("secure anchor has no sequence".into()))?;
    let hash = value
        .get("hash")
        .and_then(Value::as_str)
        .ok_or_else(|| StoreError::Verification("secure anchor has no hash".into()))?;
    let format_version = value
        .get("format_version")
        .and_then(Value::as_u64)
        .map_or(Ok(1_u16), |version| {
            u16::try_from(version).map_err(adapter_error)
        })?;
    let verification_profile = value
        .get("verification_profile")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let status = value
        .get("status")
        .map(|status| serde_json::from_value(status.clone()).map_err(adapter_error))
        .transpose()?
        .unwrap_or_default();
    Ok(SecureAnchor {
        format_version,
        sequence,
        hash: hash.to_owned(),
        verification_profile,
        status,
    })
}
