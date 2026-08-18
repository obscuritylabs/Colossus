use colossus_api::{ApiScope, ApplicationKind, scopes};
use colossus_grpc::ApplicationGrant;
#[cfg(unix)]
use colossus_grpc::{TlsIdentity, TlsKeySeed};
use colossus_worker::{
    PublicApiAuthenticationKey, PublicApiCredentialManager, PublicApiHostOptions, WorkerServer,
};
#[cfg(unix)]
use fs4::fs_std::FileExt as _;
use serde::Serialize;
#[cfg(unix)]
use sha2::{Digest as _, Sha256};
use std::{
    error::Error,
    fmt,
    fs::File,
    path::{Path, PathBuf},
};
use uuid::Uuid;
use zeroize::Zeroizing;

#[cfg(unix)]
use std::{
    fs,
    net::{Ipv4Addr, SocketAddr},
    os::unix::fs::{DirBuilderExt as _, MetadataExt as _},
};

#[cfg(unix)]
const DIRECTORY_MODE: u32 = 0o700;
#[cfg(unix)]
const LOCK_FILE_MODE: u32 = 0o600;
#[cfg(unix)]
const DESCRIPTOR_FILENAME: &str = "endpoint.json";
#[cfg(unix)]
const CERTIFICATE_FILENAME: &str = "certificate.pem";
#[cfg(unix)]
const LOCK_FILENAME: &str = ".public-api.lock";
#[cfg(unix)]
const KEYRING_SERVICE_PREFIX: &str = "dev.obscuritylabs.colossus.public-api";
const DESKTOP_EXTERNAL_KEYRING_SERVICE: &str = "com.obscuritylabs.colossus.desktop.external";
const DESKTOP_BOUND_ACCOUNT_REQUEST: &str = "auto";
#[cfg(unix)]
const AUTHENTICATION_ROOT_ACCOUNT: &str = "authentication-root-v1";
#[cfg(unix)]
const TLS_SEED_ACCOUNT: &str = "tls-seed-v1";
#[cfg(unix)]
const INSTANCE_SEED_ACCOUNT: &str = "instance-identity-seed-v1";
#[cfg(unix)]
const INSTANCE_ID_DOMAIN: &[u8] = b"colossus-public-api-instance-id-v1\0";

const KNOWN_SCOPES: [&str; 5] = [
    scopes::RUNS_EXECUTE,
    scopes::RUNS_READ,
    scopes::RUNS_CONTROL,
    scopes::PROMPTS_RESPOND,
    scopes::APPROVALS_RESPOND,
];
const UNSUPPORTED_PUBLIC_TOOLS: [&str; 1] = ["agent.delegate"];

/// Sanitized public API administration failure.
///
/// Errors deliberately omit keyring backend detail because such implementations are
/// not part of the stable CLI contract and must never be allowed to include secret
/// material.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PublicApiAdminError {
    #[cfg(not(unix))]
    UnsupportedPlatform,
    InvalidDirectory,
    #[cfg(unix)]
    DirectoryBusy,
    SecretStoreUnavailable,
    #[cfg(unix)]
    SecretStoreValueInvalid,
    InvalidKeyringIdentifier,
    ReservedKeyringNamespace,
    DestinationExists,
    InvalidRotationSource,
    InvalidScope,
    InvalidGrant,
    InvalidCredentialId,
    CredentialDeliveryFailed,
    CredentialDeliveryAndRevocationFailed {
        credential_id: String,
    },
    CredentialActivationFailed,
    CredentialActivationCompensationFailed {
        credential_id: String,
    },
    CredentialRotationRevocationUnconfirmed {
        previous_credential_id: String,
        new_credential_id: String,
    },
    CredentialMigrationRevocationUnconfirmed {
        previous_credential_id: String,
        new_credential_id: String,
    },
    CredentialRetirementCleanupUnconfirmed {
        previous_credential_id: String,
        new_credential_id: String,
    },
    WorkerUnavailable,
}

impl fmt::Display for PublicApiAdminError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            #[cfg(not(unix))]
            Self::UnsupportedPlatform => {
                "native owner-only public API administration is unsupported on this platform"
            }
            Self::InvalidDirectory => {
                "public API directory must be an absolute current-user 0700 directory"
            }
            #[cfg(unix)]
            Self::DirectoryBusy => "public API directory is already owned by another process",
            Self::SecretStoreUnavailable => "OS credential store operation failed",
            #[cfg(unix)]
            Self::SecretStoreValueInvalid => {
                "OS credential store contains invalid public API key material"
            }
            Self::InvalidKeyringIdentifier => "credential keyring identifier is invalid",
            Self::ReservedKeyringNamespace => {
                "credential destination must not use the Colossus public API key namespace"
            }
            Self::DestinationExists => {
                "credential destination already exists; pass --replace-credential explicitly"
            }
            Self::InvalidRotationSource => {
                "existing credential is not an active credential for this application and API"
            }
            Self::InvalidScope => "public API scope is unknown",
            Self::InvalidGrant => "public API application grant is invalid",
            Self::InvalidCredentialId => "public API credential identifier is invalid",
            Self::CredentialDeliveryFailed => {
                "credential delivery failed; the newly issued credential was revoked"
            }
            Self::CredentialDeliveryAndRevocationFailed { credential_id } => {
                return write!(
                    formatter,
                    "credential delivery failed and revocation could not be confirmed; reconcile credential {credential_id}"
                );
            }
            Self::CredentialActivationFailed => {
                "credential activation failed; the pending credential was revoked and its destination was restored"
            }
            Self::CredentialActivationCompensationFailed { credential_id } => {
                return write!(
                    formatter,
                    "credential activation failed and compensation could not be confirmed; reconcile credential {credential_id}"
                );
            }
            Self::CredentialRotationRevocationUnconfirmed {
                previous_credential_id,
                new_credential_id,
            } => {
                return write!(
                    formatter,
                    "credential rotation activated new credential {new_credential_id}, but revocation of prior credential {previous_credential_id} could not be confirmed; the new credential remains active at the destination; reconcile and explicitly revoke prior credential {previous_credential_id}"
                );
            }
            Self::CredentialMigrationRevocationUnconfirmed {
                previous_credential_id,
                new_credential_id,
            } => {
                return write!(
                    formatter,
                    "credential migration activated new credential {new_credential_id}, but revocation of prior credential {previous_credential_id} could not be confirmed; the new credential remains active and the source keyring entry was not deleted; reconcile prior credential {previous_credential_id}; delete the source value only if it is still credential {previous_credential_id} and its revocation is confirmed"
                );
            }
            Self::CredentialRetirementCleanupUnconfirmed {
                previous_credential_id,
                new_credential_id,
            } => {
                return write!(
                    formatter,
                    "credential migration activated new credential {new_credential_id} and revoked prior credential {previous_credential_id}, but the source keyring entry could not be safely verified and deleted; the new credential remains active; reconcile the source selector and do not delete its current value unless it is confirmed to be credential {previous_credential_id}"
                );
            }
            Self::WorkerUnavailable => "public API worker composition failed",
        };
        formatter.write_str(message)
    }
}

impl Error for PublicApiAdminError {}

