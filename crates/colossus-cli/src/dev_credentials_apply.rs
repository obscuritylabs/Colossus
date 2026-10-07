use super::{
    dev_credentials_plan::{self as plan, Failure, Plan, Result, Source},
    dev_credentials_sources,
    public_api_admin::{self, SecretStore},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use colossus_credentials::{
    DevelopmentAuthority, DevelopmentStoreScope, PlatformCredentialVault, development_journal_key,
    headless_credential_account, seal_existing, seal_existing_record,
};
use colossus_home::{ConfinedFile, ConfinedRoot};
use fs4::fs_std::FileExt as _;
use std::path::Path;
use zeroize::Zeroizing;

struct Material {
    service: String,
    account: String,
    journal: bool,
    kind: MaterialKind,
}
#[derive(Clone, Copy, Eq, PartialEq)]
enum MaterialKind {
    Seed,
    Bearer,
    Anchor,
}

/// Translate reviewed existing selectors into core seal operations. No runtime,
/// enrollment, provider creation, source write/delete or credential issuance occurs.
pub(super) fn apply(plan: &Plan, source_keys: &dyn SecretStore) -> Result<(usize, usize)> {
    apply_with_vault_keys(
        plan,
        source_keys,
        std::sync::Arc::new(colossus_credentials::SystemKeyStore),
    )
}

pub(super) fn apply_with_vault_keys(
    plan: &Plan,
    source_keys: &dyn SecretStore,
    vault_keys: std::sync::Arc<dyn colossus_credentials::PlatformKeyStore>,
) -> Result<(usize, usize)> {
    let authority_root = plan::existing_root(&plan.home.join("development-credentials"))?;
    let operation = authority_root
        .open_file(Path::new("offline-rewrap.lock"))
        .map_err(|_| Failure("development rewrap lease is unavailable"))?;
    if !operation
        .file()
        .try_lock_exclusive()
        .map_err(|_| Failure("development rewrap lease failed"))?
    {
        return Err(Failure("another offline development rewrap is active"));
    }
    if dev_credentials_sources::rebuild(plan)? != *plan {
        return Err(Failure("existing sources changed since the reviewed plan"));
    }
    let home = plan::existing_root(&plan.home)?;
    let authority = DevelopmentAuthority::open(
        &home,
        &DevelopmentAuthority::path_for_home(&home),
        std::slice::from_ref(&plan.workspace),
    )
    .map_err(|_| Failure("prepared development authority is unavailable"))?;
    let _source_leases = source_leases(plan)?;
    if plan
        .sources
        .iter()
        .any(|source| matches!(source, Source::ConnectorEnrollment { .. }))
    {
        plan::record_copy_plan(plan)?;
    }
    let materials = materials(plan)?;
    // Read every small platform source before the first target write. Missing or
    // malformed values do not become permission to generate a new authority.
    let mut originals = Vec::with_capacity(materials.len());
    for material in &materials {
        let original = source_keys
            .read(&material.service, &material.account)
            .map_err(|_| Failure("an existing platform source is unavailable"))?
            .ok_or(Failure(
                "an existing platform source is missing; no replacement was generated",
            ))?;
        validate_material(&material.kind, &original)?;
        originals.push(original);
    }
    let mut copied = 0;
    let mut reused = 0;
    let mut held_vaults = Vec::new();
    let mut source_guards = Vec::new();
    for source in &plan.sources {
        if let Source::ConnectorEnrollment {
            source_home,
            name,
            metadata,
        } = source
        {
            let root = plan::existing_root(&source_home.join("cloud-connector"))?;
            let original = PlatformCredentialVault::with_key_store(
                root,
                "cloud-connector",
                std::sync::Arc::clone(&vault_keys),
            )
            .map_err(|_| Failure("selected source enrollment vault could not be bound"))?;
            source_guards.push(original.source_guard().map_err(|_| {
                Failure("selected source enrollment custody could not be retained")
            })?);
            let target_directory = home
                .prepare_directory(Path::new("cloud-connector"))
                .map_err(|_| Failure("isolated connector target could not be prepared"))?;
            let target_root = plan::existing_root(&target_directory)?;
            let target_keys = authority
                .store(DevelopmentStoreScope::ControlPlaneVault)
                .map_err(|_| Failure("isolated connector key custody is unavailable"))?;
            let target = PlatformCredentialVault::with_key_store(
                target_root,
                "cloud-connector",
                std::sync::Arc::new(target_keys),
            )
            .map_err(|_| Failure("isolated connector vault could not be bound"))?;
            let key = colossus_connector::EnrollmentStore::credential_key(name)
                .map_err(|_| Failure("named enrollment selector is invalid"))?;
            let created = original
                .copy_existing_record(&target, &key, metadata)
                .map_err(|_| Failure("exact source enrollment could not be copied and verified"))?;
            if created {
                copied += 1;
            } else {
                reused += 1;
            }
            held_vaults.push(original);
            held_vaults.push(target);
        }
    }
    for source in &plan.sources {
        let (path, owner, scope, metadata) = match source {
            Source::DesktopVault { metadata } => (
                plan.home.join("desktop"),
                "desktop-manual",
                DevelopmentStoreScope::DesktopVault,
                metadata,
            ),
            Source::ControlPlaneVault { metadata } => (
                plan.home.join("cloud-connector"),
                "cloud-connector",
                DevelopmentStoreScope::ControlPlaneVault,
                metadata,
            ),
            Source::RuntimeOAuth {
                directory,
                owner_scope,
                metadata,
                ..
            } => (
                directory.clone(),
                owner_scope.as_str(),
                DevelopmentStoreScope::RuntimeOAuthVault,
                metadata,
            ),
            _ => continue,
        };
        let root = plan::existing_root(&path)?;
        let vault = PlatformCredentialVault::with_key_store(
            root,
            owner,
            std::sync::Arc::clone(&vault_keys),
        )
        .map_err(|_| Failure("existing vault could not be bound"))?;
        source_guards.push(
            vault
                .source_guard()
                .map_err(|_| Failure("existing vault source custody could not be retained"))?,
        );
        let target = authority
            .store(scope)
            .map_err(|_| Failure("prepared sealed target is unavailable"))?;
        let created = vault.rewrap_key(&target, metadata).map_err(|_| {
            Failure("existing vault identity or verified key could not be preserved")
        })?;
        if created {
            copied += 1;
        } else {
            reused += 1;
        }
        held_vaults.push(vault);
    }
    let public_api = authority
        .store(DevelopmentStoreScope::PublicApi)
        .map_err(|_| Failure("prepared public API target is unavailable"))?;
    let journal = materials
        .iter()
        .any(|material| material.journal)
        .then(|| authority.journal_vault())
        .transpose()
        .map_err(|_| Failure("prepared encrypted journal custody is unavailable"))?;
    for (material, original) in materials.iter().zip(&originals) {
        let created = if material.journal {
            let key = development_journal_key(&material.service, &material.account)
                .map_err(|_| Failure("journal custody reference is invalid"))?;
            let vault = journal
                .as_ref()
                .ok_or(Failure("journal custody is unavailable"))?;
            seal_existing_record(vault.as_ref(), &key, original)
                .map_err(|_| Failure("original journal custody could not be sealed and verified"))?
        } else {
            seal_existing(
                &public_api,
                &headless_credential_account(&material.service, &material.account),
                original,
            )
            .map_err(|_| Failure("original application custody could not be sealed and verified"))?
        };
        if created {
            copied += 1;
        } else {
            reused += 1;
        }
    }
    // Revalidate frozen journal/config fingerprints after all copies and before
    // activation. Vault source leases remain held until this function returns.
    for source in &plan.sources {
        match source {
            Source::Journal {
                journal,
                journal_sha256,
                config,
                config_sha256,
                ..
            }
            | Source::PlaintextJournal {
                journal,
                journal_sha256,
                config,
                config_sha256,
            } => {
                if plan::file_digest(journal)? != *journal_sha256
                    || plan::sha256(&plan::read_private(config, 1024 * 1024)?.1) != *config_sha256
                {
                    return Err(Failure("journal source changed during offline rewrap"));
                }
            }
            Source::Client {
                config,
                config_sha256,
                ..
            } => {
                if plan::sha256(&plan::read_private(config, 16 * 1024)?.1) != *config_sha256 {
                    return Err(Failure("client source changed during offline rewrap"));
                }
            }
            Source::PublicApi {
                directory,
                certificate_file_sha256: Some(expected),
                ..
            } if plan::sha256(
                &plan::read_private(&directory.join("certificate.pem"), 16 * 1024)?.1,
            ) != *expected =>
            {
                return Err(Failure("public API source changed during offline rewrap"));
            }
            _ => {}
        }
    }
    for (root, file) in &_source_leases {
        file.revalidate(root)
            .map_err(|_| Failure("source identity changed during offline rewrap"))?;
    }
    if plan.fresh_empty {
        dev_credentials_sources::validate_fresh_home(&plan.home)?;
    }
    for guard in &source_guards {
        guard
            .revalidate()
            .map_err(|_| Failure("vault source changed before offline activation"))?;
    }
    authority
        .activate(&home)
        .map_err(|_| Failure("verified development custody could not be activated"))?;
    operation
        .revalidate(&authority_root)
        .map_err(|_| Failure("development rewrap identity changed"))?;
    drop(held_vaults);
    Ok((copied, reused))
}

fn source_leases(plan: &Plan) -> Result<Vec<(ConfinedRoot, ConfinedFile)>> {
    let mut guards = Vec::new();
    let mut directories = std::collections::BTreeSet::new();
    for source in &plan.sources {
        match source {
            Source::PublicApi { directory, .. } => {
                directories.insert(directory.clone());
            }
            Source::Client { config, .. } => {
                #[derive(serde::Deserialize)]
                struct DescriptorReference {
                    descriptor: std::path::PathBuf,
                }
                let (_, bytes) = plan::read_private(config, 16 * 1024)?;
                let reference: DescriptorReference = serde_json::from_slice(&bytes)
                    .map_err(|_| Failure("client source reference changed"))?;
                directories.insert(
                    reference
                        .descriptor
                        .parent()
                        .ok_or(Failure("client API source root is invalid"))?
                        .to_owned(),
                );
            }
            Source::Journal {
                journal,
                journal_sha256,
                ..
            }
            | Source::PlaintextJournal {
                journal,
                journal_sha256,
                ..
            } => {
                let (root, file) = plan::existing_file(journal)?;
                if !file
                    .file()
                    .try_lock_exclusive()
                    .map_err(|_| Failure("journal source lease failed"))?
                {
                    return Err(Failure(
                        "stop the source journal writer before rewrapping custody",
                    ));
                }
                if plan::file_digest(journal)? != *journal_sha256 {
                    return Err(Failure("journal source changed before its offline lease"));
                }
                file.revalidate(&root)
                    .map_err(|_| Failure("journal source identity changed"))?;
                guards.push((root, file));
            }
            _ => {}
        }
    }
    for directory in directories {
        let root = plan::existing_root(&directory)?;
        let file = root
            .open_existing_file_read_write(Path::new(".public-api.lock"))
            .map_err(|_| Failure("existing public API source lease is unavailable"))?;
        if !file
            .file()
            .try_lock_exclusive()
            .map_err(|_| Failure("public API source lease failed"))?
        {
            return Err(Failure(
                "stop the source public API before rewrapping custody",
            ));
        }
        file.revalidate(&root)
            .map_err(|_| Failure("public API source identity changed"))?;
        guards.push((root, file));
    }
    Ok(guards)
}

fn materials(plan: &Plan) -> Result<Vec<Material>> {
    let mut materials = Vec::new();
    let mut references = std::collections::BTreeMap::new();
    for source in &plan.sources {
        let mut add =
            |service: &str, account: String, journal: bool, kind: MaterialKind| -> Result<()> {
                let reference = (service.to_owned(), account.clone());
                if let Some(previous) = references.get(&reference) {
                    if *previous != (journal, kind) {
                        return Err(Failure(
                            "source selectors disagree about the same credential material",
                        ));
                    }
                } else {
                    references.insert(reference, (journal, kind));
                    materials.push(Material {
                        service: service.to_owned(),
                        account,
                        journal,
                        kind,
                    });
                }
                Ok(())
            };
        match source {
            Source::PublicApi { service, .. } => {
                for account in [
                    public_api_admin::AUTHENTICATION_ROOT_ACCOUNT,
                    public_api_admin::TLS_SEED_ACCOUNT,
                    public_api_admin::INSTANCE_SEED_ACCOUNT,
                ] {
                    add(service, account.into(), false, MaterialKind::Seed)?;
                }
            }
            Source::Client {
                service, account, ..
            } => add(service, account.clone(), false, MaterialKind::Bearer)?,
            Source::Journal {
                service,
                encryption_key_ids,
                signing_key_ids,
                anchor_key_id,
                ..
            } => {
                for id in encryption_key_ids {
                    add(
                        service,
                        format!("journal-key:{id}"),
                        true,
                        MaterialKind::Seed,
                    )?;
                }
                for id in signing_key_ids {
                    add(
                        service,
                        format!("signing-key:{id}"),
                        true,
                        MaterialKind::Seed,
                    )?;
                }
                add(
                    service,
                    format!("journal-anchor:{anchor_key_id}"),
                    true,
                    MaterialKind::Anchor,
                )?;
            }
            _ => {}
        }
    }
    if materials.len() > 1024 {
        return Err(Failure("source material selection exceeds its bound"));
    }
    Ok(materials)
}

fn validate_material(kind: &MaterialKind, bytes: &[u8]) -> Result<()> {
    match kind {
        MaterialKind::Seed if bytes.len() == 32 => Ok(()),
        MaterialKind::Bearer => {
            if bytes.len() > 256 {
                return Err(Failure(
                    "existing application bearer exceeds its supported bound",
                ));
            }
            let token = std::str::from_utf8(bytes)
                .map_err(|_| Failure("existing application bearer is invalid"))?;
            let mut parts = token.split('.');
            if parts.next() != Some("cls_v1") {
                return Err(Failure(
                    "source reference is not an existing Colossus application bearer",
                ));
            }
            uuid::Uuid::parse_str(
                parts
                    .next()
                    .ok_or(Failure("existing application bearer is invalid"))?,
            )
            .map_err(|_| Failure("existing application bearer is invalid"))?;
            let mut decoded = Zeroizing::new([0_u8; 32]);
            let encoded = parts
                .next()
                .ok_or(Failure("existing application bearer is invalid"))?;
            if parts.next().is_some()
                || URL_SAFE_NO_PAD
                    .decode_slice(encoded, decoded.as_mut())
                    .map_err(|_| Failure("existing application bearer is invalid"))?
                    != 32
            {
                return Err(Failure("existing application bearer is invalid"));
            }
            Ok(())
        }
        MaterialKind::Anchor => colossus_journal_redb::decode_secure_anchor(bytes)
            .map(|_| ())
            .map_err(|_| Failure("existing journal anchor is malformed or exceeds its bound")),
        _ => Err(Failure("existing platform seed has an invalid size")),
    }
}

#[cfg(test)]
pub(super) fn validate_test_bearer(bytes: &[u8]) -> Result<()> {
    validate_material(&MaterialKind::Bearer, bytes)
}
