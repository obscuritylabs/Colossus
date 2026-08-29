use super::*;

/// Canonical storage adapters and diagnostics constructed as one startup unit.
pub(super) struct StorageComposition {
    pub(super) keys: Arc<dyn KeyProvider>,
    pub(super) writer_lease: Option<RedbWriterLease>,
    pub(super) journal: Arc<dyn EventJournal>,
    pub(super) projections: Arc<dyn ProjectionStore>,
    pub(super) recovery_reason: Option<String>,
    pub(super) diagnostic: Value,
}

pub(super) fn compose_storage(
    config: &RuntimeConfig,
    storage_path: &Path,
    tls_roots: &AdditionalRootCertificates,
) -> Result<StorageComposition, RuntimeError> {
    if config.storage.adapter != StorageAdapter::Ephemeral
        && !config.has_resolved_home_workspace()
        && let Some(parent) = storage_path.parent()
    {
        fs::create_dir_all(parent)?;
    }
    let (keys, signer): (Arc<dyn KeyProvider>, Arc<dyn CheckpointSigner>) =
        match &config.storage.keys {
            KeyConfig::None => (
                Arc::new(PlaintextKeyProvider),
                Arc::new(DisabledCheckpointSigner),
            ),
            KeyConfig::Platform {
                service,
                journal_key_id,
                signing_key_id,
            } => {
                let signing_key =
                    platform_secret(service, &format!("signing-key:{signing_key_id}"))?;
                (
                    Arc::new(PlatformKeyProvider::new(service, journal_key_id)?),
                    Arc::new(Ed25519CheckpointSigner::new(
                        signing_key_id.clone(),
                        signing_key,
                    )),
                )
            }
            KeyConfig::Environment {
                journal_variable,
                journal_key_id,
                signing_variable,
                anchor_path,
            } => {
                config.revalidate_resolved_home_file(anchor_path)?;
                config.revalidate_resolved_home_file(&anchor_path.with_extension("tmp"))?;
                let signing_key = explicit_secret(signing_variable)?;
                (
                    Arc::new(EnvironmentKeyProvider::new(
                        journal_variable,
                        journal_key_id,
                        anchor_path,
                    )),
                    Arc::new(Ed25519CheckpointSigner::new(
                        "environment-checkpoint-v1",
                        signing_key,
                    )),
                )
            }
        };
    Ok(match config.storage.adapter {
        StorageAdapter::Ephemeral => {
            let redb = Arc::new(RedbEventJournal::open_in_memory_with_startup_verification(
                Arc::clone(&keys),
                signer.clone(),
                config.storage.startup_verification,
            )?);
            let recovery_reason = redb.recovery_reason()?;
            let startup_verification = redb.startup_verification_report()?;
            StorageComposition {
                keys: Arc::clone(&keys),
                writer_lease: None,
                journal: redb.clone(),
                projections: redb,
                recovery_reason,
                diagnostic: json!({
                    "adapter": "ephemeral",
                    "path": null,
                    "instance_identity": storage_path,
                    "persistence": "process",
                    "payload_protection": config.storage.keys.protection_label(),
                    "startup_verification": startup_verification,
                }),
            }
        }
        StorageAdapter::Redb => {
            let mut lock_path = storage_path.as_os_str().to_os_string();
            lock_path.push(".writer.lock");
            let lock_path = PathBuf::from(lock_path);
            let (lease, redb) = match (
                config.open_resolved_home_file(&lock_path)?,
                config.open_resolved_home_file(storage_path)?,
            ) {
                (Some(lock), Some(state)) => {
                    let lease =
                        RedbWriterLease::acquire_file(lock.path().to_owned(), lock.into_file())?;
                    let redb = RedbEventJournal::open_file_with_startup_verification(
                        state.into_file(),
                        Arc::clone(&keys),
                        signer.clone(),
                        config.storage.startup_verification,
                    )?;
                    (lease, Arc::new(redb))
                }
                (None, None) => {
                    let lease = RedbWriterLease::acquire(storage_path)?;
                    let redb = RedbEventJournal::open_with_startup_verification(
                        storage_path,
                        Arc::clone(&keys),
                        signer.clone(),
                        config.storage.startup_verification,
                    )?;
                    (lease, Arc::new(redb))
                }
                _ => {
                    return Err(RuntimeError::Config(
                        "home-workspace storage authority is inconsistent".into(),
                    ));
                }
            };
            let recovery_reason = redb.recovery_reason()?;
            let startup_verification = redb.startup_verification_report()?;
            StorageComposition {
                keys: Arc::clone(&keys),
                writer_lease: Some(lease),
                journal: redb.clone(),
                projections: redb,
                recovery_reason,
                diagnostic: json!({
                    "adapter": "redb",
                    "path": storage_path,
                    "payload_protection": config.storage.keys.protection_label(),
                    "startup_verification": startup_verification,
                }),
            }
        }
        StorageAdapter::Postgres => {
            let postgres_config = config.storage.postgres.clone().ok_or_else(|| {
                RuntimeError::Config(
                    "storage.postgres is required when storage.adapter is postgres".into(),
                )
            })?;
            let postgres = Arc::new(
                PostgresEventJournal::open_with_tls_roots_and_startup_verification(
                    postgres_config,
                    Arc::clone(&keys),
                    signer,
                    tls_roots,
                    config.storage.startup_verification,
                )?,
            );
            let recovery_reason = postgres.recovery_reason()?;
            let mut diagnostic = postgres.diagnostic();
            diagnostic["startup_verification"] = json!(postgres.startup_verification_report()?);
            diagnostic["payload_protection"] = json!(config.storage.keys.protection_label());
            StorageComposition {
                keys,
                writer_lease: None,
                journal: postgres.clone(),
                projections: postgres,
                recovery_reason,
                diagnostic,
            }
        }
    })
}