/// Minimal secret-store boundary used by production keyring access and unit tests.
pub(super) trait SecretStore {
    fn read(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, PublicApiAdminError>;

    fn write(&self, service: &str, account: &str, secret: &[u8])
    -> Result<(), PublicApiAdminError>;

    fn delete(&self, service: &str, account: &str) -> Result<(), PublicApiAdminError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct OsCredentialStore;

impl SecretStore for OsCredentialStore {
    fn read(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, PublicApiAdminError> {
        let entry = keyring::Entry::new(service, account)
            .map_err(|_| PublicApiAdminError::SecretStoreUnavailable)?;
        match entry.get_secret() {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(PublicApiAdminError::SecretStoreUnavailable),
        }
    }

    fn write(
        &self,
        service: &str,
        account: &str,
        secret: &[u8],
    ) -> Result<(), PublicApiAdminError> {
        let entry = keyring::Entry::new(service, account)
            .map_err(|_| PublicApiAdminError::SecretStoreUnavailable)?;
        entry
            .set_secret(secret)
            .map_err(|_| PublicApiAdminError::SecretStoreUnavailable)
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), PublicApiAdminError> {
        let entry = keyring::Entry::new(service, account)
            .map_err(|_| PublicApiAdminError::SecretStoreUnavailable)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(PublicApiAdminError::SecretStoreUnavailable),
        }
    }
}

/// Locked, canonical owner-private public API directory and its stable secret material.
pub(super) struct PublicApiEnvironment {
    directory: PathBuf,
    namespace_service: String,
    authentication_root: Zeroizing<[u8; 32]>,
    #[cfg(unix)]
    tls_seed: Zeroizing<[u8; 32]>,
    #[cfg(unix)]
    instance_seed: Zeroizing<[u8; 32]>,
    _lease: File,
    #[cfg(unix)]
    directory_device: u64,
    #[cfg(unix)]
    directory_inode: u64,
}

impl PublicApiEnvironment {
    pub(super) fn open(path: &Path, store: &dyn SecretStore) -> Result<Self, PublicApiAdminError> {
        #[cfg(not(unix))]
        {
            let _ = (path, store);
            Err(PublicApiAdminError::UnsupportedPlatform)
        }

        #[cfg(unix)]
        {
            let directory = prepare_owner_private_directory(path)?;
            let lease = acquire_directory_lease(&directory)?;
            let directory_metadata = owner_private_directory_metadata(&directory)?;
            let directory_device = directory_metadata.dev();
            let directory_inode = directory_metadata.ino();
            let namespace_service = namespace_service(&directory);
            let authentication_root =
                load_or_create_exact_key(store, &namespace_service, AUTHENTICATION_ROOT_ACCOUNT)?;
            let tls_seed = load_or_create_exact_key(store, &namespace_service, TLS_SEED_ACCOUNT)?;
            let instance_seed =
                load_or_create_exact_key(store, &namespace_service, INSTANCE_SEED_ACCOUNT)?;
            revalidate_exact_directory(&directory, directory_device, directory_inode)?;
            Ok(Self {
                directory,
                namespace_service,
                authentication_root,
                tls_seed,
                instance_seed,
                _lease: lease,
                directory_device,
                directory_inode,
            })
        }
    }

    pub(super) fn credential_manager(&self, server: &WorkerServer) -> PublicApiCredentialManager {
        server.public_api_credential_manager(PublicApiAuthenticationKey::new(
            *self.authentication_root,
        ))
    }

    pub(super) fn host_options(
        &self,
        credentials: &PublicApiCredentialManager,
    ) -> Result<PublicApiHostOptions, PublicApiAdminError> {
        #[cfg(not(unix))]
        {
            let _ = credentials;
            Err(PublicApiAdminError::UnsupportedPlatform)
        }

        #[cfg(unix)]
        {
            revalidate_exact_directory(
                &self.directory,
                self.directory_device,
                self.directory_inode,
            )?;
            let tls = self.tls_identity()?;
            PublicApiHostOptions::new(
                SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
                stable_instance_id(&self.instance_seed),
                self.directory.join(DESCRIPTOR_FILENAME),
                self.directory.join(CERTIFICATE_FILENAME),
                tls,
                credentials,
            )
            .map_err(|_| PublicApiAdminError::WorkerUnavailable)
        }
    }

    fn public_identity(&self) -> Result<(Uuid, String), PublicApiAdminError> {
        #[cfg(not(unix))]
        {
            Err(PublicApiAdminError::UnsupportedPlatform)
        }

        #[cfg(unix)]
        {
            let tls = self.tls_identity()?;
            Ok((
                stable_instance_id(&self.instance_seed),
                tls.certificate_sha256().to_owned(),
            ))
        }
    }

    #[cfg(unix)]
    fn tls_identity(&self) -> Result<TlsIdentity, PublicApiAdminError> {
        TlsIdentity::from_seed(TlsKeySeed::new(*self.tls_seed))
            .map_err(|_| PublicApiAdminError::WorkerUnavailable)
    }

    fn rejects_destination_service(&self, service: &str) -> bool {
        service == self.namespace_service
    }

    pub(super) fn directory(&self) -> &Path {
        &self.directory
    }
}

impl fmt::Debug for PublicApiEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublicApiEnvironment")
            .field("directory", &self.directory)
            .field("namespace_service", &self.namespace_service)
            .field("authentication_root", &"[REDACTED]")
            .field("tls_seed", &"[REDACTED]")
            .field("instance_seed", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Serialize)]
pub(super) struct EnrollmentMetadata {
    pub(super) application_id: String,
    pub(super) credential_id: String,
    pub(super) instance_id: String,
    pub(super) certificate_sha256: String,
    pub(super) scopes: Vec<String>,
    pub(super) allowed_roles: Vec<String>,
    pub(super) allowed_tools: Vec<String>,
    pub(super) credential_keyring_service: String,
    pub(super) credential_keyring_account: String,
    pub(super) replaced_destination: bool,
    pub(super) revoked_credential_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct RevocationMetadata {
    pub(super) credential_id: String,
    pub(super) revoked: bool,
}

pub(super) struct EnrollmentRequest<'a> {
    pub(super) application_id: &'a str,
    pub(super) scopes: &'a [String],
    pub(super) roles: &'a [String],
    pub(super) tools: &'a [String],
    pub(super) destination_service: &'a str,
    pub(super) destination_account: &'a str,
    pub(super) replace_destination: bool,
    pub(super) retirement_source: Option<CredentialRetirementSource<'a>>,
}

pub(super) struct CredentialRetirementSource<'a> {
    pub(super) service: &'a str,
    pub(super) account: &'a str,
}

pub(super) fn enroll_application(
    environment: &PublicApiEnvironment,
    server: &WorkerServer,
    destination: &dyn SecretStore,
    request: EnrollmentRequest<'_>,
) -> Result<EnrollmentMetadata, PublicApiAdminError> {
    validate_keyring_identifier(request.destination_service)?;
    validate_keyring_identifier(request.destination_account)?;
    if environment.rejects_destination_service(request.destination_service) {
        return Err(PublicApiAdminError::ReservedKeyringNamespace);
    }
    if request.replace_destination && request.retirement_source.is_some() {
        return Err(PublicApiAdminError::InvalidRotationSource);
    }

    if request.scopes.is_empty() || request.roles.is_empty() {
        return Err(PublicApiAdminError::InvalidGrant);
    }
    let (instance_id, certificate_sha256) = environment.public_identity()?;
    let destination_account = bound_destination_account(
        request.destination_service,
        request.destination_account,
        instance_id,
        &certificate_sha256,
    );
    validate_keyring_identifier(&destination_account)?;
    if let Some(source) = request.retirement_source.as_ref() {
        validate_keyring_identifier(source.service)?;
        validate_keyring_identifier(source.account)?;
        if environment.rejects_destination_service(source.service) {
            return Err(PublicApiAdminError::ReservedKeyringNamespace);
        }
        if source.service == request.destination_service && source.account == destination_account {
            return Err(PublicApiAdminError::InvalidRotationSource);
        }
    }
    let exact_scopes = normalize_scopes(request.scopes)?;
    let mut roles = request.roles.to_vec();
    roles.sort();
    roles.dedup();
    let tools = normalize_tool_ceiling(request.tools)?;

    let grant = ApplicationGrant::new(
        request.application_id,
        ApplicationKind::Enrolled,
        exact_scopes.clone(),
        roles.clone(),
        tools.clone(),
    )
    .map_err(|_| PublicApiAdminError::InvalidGrant)?;
    let manager = environment.credential_manager(server);
    let prepared_destination = prepare_destination(
        destination,
        request.destination_service,
        &destination_account,
        request.replace_destination,
        |bearer| {
            manager
                .validate_rotation_source(bearer, request.application_id)
                .map_err(|_| ())
        },
    )?;
    let retirement_source = request
        .retirement_source
        .as_ref()
        .map(|source| {
            prepare_retirement_source(destination, source.service, source.account, |bearer| {
                manager
                    .validate_rotation_source(bearer, request.application_id)
                    .map_err(|_| ())
            })
        })
        .transpose()?;
    let issued = manager
        .issue_pending(&grant)
        .map_err(|_| PublicApiAdminError::WorkerUnavailable)?;
    let credential_id = issued.credential_id().to_owned();
    let replaced_destination = prepared_destination.previous.is_some();
    let revoked_credential_id = if let Some(retirement_source) = retirement_source {
        debug_assert!(prepared_destination.previous.is_none());
        Some(install_migrated_credential(
            destination,
            request.destination_service,
            &destination_account,
            &credential_id,
            issued.expose_token().as_bytes(),
            retirement_source,
            |candidate| manager.activate(candidate),
            |candidate| manager.revoke(candidate),
        )?)
    } else {
        install_pending_credential(
            destination,
            request.destination_service,
            &destination_account,
            &credential_id,
            issued.expose_token().as_bytes(),
            prepared_destination.previous,
            |candidate| manager.activate(candidate),
            |candidate| manager.revoke(candidate),
        )?
    };

    Ok(EnrollmentMetadata {
        application_id: request.application_id.to_owned(),
        credential_id,
        instance_id: instance_id.to_string(),
        certificate_sha256,
        scopes: exact_scopes
            .into_iter()
            .map(|scope| scope.as_str().to_owned())
            .collect(),
        allowed_roles: roles,
        allowed_tools: tools,
        credential_keyring_service: request.destination_service.to_owned(),
        credential_keyring_account: destination_account,
        replaced_destination,
        revoked_credential_id,
    })
}

