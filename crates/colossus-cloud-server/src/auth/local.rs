use super::*;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use colossus_cloud::{LocalCredential, hash_identity, identity_key, normalize_username};

pub(super) async fn hash_password(
    permits: Arc<tokio::sync::Semaphore>,
    password: Zeroizing<String>,
) -> CloudResult<String> {
    if password.chars().count() < 15 || password.len() > 1024 {
        return Err(CloudError::InvalidArgument);
    }
    let permit = permits
        .try_acquire_owned()
        .map_err(|_| CloudError::ResourceExhausted)?;
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|_| CloudError::Storage)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let salt = SaltString::encode_b64(&salt).map_err(|_| CloudError::Storage)?;
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|_| CloudError::Storage)
    })
    .await
    .map_err(|_| CloudError::Storage)?
}
impl Authentication {
    pub(crate) async fn password_hash(&self, password: Zeroizing<String>) -> CloudResult<String> {
        hash_password(self.password_permits.clone(), password).await
    }
    pub(crate) async fn local_login(
        &self,
        headers: &HeaderMap,
        username: String,
        password: Zeroizing<String>,
    ) -> CloudResult<HeaderValue> {
        self.csrf(headers)?;
        let config = self
            .config
            .local_auth
            .as_ref()
            .ok_or(CloudError::PermissionDenied)?;
        if !(1..=1024).contains(&password.len()) {
            return Err(CloudError::PermissionDenied);
        }
        let username = normalize_username(&username).map_err(|_| CloudError::PermissionDenied)?;
        self.login_budget(&username).await?;
        let record = self
            .store
            .read(&identity_key(
                EntityKind::LocalCredential,
                &hash_identity(&[&username]),
            ))
            .await
            .ok();
        let credential = record.and_then(|r| {
            serde_json::from_value::<LocalCredential>(r.value)
                .ok()
                .map(|credential| (r.revision, credential))
        });
        let encoded = credential
            .as_ref()
            .map(|(_, c)| c.password_hash.clone())
            .unwrap_or_else(|| self.dummy_password_hash.clone());
        let permit = self
            .password_permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| CloudError::ResourceExhausted)?;
        let verified = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            PasswordHash::new(&encoded).is_ok_and(|hash| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &hash)
                    .is_ok()
            })
        })
        .await
        .map_err(|_| CloudError::PermissionDenied)?;
        let (revision, credential) = credential
            .filter(|_| verified)
            .ok_or(CloudError::PermissionDenied)?;
        let account = self
            .repository()?
            .account(&credential.user_id)
            .await
            .map_err(|_| CloudError::PermissionDenied)?;
        if !account.user.active {
            return Err(CloudError::PermissionDenied);
        }
        // Reconcile after account lookup: old password verification must never
        // acquire the newer account epoch when credential rotation raced with it.
        let current = self
            .store
            .read(&identity_key(
                EntityKind::LocalCredential,
                &hash_identity(&[&username]),
            ))
            .await
            .map_err(|_| CloudError::PermissionDenied)?;
        if current.revision != revision {
            return Err(CloudError::PermissionDenied);
        }
        let session = Nonce::new_random().secret().to_owned();
        let now = crate::http::now();
        let seconds = config.session_seconds;
        self.store
            .put_session(AuthSession {
                session_hash: self.local_session_hash(&session),
                subject: account.user.id,
                csrf_hash: String::new(),
                created_at: now,
                expires_at: now + seconds,
                security_epoch: account.security_epoch,
            })
            .await?;
        self.set_cookie("colossus_session", &session, seconds)
    }
    async fn login_budget(&self, username: &str) -> CloudResult<()> {
        let now = crate::http::now();
        let bucket = (now / 60).to_string();
        // Shared database counters survive replicas; neither remote IP nor proxy headers are trusted.
        for (name, limit) in [
            (hash_identity(&["local-login-global", &bucket]), 120u64),
            (hash_identity(&["local-login", username, &bucket]), 8u64),
        ] {
            self.consume_auth_budget(&name, limit).await?;
        }
        Ok(())
    }
    pub(super) async fn consume_auth_budget(&self, name: &str, limit: u64) -> CloudResult<()> {
        let now = crate::http::now();
        let key = flow_key(name);
        for attempt in 0..8 {
            let (revision, count) = match self.store.read(&key).await {
                Ok(r) => (
                    r.revision,
                    r.value
                        .get("attempts")
                        .and_then(serde_json::Value::as_u64)
                        .ok_or(CloudError::Storage)?,
                ),
                Err(CloudError::NotFound) => (0, 0),
                Err(e) => return Err(e),
            };
            if count >= limit {
                return Err(CloudError::ResourceExhausted);
            }
            match self
                .store
                .commit(CloudTransaction {
                    entities: vec![EntityMutation {
                        key: key.clone(),
                        expected_revision: revision,
                        value: serde_json::json!({"attempts":count+1,"expires_at":now+600}),
                        actor: "browser-authentication".into(),
                        operation: "cloud.auth.attempt.v3".into(),
                    }],
                    ..Default::default()
                })
                .await
            {
                Ok(()) => break,
                Err(colossus_ports::StoreError::Conflict { .. }) if attempt < 7 => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}