fn bound_destination_account(
    service: &str,
    account: &str,
    instance_id: Uuid,
    certificate_sha256: &str,
) -> String {
    if service == DESKTOP_EXTERNAL_KEYRING_SERVICE && account == DESKTOP_BOUND_ACCOUNT_REQUEST {
        format!("daemon-{instance_id}-{certificate_sha256}")
    } else {
        account.to_owned()
    }
}

pub(super) fn revoke_credential(
    environment: &PublicApiEnvironment,
    server: &WorkerServer,
    credential_id: &str,
) -> Result<RevocationMetadata, PublicApiAdminError> {
    let parsed =
        Uuid::parse_str(credential_id).map_err(|_| PublicApiAdminError::InvalidCredentialId)?;
    if parsed.to_string() != credential_id {
        return Err(PublicApiAdminError::InvalidCredentialId);
    }
    let manager = environment.credential_manager(server);
    let revoked = manager
        .revoke(credential_id)
        .map_err(|_| PublicApiAdminError::WorkerUnavailable)?;
    Ok(RevocationMetadata {
        credential_id: credential_id.to_owned(),
        revoked,
    })
}

struct PreviousCredential {
    credential_id: String,
    bearer: Zeroizing<Vec<u8>>,
}

impl fmt::Debug for PreviousCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreviousCredential")
            .field("credential_id", &self.credential_id)
            .field("bearer", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug)]
struct PreparedDestination {
    previous: Option<PreviousCredential>,
}

struct PreparedRetirementSource {
    service: String,
    account: String,
    credential: PreviousCredential,
}

fn prepare_retirement_source<E>(
    store: &dyn SecretStore,
    service: &str,
    account: &str,
    validate_rotation_source: impl FnOnce(&str) -> Result<String, E>,
) -> Result<PreparedRetirementSource, PublicApiAdminError> {
    let existing = store
        .read(service, account)?
        .ok_or(PublicApiAdminError::InvalidRotationSource)?;
    let bearer = std::str::from_utf8(existing.as_ref())
        .map_err(|_| PublicApiAdminError::InvalidRotationSource)?;
    let credential_id =
        validate_rotation_source(bearer).map_err(|_| PublicApiAdminError::InvalidRotationSource)?;
    Ok(PreparedRetirementSource {
        service: service.to_owned(),
        account: account.to_owned(),
        credential: PreviousCredential {
            credential_id,
            bearer: existing,
        },
    })
}

fn prepare_destination<E>(
    store: &dyn SecretStore,
    service: &str,
    account: &str,
    replace: bool,
    validate_rotation_source: impl FnOnce(&str) -> Result<String, E>,
) -> Result<PreparedDestination, PublicApiAdminError> {
    let Some(existing) = store.read(service, account)? else {
        return Ok(PreparedDestination { previous: None });
    };
    if !replace {
        return Err(PublicApiAdminError::DestinationExists);
    }
    let bearer = std::str::from_utf8(existing.as_ref())
        .map_err(|_| PublicApiAdminError::InvalidRotationSource)?;
    let credential_id =
        validate_rotation_source(bearer).map_err(|_| PublicApiAdminError::InvalidRotationSource)?;
    Ok(PreparedDestination {
        previous: Some(PreviousCredential {
            credential_id,
            bearer: existing,
        }),
    })
}

fn deliver_issued_credential<E>(
    store: &dyn SecretStore,
    service: &str,
    account: &str,
    credential_id: &str,
    token: &[u8],
    previous: Option<&PreviousCredential>,
    revoke: impl FnOnce() -> Result<(), E>,
) -> Result<(), PublicApiAdminError> {
    if store.write(service, account, token).is_ok() {
        return Ok(());
    }
    let destination_restored = restore_destination(store, service, account, previous);
    match (revoke(), destination_restored) {
        (Ok(()), true) => Err(PublicApiAdminError::CredentialDeliveryFailed),
        (Ok(()) | Err(_), false) | (Err(_), true) => {
            Err(PublicApiAdminError::CredentialDeliveryAndRevocationFailed {
                credential_id: credential_id.to_owned(),
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn install_pending_credential<E>(
    store: &dyn SecretStore,
    service: &str,
    account: &str,
    new_credential_id: &str,
    token: &[u8],
    previous: Option<PreviousCredential>,
    mut activate: impl FnMut(&str) -> Result<bool, E>,
    mut revoke: impl FnMut(&str) -> Result<bool, E>,
) -> Result<Option<String>, PublicApiAdminError> {
    deliver_issued_credential(
        store,
        service,
        account,
        new_credential_id,
        token,
        previous.as_ref(),
        || match revoke(new_credential_id) {
            Ok(true) => Ok(()),
            Ok(false) | Err(_) => Err(()),
        },
    )?;
    if !matches!(activate(new_credential_id), Ok(true)) {
        return Err(compensate_failed_activation(
            store,
            service,
            account,
            previous.as_ref(),
            new_credential_id,
            &mut revoke,
        ));
    }
    retire_replaced_credential(previous, new_credential_id, revoke)
}

#[allow(clippy::too_many_arguments)]
fn install_migrated_credential<E>(
    store: &dyn SecretStore,
    destination_service: &str,
    destination_account: &str,
    new_credential_id: &str,
    token: &[u8],
    retirement_source: PreparedRetirementSource,
    mut activate: impl FnMut(&str) -> Result<bool, E>,
    mut revoke: impl FnMut(&str) -> Result<bool, E>,
) -> Result<String, PublicApiAdminError> {
    install_pending_credential(
        store,
        destination_service,
        destination_account,
        new_credential_id,
        token,
        None,
        |candidate| activate(candidate),
        |candidate| revoke(candidate),
    )?;
    retire_migrated_credential(store, retirement_source, new_credential_id, |candidate| {
        revoke(candidate)
    })
}

fn retire_migrated_credential<E>(
    store: &dyn SecretStore,
    retirement_source: PreparedRetirementSource,
    new_credential_id: &str,
    mut revoke: impl FnMut(&str) -> Result<bool, E>,
) -> Result<String, PublicApiAdminError> {
    let previous_credential_id = retirement_source.credential.credential_id.clone();
    if !revoke(&previous_credential_id).is_ok_and(|revoked| revoked) {
        return Err(
            PublicApiAdminError::CredentialMigrationRevocationUnconfirmed {
                previous_credential_id,
                new_credential_id: new_credential_id.to_owned(),
            },
        );
    }

    // Do not delete a different credential that was written to the source selector
    // concurrently. A missing entry already satisfies cleanup; any read error or
    // changed value requires explicit reconciliation. The OS keyring API has no
    // portable compare-and-delete primitive, so the production adapter narrows but
    // cannot eliminate the final same-user replacement race.
    let source_still_matches = match store
        .read(&retirement_source.service, &retirement_source.account)
    {
        Ok(None) => return Ok(previous_credential_id),
        Ok(Some(current)) => current.as_slice() == retirement_source.credential.bearer.as_slice(),
        Err(_) => false,
    };
    if !source_still_matches
        || store
            .delete(&retirement_source.service, &retirement_source.account)
            .is_err()
    {
        return Err(
            PublicApiAdminError::CredentialRetirementCleanupUnconfirmed {
                previous_credential_id,
                new_credential_id: new_credential_id.to_owned(),
            },
        );
    }
    Ok(previous_credential_id)
}

fn compensate_failed_activation<E>(
    store: &dyn SecretStore,
    service: &str,
    account: &str,
    previous: Option<&PreviousCredential>,
    new_credential_id: &str,
    mut revoke: impl FnMut(&str) -> Result<bool, E>,
) -> PublicApiAdminError {
    let revoked = revoke(new_credential_id).is_ok_and(|value| value);
    let destination_restored = restore_destination(store, service, account, previous);
    if revoked && destination_restored {
        PublicApiAdminError::CredentialActivationFailed
    } else {
        PublicApiAdminError::CredentialActivationCompensationFailed {
            credential_id: new_credential_id.to_owned(),
        }
    }
}

fn restore_destination(
    store: &dyn SecretStore,
    service: &str,
    account: &str,
    previous: Option<&PreviousCredential>,
) -> bool {
    match previous {
        Some(previous) => store
            .write(service, account, previous.bearer.as_ref())
            .is_ok(),
        None => store.delete(service, account).is_ok(),
    }
}

fn retire_replaced_credential<E>(
    previous: Option<PreviousCredential>,
    new_credential_id: &str,
    mut revoke: impl FnMut(&str) -> Result<bool, E>,
) -> Result<Option<String>, PublicApiAdminError> {
    let Some(previous) = previous else {
        return Ok(None);
    };
    if revoke(&previous.credential_id).is_ok_and(|revoked| revoked) {
        return Ok(Some(previous.credential_id));
    }

    // The replacement was durably activated before this point. An error from
    // revoking the prior credential may mean that revocation committed but its
    // outcome was lost. Rolling back here could therefore revoke the replacement
    // and restore an unusable prior bearer, leaving the application with no active
    // credential. Preserve the active replacement and report both non-secret IDs
    // so an administrator can reconcile the uncertain prior state safely.
    Err(
        PublicApiAdminError::CredentialRotationRevocationUnconfirmed {
            previous_credential_id: previous.credential_id,
            new_credential_id: new_credential_id.to_owned(),
        },
    )
}

fn validate_keyring_identifier(value: &str) -> Result<(), PublicApiAdminError> {
    if value.is_empty()
        || value.len() > 255
        || value.trim() != value
        || value.contains("..")
        || matches!(value.as_bytes().first(), Some(b'/' | b':'))
        || matches!(value.as_bytes().last(), Some(b'/' | b':'))
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/' | b'@' | b'+')
        })
    {
        return Err(PublicApiAdminError::InvalidKeyringIdentifier);
    }
    Ok(())
}

fn normalize_tool_ceiling(tools: &[String]) -> Result<Vec<String>, PublicApiAdminError> {
    let mut tools = tools.to_vec();
    tools.sort();
    tools.dedup();
    if tools
        .iter()
        .any(|tool| UNSUPPORTED_PUBLIC_TOOLS.contains(&tool.as_str()))
    {
        return Err(PublicApiAdminError::InvalidGrant);
    }
    Ok(tools)
}

fn normalize_scopes(scopes: &[String]) -> Result<Vec<ApiScope>, PublicApiAdminError> {
    let mut scopes = scopes
        .iter()
        .map(|scope| {
            if KNOWN_SCOPES.contains(&scope.as_str()) {
                ApiScope::new(scope.clone()).map_err(|_| PublicApiAdminError::InvalidScope)
            } else {
                Err(PublicApiAdminError::InvalidScope)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    scopes.sort();
    scopes.dedup();
    Ok(scopes)
}

#[cfg(unix)]
fn load_or_create_exact_key(
    store: &dyn SecretStore,
    service: &str,
    account: &str,
) -> Result<Zeroizing<[u8; 32]>, PublicApiAdminError> {
    if let Some(existing) = store.read(service, account)? {
        return exact_key(existing);
    }

    let mut generated = Zeroizing::new([0_u8; 32]);
    getrandom::fill(generated.as_mut()).map_err(|_| PublicApiAdminError::SecretStoreUnavailable)?;
    store.write(service, account, generated.as_ref())?;
    let stored = store
        .read(service, account)?
        .ok_or(PublicApiAdminError::SecretStoreUnavailable)?;
    let stored = exact_key(stored)?;
    if stored.as_ref() != generated.as_ref() {
        return Err(PublicApiAdminError::SecretStoreUnavailable);
    }
    Ok(stored)
}

#[cfg(unix)]
fn exact_key(value: Zeroizing<Vec<u8>>) -> Result<Zeroizing<[u8; 32]>, PublicApiAdminError> {
    if value.len() != 32 {
        return Err(PublicApiAdminError::SecretStoreValueInvalid);
    }
    let mut key = Zeroizing::new([0_u8; 32]);
    key.copy_from_slice(&value);
    Ok(key)
}

#[cfg(unix)]
fn namespace_service(directory: &Path) -> String {
    let digest = Sha256::digest(directory.as_os_str().as_encoded_bytes());
    format!("{KEYRING_SERVICE_PREFIX}.{}", lowercase_hex(&digest))
}

#[cfg(unix)]
fn stable_instance_id(seed: &[u8; 32]) -> Uuid {
    let mut hasher = Sha256::new();
    hasher.update(INSTANCE_ID_DOMAIN);
    hasher.update(seed);
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // RFC 9562 version 8 is reserved for application-defined deterministic UUIDs.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

#[cfg(unix)]
fn lowercase_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(unix)]
fn prepare_owner_private_directory(path: &Path) -> Result<PathBuf, PublicApiAdminError> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(PublicApiAdminError::InvalidDirectory);
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_directory_metadata(&metadata)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or(PublicApiAdminError::InvalidDirectory)?;
            let canonical_parent = parent
                .canonicalize()
                .map_err(|_| PublicApiAdminError::InvalidDirectory)?;
            let name = path
                .file_name()
                .ok_or(PublicApiAdminError::InvalidDirectory)?;
            let target = canonical_parent.join(name);
            match fs::symlink_metadata(&target) {
                Ok(metadata) => validate_directory_metadata(&metadata)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fs::DirBuilder::new()
                        .mode(DIRECTORY_MODE)
                        .create(&target)
                        .map_err(|_| PublicApiAdminError::InvalidDirectory)?;
                }
                Err(_) => return Err(PublicApiAdminError::InvalidDirectory),
            }
        }
        Err(_) => return Err(PublicApiAdminError::InvalidDirectory),
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| PublicApiAdminError::InvalidDirectory)?;
    revalidate_owner_private_directory(&canonical)?;
    Ok(canonical)
}

#[cfg(unix)]
fn revalidate_owner_private_directory(path: &Path) -> Result<(), PublicApiAdminError> {
    owner_private_directory_metadata(path).map(|_| ())
}

#[cfg(unix)]
fn owner_private_directory_metadata(path: &Path) -> Result<fs::Metadata, PublicApiAdminError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| PublicApiAdminError::InvalidDirectory)?;
    validate_directory_metadata(&metadata)?;
    Ok(metadata)
}

#[cfg(unix)]
fn revalidate_exact_directory(
    path: &Path,
    expected_device: u64,
    expected_inode: u64,
) -> Result<(), PublicApiAdminError> {
    let metadata = owner_private_directory_metadata(path)?;
    if metadata.dev() != expected_device || metadata.ino() != expected_inode {
        return Err(PublicApiAdminError::InvalidDirectory);
    }
    Ok(())
}

#[cfg(unix)]
fn validate_directory_metadata(metadata: &fs::Metadata) -> Result<(), PublicApiAdminError> {
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.mode() & 0o777 != DIRECTORY_MODE
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(PublicApiAdminError::InvalidDirectory);
    }
    Ok(())
}

#[cfg(unix)]
fn acquire_directory_lease(directory: &Path) -> Result<File, PublicApiAdminError> {
    let path = directory.join(LOCK_FILENAME);
    let before = fs::symlink_metadata(&path).ok();
    if before.as_ref().is_some_and(|metadata| {
        metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.mode() & 0o777 != LOCK_FILE_MODE
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.nlink() != 1
    }) {
        return Err(PublicApiAdminError::InvalidDirectory);
    }
    let file = rustix::fs::open(
        &path,
        rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::RDWR
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .map(File::from)
    .map_err(|_| PublicApiAdminError::InvalidDirectory)?;
    let metadata = file
        .metadata()
        .map_err(|_| PublicApiAdminError::InvalidDirectory)?;
    if !metadata.is_file()
        || metadata.mode() & 0o777 != LOCK_FILE_MODE
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.nlink() != 1
        || before
            .as_ref()
            .is_some_and(|before| before.dev() != metadata.dev() || before.ino() != metadata.ino())
    {
        return Err(PublicApiAdminError::InvalidDirectory);
    }
    if !file
        .try_lock_exclusive()
        .map_err(|_| PublicApiAdminError::InvalidDirectory)?
    {
        return Err(PublicApiAdminError::DirectoryBusy);
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::BTreeMap,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };

    #[derive(Default)]
    struct MemoryStore {
        values: Mutex<BTreeMap<(String, String), Vec<u8>>>,
        fail_writes: AtomicBool,
        fail_deletes: AtomicBool,
        retained_failed_secret: Mutex<Option<Vec<u8>>>,
    }

    impl SecretStore for MemoryStore {
        fn read(
            &self,
            service: &str,
            account: &str,
        ) -> Result<Option<Zeroizing<Vec<u8>>>, PublicApiAdminError> {
            Ok(self
                .values
                .lock()
                .expect("values")
                .get(&(service.to_owned(), account.to_owned()))
                .cloned()
                .map(Zeroizing::new))
        }

        fn write(
            &self,
            service: &str,
            account: &str,
            secret: &[u8],
        ) -> Result<(), PublicApiAdminError> {
            if self.fail_writes.load(Ordering::Acquire) {
                // A failing implementation must not retain or include the supplied
                // bearer in its error. The production adapter has the same property.
                *self.retained_failed_secret.lock().expect("failed secret") = None;
                return Err(PublicApiAdminError::SecretStoreUnavailable);
            }
            self.values
                .lock()
                .expect("values")
                .insert((service.to_owned(), account.to_owned()), secret.to_vec());
            Ok(())
        }

        fn delete(&self, service: &str, account: &str) -> Result<(), PublicApiAdminError> {
            if self.fail_deletes.load(Ordering::Acquire) {
                return Err(PublicApiAdminError::SecretStoreUnavailable);
            }
            self.values
                .lock()
                .expect("values")
                .remove(&(service.to_owned(), account.to_owned()));
            Ok(())
        }
    }

    #[cfg(unix)]
    #[test]
    fn stable_material_uses_path_specific_namespaces_and_redacted_debug() {
        let directory = tempfile::tempdir().expect("directory");
        let first_path = directory.path().join("first");
        let second_path = directory.path().join("second");
        let store = MemoryStore::default();
        let first = PublicApiEnvironment::open(&first_path, &store).expect("first");
        let first_service = first.namespace_service.clone();
        let first_instance = stable_instance_id(&first.instance_seed);
        let first_identity = first.public_identity().expect("public identity");
        assert_eq!(first_identity.0, first_instance);
        assert_eq!(first_identity.1.len(), 64);
        assert!(
            first_identity
                .1
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        assert!(!format!("{first:?}").contains(&lowercase_hex(&first.authentication_root[..])));
        drop(first);

        // Another parallel test can fork while this test owns the close-on-exec
        // lease descriptor. The child retains that descriptor only until exec,
        // so tolerate that bounded test-process handoff before reopening.
        let reopen_deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let reopened = loop {
            match PublicApiEnvironment::open(&first_path, &store) {
                Ok(environment) => break environment,
                Err(PublicApiAdminError::DirectoryBusy)
                    if std::time::Instant::now() < reopen_deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(error) => panic!("reopen: {error:?}"),
            }
        };
        assert_eq!(reopened.namespace_service, first_service);
        assert_eq!(stable_instance_id(&reopened.instance_seed), first_instance);
        assert_eq!(
            reopened.public_identity().expect("reopened identity"),
            first_identity
        );
        drop(reopened);

        let second = PublicApiEnvironment::open(&second_path, &store).expect("second");
        assert_ne!(second.namespace_service, first_service);
    }

    #[test]
    fn existing_destination_requires_explicit_replacement() {
        let store = MemoryStore::default();
        store
            .write("com.example.app", "colossus-token", b"old")
            .expect("seed");
        assert_eq!(
            prepare_destination(
                &store,
                "com.example.app",
                "colossus-token",
                false,
                |_| Ok::<_, ()>("old-id".into())
            )
            .expect_err("replacement must be explicit"),
            PublicApiAdminError::DestinationExists
        );
        let prepared =
            prepare_destination(&store, "com.example.app", "colossus-token", true, |_| {
                Ok::<_, ()>("old-id".into())
            })
            .expect("explicit replacement");
        assert_eq!(
            prepared
                .previous
                .as_ref()
                .map(|previous| previous.credential_id.as_str()),
            Some("old-id")
        );
    }

    #[test]
    fn desktop_auto_destination_is_bound_to_instance_and_certificate() {
        let instance_id =
            Uuid::parse_str("01968a3e-0ab3-7f10-bb27-4eadbd550007").expect("instance id");
        let pin = "a".repeat(64);
        assert_eq!(
            bound_destination_account(
                DESKTOP_EXTERNAL_KEYRING_SERVICE,
                DESKTOP_BOUND_ACCOUNT_REQUEST,
                instance_id,
                &pin,
            ),
            format!("daemon-{instance_id}-{pin}")
        );
        assert_eq!(
            bound_destination_account("com.example.app", "auto", instance_id, &pin),
            "auto"
        );
    }

    #[test]
    fn malformed_or_unrelated_rotation_source_fails_before_mutation() {
        let store = MemoryStore::default();
        store
            .write("com.example.app", "colossus-token", b"not-a-colossus-token")
            .expect("seed");
        let error = prepare_destination(&store, "com.example.app", "colossus-token", true, |_| {
            Err::<String, _>(())
        })
        .expect_err("malformed or unrelated token");
        assert_eq!(error, PublicApiAdminError::InvalidRotationSource);
        assert_eq!(
            store
                .read("com.example.app", "colossus-token")
                .expect("read")
                .expect("existing")
                .as_slice(),
            b"not-a-colossus-token"
        );
        assert!(!error.to_string().contains("not-a-colossus-token"));

        let retirement_error =
            prepare_retirement_source(&store, "com.example.app", "colossus-token", |_| {
                Err::<String, _>(())
            })
            .err()
            .expect("migration source must authenticate");
        assert_eq!(retirement_error, PublicApiAdminError::InvalidRotationSource);
        assert_eq!(
            store
                .read("com.example.app", "colossus-token")
                .expect("read")
                .expect("existing")
                .as_slice(),
            b"not-a-colossus-token"
        );
        assert!(
            !retirement_error
                .to_string()
                .contains("not-a-colossus-token")
        );
    }

    #[test]
    fn migrated_credential_activates_then_revokes_then_deletes_legacy_source() {
        let store = MemoryStore::default();
        store
            .write(
                "com.obscuritylabs.colossus.desktop",
                "colossus-public-api",
                b"old-active-secret",
            )
            .expect("legacy credential");
        let retirement_source = prepare_retirement_source(
            &store,
            "com.obscuritylabs.colossus.desktop",
            "colossus-public-api",
            |_| Ok::<_, ()>("old-id".into()),
        )
        .expect("validated source");
        let calls = Mutex::new(Vec::new());

        let retired = install_migrated_credential(
            &store,
            DESKTOP_EXTERNAL_KEYRING_SERVICE,
            "daemon-instance-pin",
            "new-id",
            b"new-active-secret",
            retirement_source,
            |credential_id| {
                assert_eq!(credential_id, "new-id");
                assert_eq!(
                    store
                        .read(DESKTOP_EXTERNAL_KEYRING_SERVICE, "daemon-instance-pin")
                        .expect("destination read")
                        .expect("delivered destination")
                        .as_slice(),
                    b"new-active-secret"
                );
                assert!(
                    store
                        .read("com.obscuritylabs.colossus.desktop", "colossus-public-api")
                        .expect("source read")
                        .is_some(),
                    "the source entry remains until its credential is revoked"
                );
                calls
                    .lock()
                    .expect("calls")
                    .push(format!("activate:{credential_id}"));
                Ok::<_, ()>(true)
            },
            |credential_id| {
                assert_eq!(credential_id, "old-id");
                assert!(
                    store
                        .read("com.obscuritylabs.colossus.desktop", "colossus-public-api")
                        .expect("source read")
                        .is_some(),
                    "keyring deletion must follow durable revocation"
                );
                calls
                    .lock()
                    .expect("calls")
                    .push(format!("revoke:{credential_id}"));
                Ok::<_, ()>(true)
            },
        )
        .expect("migration");

        assert_eq!(retired, "old-id");
        assert_eq!(
            *calls.lock().expect("calls"),
            ["activate:new-id", "revoke:old-id"]
        );
        assert!(
            store
                .read("com.obscuritylabs.colossus.desktop", "colossus-public-api")
                .expect("source read")
                .is_none(),
            "confirmed revocation is followed by legacy keyring cleanup"
        );
    }

    #[test]
    fn migration_activation_failure_preserves_legacy_source() {
        let store = MemoryStore::default();
        store
            .write(
                "com.obscuritylabs.colossus.desktop",
                "colossus-public-api",
                b"old-active-secret",
            )
            .expect("legacy credential");
        let retirement_source = prepare_retirement_source(
            &store,
            "com.obscuritylabs.colossus.desktop",
            "colossus-public-api",
            |_| Ok::<_, ()>("old-id".into()),
        )
        .expect("validated source");
        let calls = Mutex::new(Vec::new());

        let error = install_migrated_credential(
            &store,
            DESKTOP_EXTERNAL_KEYRING_SERVICE,
            "daemon-instance-pin",
            "new-id",
            b"new-pending-secret",
            retirement_source,
            |credential_id| {
                calls
                    .lock()
                    .expect("calls")
                    .push(format!("activate:{credential_id}"));
                Ok::<_, ()>(false)
            },
            |credential_id| {
                calls
                    .lock()
                    .expect("calls")
                    .push(format!("revoke:{credential_id}"));
                Ok::<_, ()>(true)
            },
        )
        .expect_err("activation failure");

        assert_eq!(error, PublicApiAdminError::CredentialActivationFailed);
        assert_eq!(
            *calls.lock().expect("calls"),
            ["activate:new-id", "revoke:new-id"]
        );
        assert!(
            store
                .read(DESKTOP_EXTERNAL_KEYRING_SERVICE, "daemon-instance-pin")
                .expect("destination read")
                .is_none()
        );
        assert_eq!(
            store
                .read("com.obscuritylabs.colossus.desktop", "colossus-public-api")
                .expect("source read")
                .expect("legacy source preserved")
                .as_slice(),
            b"old-active-secret"
        );
    }

    #[test]
    fn migration_revocation_failure_preserves_both_reconciliation_handles() {
        let store = MemoryStore::default();
        store
            .write(
                "com.obscuritylabs.colossus.desktop",
                "colossus-public-api",
                b"old-secret-token",
            )
            .expect("legacy credential");
        let retirement_source = prepare_retirement_source(
            &store,
            "com.obscuritylabs.colossus.desktop",
            "colossus-public-api",
            |_| Ok::<_, ()>("old-id".into()),
        )
        .expect("validated source");

        let error = install_migrated_credential(
            &store,
            DESKTOP_EXTERNAL_KEYRING_SERVICE,
            "daemon-instance-pin",
            "new-id",
            b"new-secret-token",
            retirement_source,
            |_| Ok::<_, ()>(true),
            |_| Err::<bool, _>(()),
        )
        .expect_err("unconfirmed revocation");

        assert_eq!(
            error,
            PublicApiAdminError::CredentialMigrationRevocationUnconfirmed {
                previous_credential_id: "old-id".into(),
                new_credential_id: "new-id".into(),
            }
        );
        assert!(
            store
                .read("com.obscuritylabs.colossus.desktop", "colossus-public-api")
                .expect("source read")
                .is_some(),
            "an unconfirmed revocation never deletes the source"
        );
        assert_eq!(
            store
                .read(DESKTOP_EXTERNAL_KEYRING_SERVICE, "daemon-instance-pin")
                .expect("destination read")
                .expect("active replacement")
                .as_slice(),
            b"new-secret-token"
        );
        let message = error.to_string();
        assert!(message.contains("old-id"));
        assert!(message.contains("new-id"));
        assert!(message.contains("only if it is still credential old-id"));
        assert!(!message.contains("secret-token"));
    }

    #[test]
    fn migration_keyring_cleanup_failure_follows_confirmed_revocation() {
        let store = MemoryStore::default();
        store
            .write(
                "com.obscuritylabs.colossus.desktop",
                "colossus-public-api",
                b"old-secret-token",
            )
            .expect("legacy credential");
        let retirement_source = prepare_retirement_source(
            &store,
            "com.obscuritylabs.colossus.desktop",
            "colossus-public-api",
            |_| Ok::<_, ()>("old-id".into()),
        )
        .expect("validated source");
        store.fail_deletes.store(true, Ordering::Release);
        let calls = Mutex::new(Vec::new());

        let error = install_migrated_credential(
            &store,
            DESKTOP_EXTERNAL_KEYRING_SERVICE,
            "daemon-instance-pin",
            "new-id",
            b"new-secret-token",
            retirement_source,
            |credential_id| {
                calls
                    .lock()
                    .expect("calls")
                    .push(format!("activate:{credential_id}"));
                Ok::<_, ()>(true)
            },
            |credential_id| {
                calls
                    .lock()
                    .expect("calls")
                    .push(format!("revoke:{credential_id}"));
                Ok::<_, ()>(true)
            },
        )
        .expect_err("keyring cleanup failure");

        assert_eq!(
            *calls.lock().expect("calls"),
            ["activate:new-id", "revoke:old-id"]
        );
        assert_eq!(
            error,
            PublicApiAdminError::CredentialRetirementCleanupUnconfirmed {
                previous_credential_id: "old-id".into(),
                new_credential_id: "new-id".into(),
            }
        );
        assert!(
            store
                .read("com.obscuritylabs.colossus.desktop", "colossus-public-api")
                .expect("source read")
                .is_some()
        );
        assert_eq!(
            store
                .read(DESKTOP_EXTERNAL_KEYRING_SERVICE, "daemon-instance-pin")
                .expect("destination read")
                .expect("active replacement")
                .as_slice(),
            b"new-secret-token"
        );
        let message = error.to_string();
        assert!(message.contains("old-id"));
        assert!(message.contains("new-id"));
        assert!(!message.contains("secret-token"));
    }

    #[test]
    fn migration_never_deletes_a_changed_source_keyring_value() {
        let store = MemoryStore::default();
        store
            .write(
                "com.obscuritylabs.colossus.desktop",
                "colossus-public-api",
                b"old-secret-token",
            )
            .expect("legacy credential");
        let retirement_source = prepare_retirement_source(
            &store,
            "com.obscuritylabs.colossus.desktop",
            "colossus-public-api",
            |_| Ok::<_, ()>("old-id".into()),
        )
        .expect("validated source");

        let error = install_migrated_credential(
            &store,
            DESKTOP_EXTERNAL_KEYRING_SERVICE,
            "daemon-instance-pin",
            "new-id",
            b"new-secret-token",
            retirement_source,
            |_| Ok::<_, ()>(true),
            |_| {
                store
                    .write(
                        "com.obscuritylabs.colossus.desktop",
                        "colossus-public-api",
                        b"concurrently-replaced-token",
                    )
                    .expect("replace source selector");
                Ok::<_, ()>(true)
            },
        )
        .expect_err("changed source requires reconciliation");

        assert_eq!(
            error,
            PublicApiAdminError::CredentialRetirementCleanupUnconfirmed {
                previous_credential_id: "old-id".into(),
                new_credential_id: "new-id".into(),
            }
        );
        assert_eq!(
            store
                .read("com.obscuritylabs.colossus.desktop", "colossus-public-api")
                .expect("source read")
                .expect("changed source remains")
                .as_slice(),
            b"concurrently-replaced-token"
        );
        assert!(
            error
                .to_string()
                .contains("do not delete its current value unless it is confirmed")
        );
    }

    #[test]
    fn failed_delivery_compensates_without_disclosing_the_bearer() {
        let store = MemoryStore::default();
        store.fail_writes.store(true, Ordering::Release);
        let revoked = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&revoked);
        let bearer = b"cls_v1.credential.super-secret-bearer";
        let error = deliver_issued_credential(
            &store,
            "com.example.app",
            "colossus-token",
            "new-id",
            bearer,
            None,
            move || {
                observed.store(true, Ordering::Release);
                Ok::<_, ()>(())
            },
        )
        .expect_err("delivery must fail");
        assert!(revoked.load(Ordering::Acquire));
        assert_eq!(error, PublicApiAdminError::CredentialDeliveryFailed);
        assert!(!error.to_string().contains("super-secret-bearer"));
        assert!(
            store
                .retained_failed_secret
                .lock()
                .expect("failed secret")
                .is_none()
        );
    }

    #[test]
    fn failed_delivery_reports_unconfirmed_compensation_without_secret() {
        let store = MemoryStore::default();
        store.fail_writes.store(true, Ordering::Release);
        let bearer = b"cls_v1.credential.another-secret-bearer";
        let error = deliver_issued_credential(
            &store,
            "com.example.app",
            "colossus-token",
            "new-id",
            bearer,
            None,
            || Err("revoke failed"),
        )
        .expect_err("delivery must fail");
        assert_eq!(
            error,
            PublicApiAdminError::CredentialDeliveryAndRevocationFailed {
                credential_id: "new-id".into()
            }
        );
        assert!(!error.to_string().contains("another-secret-bearer"));
    }

    #[test]
    fn failed_activation_revokes_pending_credential_and_clears_new_destination() {
        let store = MemoryStore::default();
        store
            .write("com.example.app", "colossus-token", b"new-pending-secret")
            .expect("pending stored");
        let calls = Mutex::new(Vec::new());
        let error = compensate_failed_activation(
            &store,
            "com.example.app",
            "colossus-token",
            None,
            "new-id",
            |credential_id| {
                calls.lock().expect("calls").push(credential_id.to_owned());
                Ok::<_, ()>(true)
            },
        );

        assert_eq!(error, PublicApiAdminError::CredentialActivationFailed);
        assert_eq!(*calls.lock().expect("calls"), ["new-id"]);
        assert!(
            store
                .read("com.example.app", "colossus-token")
                .expect("read")
                .is_none()
        );
    }

    #[test]
    fn failed_rotation_activation_restores_prior_destination() {
        let store = MemoryStore::default();
        store
            .write("com.example.app", "colossus-token", b"new-pending-secret")
            .expect("pending stored");
        let previous = PreviousCredential {
            credential_id: "old-id".into(),
            bearer: Zeroizing::new(b"old-active-secret".to_vec()),
        };
        let error = compensate_failed_activation(
            &store,
            "com.example.app",
            "colossus-token",
            Some(&previous),
            "new-id",
            |_| Ok::<_, ()>(true),
        );

        assert_eq!(error, PublicApiAdminError::CredentialActivationFailed);
        assert_eq!(
            store
                .read("com.example.app", "colossus-token")
                .expect("read")
                .expect("restored")
                .as_slice(),
            b"old-active-secret"
        );
    }

    #[test]
    fn failed_activation_reports_unconfirmed_revocation_without_secret() {
        let store = MemoryStore::default();
        store
            .write(
                "com.example.app",
                "colossus-token",
                b"new-unconfirmed-secret",
            )
            .expect("pending stored");
        let error = compensate_failed_activation(
            &store,
            "com.example.app",
            "colossus-token",
            None,
            "new-id",
            |_| Err::<bool, _>("storage unavailable"),
        );

        assert_eq!(
            error,
            PublicApiAdminError::CredentialActivationCompensationFailed {
                credential_id: "new-id".into()
            }
        );
        assert!(error.to_string().contains("new-id"));
        assert!(!error.to_string().contains("unconfirmed-secret"));
    }

    #[test]
    fn credential_lifecycle_never_activates_after_keyring_write_failure() {
        let store = MemoryStore::default();
        store.fail_writes.store(true, Ordering::Release);
        let calls = Mutex::new(Vec::new());
        let error = install_pending_credential(
            &store,
            "com.example.app",
            "colossus-token",
            "new-id",
            b"new-pending-secret",
            None,
            |credential_id| {
                calls
                    .lock()
                    .expect("calls")
                    .push(format!("activate:{credential_id}"));
                Ok::<_, ()>(true)
            },
            |credential_id| {
                calls
                    .lock()
                    .expect("calls")
                    .push(format!("revoke:{credential_id}"));
                Ok::<_, ()>(true)
            },
        )
        .expect_err("write failure must compensate");

        assert_eq!(error, PublicApiAdminError::CredentialDeliveryFailed);
        assert_eq!(
            *calls.lock().expect("calls"),
            ["revoke:new-id"],
            "a token is activated only after confirmed delivery"
        );
        assert!(
            store
                .read("com.example.app", "colossus-token")
                .expect("read")
                .is_none()
        );
    }

    #[test]
    fn credential_lifecycle_activation_failure_revokes_and_removes_new_token() {
        let store = MemoryStore::default();
        let calls = Mutex::new(Vec::new());
        let error = install_pending_credential(
            &store,
            "com.example.app",
            "colossus-token",
            "new-id",
            b"new-pending-secret",
            None,
            |credential_id| {
                assert_eq!(
                    store
                        .read("com.example.app", "colossus-token")
                        .expect("read delivered token")
                        .expect("delivered token")
                        .as_slice(),
                    b"new-pending-secret"
                );
                calls
                    .lock()
                    .expect("calls")
                    .push(format!("activate:{credential_id}"));
                Ok::<_, ()>(false)
            },
            |credential_id| {
                calls
                    .lock()
                    .expect("calls")
                    .push(format!("revoke:{credential_id}"));
                Ok::<_, ()>(true)
            },
        )
        .expect_err("activation failure must compensate");

        assert_eq!(error, PublicApiAdminError::CredentialActivationFailed);
        assert_eq!(
            *calls.lock().expect("calls"),
            ["activate:new-id", "revoke:new-id"]
        );
        assert!(
            store
                .read("com.example.app", "colossus-token")
                .expect("read")
                .is_none()
        );
    }

    #[test]
    fn credential_rotation_activation_failure_preserves_old_active_credential() {
        let store = MemoryStore::default();
        store
            .write("com.example.app", "colossus-token", b"old-active-secret")
            .expect("old token");
        let calls = Mutex::new(Vec::new());
        let error = install_pending_credential(
            &store,
            "com.example.app",
            "colossus-token",
            "new-id",
            b"new-pending-secret",
            Some(PreviousCredential {
                credential_id: "old-id".into(),
                bearer: Zeroizing::new(b"old-active-secret".to_vec()),
            }),
            |credential_id| {
                calls
                    .lock()
                    .expect("calls")
                    .push(format!("activate:{credential_id}"));
                Ok::<_, ()>(false)
            },
            |credential_id| {
                calls
                    .lock()
                    .expect("calls")
                    .push(format!("revoke:{credential_id}"));
                Ok::<_, ()>(true)
            },
        )
        .expect_err("activation failure must preserve old credential");

        assert_eq!(error, PublicApiAdminError::CredentialActivationFailed);
        assert_eq!(
            *calls.lock().expect("calls"),
            ["activate:new-id", "revoke:new-id"],
            "the old credential must not be revoked before new activation"
        );
        assert_eq!(
            store
                .read("com.example.app", "colossus-token")
                .expect("read")
                .expect("restored old token")
                .as_slice(),
            b"old-active-secret"
        );
    }

    #[test]
    fn credential_rotation_activates_new_before_revoking_old() {
        let store = MemoryStore::default();
        store
            .write("com.example.app", "colossus-token", b"old-active-secret")
            .expect("old token");
        let calls = Mutex::new(Vec::new());
        let retired = install_pending_credential(
            &store,
            "com.example.app",
            "colossus-token",
            "new-id",
            b"new-active-secret",
            Some(PreviousCredential {
                credential_id: "old-id".into(),
                bearer: Zeroizing::new(b"old-active-secret".to_vec()),
            }),
            |credential_id| {
                assert_eq!(
                    store
                        .read("com.example.app", "colossus-token")
                        .expect("read delivered token")
                        .expect("delivered token")
                        .as_slice(),
                    b"new-active-secret"
                );
                calls
                    .lock()
                    .expect("calls")
                    .push(format!("activate:{credential_id}"));
                Ok::<_, ()>(true)
            },
            |credential_id| {
                calls
                    .lock()
                    .expect("calls")
                    .push(format!("revoke:{credential_id}"));
                Ok::<_, ()>(true)
            },
        )
        .expect("rotation");

        assert_eq!(retired.as_deref(), Some("old-id"));
        assert_eq!(
            *calls.lock().expect("calls"),
            ["activate:new-id", "revoke:old-id"]
        );
        assert_eq!(
            store
                .read("com.example.app", "colossus-token")
                .expect("read")
                .expect("new token")
                .as_slice(),
            b"new-active-secret"
        );
    }

    #[test]
    fn replacement_stores_new_then_revokes_old() {
        let store = MemoryStore::default();
        store
            .write("com.example.app", "colossus-token", b"new-token")
            .expect("new stored first");
        let calls = Mutex::new(Vec::new());
        let retired = retire_replaced_credential(
            Some(PreviousCredential {
                credential_id: "old-id".into(),
                bearer: Zeroizing::new(b"old-token".to_vec()),
            }),
            "new-id",
            |credential_id| {
                calls.lock().expect("calls").push(credential_id.to_owned());
                Ok::<_, ()>(true)
            },
        )
        .expect("rotation");
        assert_eq!(retired.as_deref(), Some("old-id"));
        assert_eq!(*calls.lock().expect("calls"), ["old-id"]);
        assert_eq!(
            store
                .read("com.example.app", "colossus-token")
                .expect("read")
                .expect("stored")
                .as_slice(),
            b"new-token"
        );
    }

    #[test]
    fn old_revocation_failure_preserves_active_new_credential_and_reports_both_ids() {
        let store = MemoryStore::default();
        store
            .write("com.example.app", "colossus-token", b"old-secret-token")
            .expect("old token");
        let calls = Mutex::new(Vec::new());
        let error = install_pending_credential(
            &store,
            "com.example.app",
            "colossus-token",
            "new-id",
            b"new-secret-token",
            Some(PreviousCredential {
                credential_id: "old-id".into(),
                bearer: Zeroizing::new(b"old-secret-token".to_vec()),
            }),
            |credential_id| {
                calls
                    .lock()
                    .expect("calls")
                    .push(format!("activate:{credential_id}"));
                Ok::<_, ()>(true)
            },
            |credential_id| {
                calls
                    .lock()
                    .expect("calls")
                    .push(format!("revoke:{credential_id}"));
                Err::<bool, _>(())
            },
        )
        .expect_err("old revocation is unconfirmed");
        assert_eq!(
            error,
            PublicApiAdminError::CredentialRotationRevocationUnconfirmed {
                previous_credential_id: "old-id".into(),
                new_credential_id: "new-id".into(),
            }
        );
        assert_eq!(
            *calls.lock().expect("calls"),
            ["activate:new-id", "revoke:old-id"]
        );
        assert_eq!(
            store
                .read("com.example.app", "colossus-token")
                .expect("read")
                .expect("new token remains installed")
                .as_slice(),
            b"new-secret-token"
        );
        let message = error.to_string();
        assert!(message.contains("old-id"));
        assert!(message.contains("new-id"));
        assert!(message.contains("explicitly revoke prior credential"));
        assert!(!message.contains("secret-token"));
    }

    #[cfg(unix)]
    #[test]
    fn directory_rejects_links_and_non_owner_private_modes() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let root = tempfile::tempdir().expect("root");
        let store = MemoryStore::default();
        let insecure = root.path().join("insecure");
        fs::create_dir(&insecure).expect("insecure");
        fs::set_permissions(&insecure, fs::Permissions::from_mode(0o755)).expect("permissions");
        assert!(matches!(
            PublicApiEnvironment::open(&insecure, &store),
            Err(PublicApiAdminError::InvalidDirectory)
        ));

        let target = root.path().join("target");
        fs::create_dir(&target).expect("target");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).expect("permissions");
        let link = root.path().join("link");
        symlink(&target, &link).expect("link");
        assert!(matches!(
            PublicApiEnvironment::open(&link, &store),
            Err(PublicApiAdminError::InvalidDirectory)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn directory_inode_replacement_is_detected_after_locking() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().expect("root");
        let store = MemoryStore::default();
        let path = root.path().join("public-api");
        let environment = PublicApiEnvironment::open(&path, &store).expect("environment");
        let moved = root.path().join("moved-public-api");
        fs::rename(&path, &moved).expect("rename");
        fs::create_dir(&path).expect("replacement");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("permissions");
        assert!(matches!(
            revalidate_exact_directory(
                &path,
                environment.directory_device,
                environment.directory_inode
            ),
            Err(PublicApiAdminError::InvalidDirectory)
        ));
    }

    #[test]
    fn exact_known_scope_and_keyring_identifier_validation_is_closed() {
        assert!(KNOWN_SCOPES.contains(&scopes::RUNS_EXECUTE));
        assert!(validate_keyring_identifier("com.example.app").is_ok());
        assert!(validate_keyring_identifier("bad value").is_err());
        assert!(validate_keyring_identifier("../bad").is_err());
        assert!(!KNOWN_SCOPES.contains(&"runs:*"));
        assert_eq!(
            normalize_scopes(&["runs:*".into()]),
            Err(PublicApiAdminError::InvalidScope)
        );
        assert!(UNSUPPORTED_PUBLIC_TOOLS.contains(&"agent.delegate"));
        assert_eq!(
            normalize_tool_ceiling(&["agent.delegate".into()]),
            Err(PublicApiAdminError::InvalidGrant)
        );
        assert_eq!(
            normalize_tool_ceiling(&["session.list".into(), "session.list".into()])
                .expect("supported tool"),
            ["session.list"]
        );
    }
}
