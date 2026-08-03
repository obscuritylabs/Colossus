//! Minimal managed-sidecar composition executable.

use colossus_access::AccessProfile;
use colossus_api::{ApiScope, ApplicationKind};
use colossus_grpc::{TlsIdentity, TlsKeySeed};
use colossus_ports::StoreError;
use colossus_provider::ProviderKind;
use colossus_runtime::{
    HostCredentialResolver, KeyConfig, ModelCapabilities, ModelProfileConfig, ModelsConfig,
    ProviderProfileConfig, ProvidersConfig, RuntimeConfig, RuntimeError, RuntimeOpenOptions,
    WorkspaceIdentityToken,
};
use colossus_sidecar_protocol::{
    AckRequest, ActivatedResponse, BootstrapGrant, BootstrapRequest, ChildFrame, FailureCode,
    FailureResponse, ManagedAccessProfile, ManagedProviderKind, ManagedRuntimeConfig,
    PROTOCOL_VERSION, ParentFrame, ReadyResponse, SecretString,
    WorkspaceIdentity as BootstrapWorkspaceIdentity, decode_worker_authentication, read_frame,
    write_frame,
};
use colossus_worker::{
    ApplicationGrant, PublicApiAuthenticationKey, PublicApiDeploymentMode, PublicApiHostOptions,
    WorkerApprovalMode, WorkerAuthenticationKey, WorkerServer,
};
#[cfg(test)]
use std::collections::BTreeMap;
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::{Read as _, Write as _},
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
};
use uuid::Uuid;
use zeroize::Zeroizing;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _};

const MANAGED_SIDECAR_ARGUMENT: &str = "__managed-sidecar-v1";
const SANDBOX_HELPER_ARGUMENT: &str = "__sandbox-helper";
const SANDBOX_PROBE_ARGUMENT: &str = "__sandbox-protection-probe";
const PUBLIC_API_DIRECTORY: &str = "public-api";
const DESCRIPTOR_FILENAME: &str = "endpoint.json";
const CERTIFICATE_FILENAME: &str = "certificate.pem";
const MANAGED_CONFIG_FILENAME: &str = "managed-config.yaml";
const MANAGED_CA_BUNDLE_FILENAME: &str = "additional-ca-bundle.pem";
const MANAGED_KEYRING_SERVICE: &str = "com.obscuritylabs.colossus.managed-runtime";
const MAX_CA_BUNDLE_BYTES: u64 = 4 * 1024 * 1024;

pub(crate) fn main_entry() -> ExitCode {
    let argument = std::env::args_os().nth(1);
    if argument.as_deref() == Some(std::ffi::OsStr::new(SANDBOX_HELPER_ARGUMENT)) {
        return match colossus_sandbox::run_helper_stdio() {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        };
    }
    if argument.as_deref() == Some(std::ffi::OsStr::new(SANDBOX_PROBE_ARGUMENT)) {
        return match colossus_sandbox::run_native_protection_probe() {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        };
    }
    if argument.as_deref() != Some(std::ffi::OsStr::new(MANAGED_SIDECAR_ARGUMENT))
        || std::env::args_os().nth(2).is_some()
    {
        return ExitCode::FAILURE;
    }
    #[cfg(unix)]
    if !managed_session_is_established() && rustix::process::setsid().is_err() {
        send_failure(None, FailureCode::RuntimeFailed);
        return ExitCode::FAILURE;
    }
    #[cfg(not(any(unix, windows)))]
    {
        send_failure(None, FailureCode::RuntimeFailed);
        return ExitCode::FAILURE;
    }
    #[cfg(windows)]
    let _bootstrap_pipe = match connect_windows_bootstrap_pipe() {
        Ok(pipe) => pipe,
        Err(()) => return ExitCode::FAILURE,
    };

    let mut input = std::io::stdin();
    let request = match read_frame::<_, ParentFrame>(&mut input) {
        Ok(ParentFrame::Bootstrap(request)) if request.validate().is_ok() => *request,
        _ => {
            send_failure(None, FailureCode::InvalidBootstrap);
            return ExitCode::FAILURE;
        }
    };
    let exchange_id = request.exchange_id.clone();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            send_failure(Some(exchange_id), FailureCode::RuntimeFailed);
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(request, &mut input)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            send_failure(Some(exchange_id), failure);
            ExitCode::FAILURE
        }
    }
}

#[cfg(unix)]
fn managed_session_is_established() -> bool {
    let process = rustix::process::getpid();
    rustix::process::getsid(None).ok() == Some(process)
        && rustix::process::getpgid(None).ok() == Some(process)
}

async fn run(request: BootstrapRequest, input: &mut std::io::Stdin) -> Result<(), FailureCode> {
    let instance_id =
        Uuid::parse_str(&request.instance_id).map_err(|_| FailureCode::InvalidBootstrap)?;
    let instance_dir = private_directory(Path::new(&request.instance_dir))?;
    let workspace =
        BoundWorkspace::open(Path::new(&request.workspace), &request.workspace_identity)?;
    // Bind process-relative operations to the descriptor that reproduced the native
    // parent's opaque identity. The runtime separately reopens the pathname and checks
    // the same device/inode token as part of workspace lease acquisition.
    #[cfg(unix)]
    rustix::process::fchdir(&workspace.directory).map_err(|_| FailureCode::InvalidWorkspace)?;
    #[cfg(windows)]
    {
        workspace
            .binding
            .revalidate()
            .map_err(|_| FailureCode::InvalidWorkspace)?;
        std::env::set_current_dir(&workspace.canonical_path)
            .map_err(|_| FailureCode::InvalidWorkspace)?;
    }
    let ca_bundle_path = request
        .ca_bundle_path
        .as_deref()
        .map(|path| install_private_ca_bundle(Path::new(path), &instance_dir))
        .transpose()?;
    let config = managed_runtime_config(
        &request.runtime,
        instance_id,
        &instance_dir,
        ca_bundle_path.as_deref(),
    )?;
    persist_managed_config(&instance_dir, &config)?;
    let runtime_options = RuntimeOpenOptions::for_workspace(&workspace.canonical_path)
        .map_err(|_| FailureCode::InvalidWorkspace)?
        .with_expected_workspace_identity(workspace.runtime_identity()?);

    let provider_credentials = HostCredentialResolver::new(
        request
            .host_credentials
            .into_iter()
            .map(|credential| (credential.id, credential.secret.expose().to_owned())),
    )
    .map_err(|_| FailureCode::InvalidConfiguration)?;
    let worker_authentication = request
        .worker_ipc_authentication
        .as_ref()
        .map(decode_worker_authentication)
        .transpose()
        .map_err(|_| FailureCode::InvalidBootstrap)?;
    let provider_credentials = std::sync::Arc::new(provider_credentials);
    // Keep the independently opened child descriptor alive across Runtime::open and
    // for the complete worker lifetime. This prevents inode reuse while the runtime's
    // own retained descriptor and per-effect identity checks are active.
    let _workspace_binding = workspace;
    let server = if let Some(authentication) = worker_authentication {
        WorkerServer::open_with_mode_at_workspace_provider_credentials_and_authentication(
            &config,
            WorkerApprovalMode::Ask,
            runtime_options,
            provider_credentials,
            WorkerAuthenticationKey::from_zeroizing(authentication),
        )
    } else {
        WorkerServer::open_with_mode_at_workspace_and_provider_credentials(
            &config,
            WorkerApprovalMode::Ask,
            runtime_options,
            provider_credentials,
        )
    }
    .map_err(map_worker_open_failure)?
    .prepare_worker_ipc()
    .await
    .map_err(map_worker_open_failure)?;

    let mut authentication_root = Zeroizing::new([0_u8; 32]);
    getrandom::fill(authentication_root.as_mut()).map_err(|_| FailureCode::PublicApiSetup)?;
    let credentials = Arc::new(
        server.public_api_credential_manager(PublicApiAuthenticationKey::new(*authentication_root)),
    );
    let primary_grant = application_grant(request.grant.clone())?;
    let approval_broker_grant = request
        .approval_broker_grant
        .clone()
        .map(|grant| approval_broker_grant(&request.grant, grant))
        .transpose()?;
    let public_directory = prepare_public_directory(&instance_dir)?;
    let tls =
        TlsIdentity::from_seed(TlsKeySeed::random().map_err(|_| FailureCode::PublicApiSetup)?)
            .map_err(|_| FailureCode::PublicApiSetup)?;
    let options = PublicApiHostOptions::new(
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        instance_id,
        public_directory.join(DESCRIPTOR_FILENAME),
        public_directory.join(CERTIFICATE_FILENAME),
        tls,
        credentials.as_ref(),
    )
    .map_err(|_| FailureCode::PublicApiSetup)?
    .with_deployment_mode(PublicApiDeploymentMode::Sidecar);
    let server = server
        .enable_public_api(options)
        .await
        .map_err(|_| FailureCode::PublicApiSetup)?;
    let metadata = server
        .public_api_ready_metadata()
        .ok_or(FailureCode::PublicApiSetup)?;
    let certificate_pem = std::str::from_utf8(metadata.certificate_pem())
        .map_err(|_| FailureCode::PublicApiSetup)?
        .to_owned();
    let grants = std::iter::once(primary_grant)
        .chain(approval_broker_grant)
        .collect::<Vec<_>>();
    let mut issued = credentials
        .issue_pending_batch(&grants)
        .map_err(|_| FailureCode::PublicApiSetup)?;
    let primary_issued = issued.remove(0);
    let approval_broker_issued = issued.pop();
    let credential_id = primary_issued.credential_id().to_owned();
    let approval_broker_credential_id = approval_broker_issued
        .as_ref()
        .map(|credential| credential.credential_id().to_owned());
    let credential_ids = std::iter::once(credential_id.clone())
        .chain(approval_broker_credential_id.clone())
        .collect::<Vec<_>>();
    let bearer = match SecretString::new(primary_issued.expose_token().to_owned()) {
        Ok(bearer) => bearer,
        Err(_) => {
            let _ = credentials.revoke_batch(&credential_ids);
            return Err(FailureCode::PublicApiSetup);
        }
    };
    let approval_broker_bearer = match approval_broker_issued
        .as_ref()
        .map(|credential| SecretString::new(credential.expose_token().to_owned()))
        .transpose()
    {
        Ok(bearer) => bearer,
        Err(_) => {
            let _ = credentials.revoke_batch(&credential_ids);
            return Err(FailureCode::PublicApiSetup);
        }
    };
    let ready = ReadyResponse {
        protocol_version: PROTOCOL_VERSION,
        exchange_id: request.exchange_id.clone(),
        instance_id: metadata.instance_id().to_string(),
        api_major: request.api_major,
        deployment_mode: "sidecar".into(),
        endpoint: metadata.endpoint().to_owned(),
        certificate_pem,
        certificate_sha256: metadata.certificate_sha256().to_owned(),
        credential_id: credential_id.clone(),
        bearer,
        approval_broker_credential_id: approval_broker_credential_id.clone(),
        approval_broker_bearer,
    };
    if ready.validate().is_err() {
        let _ = credentials.revoke_batch(&credential_ids);
        return Err(FailureCode::PublicApiSetup);
    }
    let ready_result = {
        let mut output = std::io::stdout().lock();
        write_frame(&mut output, &ChildFrame::Ready(ready))
    };
    if ready_result.is_err() {
        let _ = credentials.revoke_batch(&credential_ids);
        return Err(FailureCode::PublicApiSetup);
    }
    drop(primary_issued);
    drop(approval_broker_issued);

    let ack = match read_frame::<_, ParentFrame>(input) {
        Ok(ParentFrame::Ack(ack)) => ack,
        _ => {
            let _ = credentials.revoke_batch(&credential_ids);
            return Err(FailureCode::CredentialActivation);
        }
    };
    if let Err(error) = validate_ack(
        &ack,
        &request.exchange_id,
        &credential_id,
        approval_broker_credential_id.as_deref(),
    ) {
        let _ = credentials.revoke_batch(&credential_ids);
        return Err(error);
    }
    match credentials.activate_batch(&credential_ids) {
        Ok(true) => {}
        Ok(false) | Err(_) => {
            let _ = credentials.revoke_batch(&credential_ids);
            return Err(FailureCode::CredentialActivation);
        }
    }
    let activated_result = {
        let mut output = std::io::stdout().lock();
        write_frame(
            &mut output,
            &ChildFrame::Activated(ActivatedResponse {
                protocol_version: PROTOCOL_VERSION,
                exchange_id: request.exchange_id,
                credential_id: credential_id.clone(),
                approval_broker_credential_id: approval_broker_credential_id.clone(),
            }),
        )
    };
    if activated_result.is_err() {
        let _ = credentials.revoke_batch(&credential_ids);
        return Err(FailureCode::CredentialActivation);
    }

    let (guardian_tx, guardian_rx) = tokio::sync::oneshot::channel();
    if std::thread::Builder::new()
        .name("colossus-sidecar-guardian".into())
        .spawn(move || {
            let mut input = std::io::stdin().lock();
            let mut byte = [0_u8; 1];
            let _ = input.read(&mut byte);
            let _ = guardian_tx.send(());
        })
        .is_err()
    {
        let _ = credentials.revoke_batch(&credential_ids);
        return Err(FailureCode::RuntimeFailed);
    }
    let shutdown_credentials = Arc::clone(&credentials);
    let guardian_credential_ids = credential_ids.clone();
    let serve_result = server
        .serve_until(async move {
            let _ = guardian_rx.await;
            let _ = shutdown_credentials.revoke_batch(&guardian_credential_ids);
        })
        .await;
    // `serve_until` may return before guardian EOF (for example after a transport or
    // runtime failure). Revoke idempotently on every exit path so neither active
    // bootstrap credential survives the supervised sidecar process.
    let _ = credentials.revoke_batch(&credential_ids);
    serve_result.map_err(|_| FailureCode::RuntimeFailed)
}

fn managed_runtime_config(
    managed: &ManagedRuntimeConfig,
    instance_id: Uuid,
    instance_dir: &Path,
    ca_bundle_path: Option<&Path>,
) -> Result<RuntimeConfig, FailureCode> {
    managed
        .validate()
        .map_err(|_| FailureCode::InvalidConfiguration)?;
    let mut config = RuntimeConfig::offline_template(instance_dir.join("state.redb"));
    config.storage.keys = KeyConfig::Platform {
        service: MANAGED_KEYRING_SERVICE.into(),
        journal_key_id: format!("journal-{instance_id}"),
        signing_key_id: format!("checkpoint-{instance_id}"),
    };
    let access_profile = match managed.access_profile {
        ManagedAccessProfile::Minimal => AccessProfile::Minimal,
        ManagedAccessProfile::Development => AccessProfile::Development,
        ManagedAccessProfile::AllowAll => AccessProfile::AllowAll,
        ManagedAccessProfile::Pinned => AccessProfile::Pinned,
    };
    config.set_access_profile(access_profile);
    config.network.ca_bundle_path = ca_bundle_path.map(Path::to_path_buf);
    if matches!(
        managed.access_profile,
        ManagedAccessProfile::Development | ManagedAccessProfile::AllowAll
    ) {
        config.set_sandbox_profile("workspace-development");
    }
    config.providers = ProvidersConfig {
        profiles: managed
            .providers
            .iter()
            .map(|provider| {
                let kind = match provider.kind {
                    ManagedProviderKind::Echo => ProviderKind::Echo,
                    ManagedProviderKind::OpenAiResponses => ProviderKind::OpenAiResponses,
                    ManagedProviderKind::OpenAiCompatible => ProviderKind::OpenAiCompatible,
                };
                (
                    provider.profile.clone(),
                    ProviderProfileConfig {
                        kind,
                        base_url: provider.base_url.clone(),
                        credential_reference: provider
                            .credential_id
                            .as_ref()
                            .map(|identifier| format!("host:{identifier}")),
                        timeout_ms: provider.timeout_ms,
                    },
                )
            })
            .collect(),
    };
    config.models = ModelsConfig {
        profiles: managed
            .models
            .iter()
            .map(|model| {
                (
                    model.profile.clone(),
                    ModelProfileConfig {
                        provider_profile: model.provider_profile.clone(),
                        model: model.model.clone(),
                        context_window_tokens: model.context_window_tokens,
                        max_output_tokens: model.max_output_tokens,
                        capabilities: ModelCapabilities {
                            tool_calls: model.capabilities.tool_calls,
                            streaming: model.capabilities.streaming,
                        },
                        reasoning_effort: None,
                    },
                )
            })
            .collect(),
        roles: managed.roles.clone(),
    };
    config.sandbox.network_destinations = managed
        .providers
        .iter()
        .filter_map(|provider| provider.base_url.as_deref())
        .map(|base_url| {
            url::Url::parse(base_url)
                .map_err(|_| FailureCode::InvalidConfiguration)
                .map(|url| url.origin().ascii_serialization())
        })
        .collect::<Result<BTreeSet<_>, _>>()?
        .into_iter()
        .collect();
    config.memory.index_path = Some(instance_dir.join("memory-index"));
    config.workflows.user = instance_dir.join("workflows");
    config.skills.user = instance_dir.join("skills");
    config.packs.install_root = instance_dir.join("packs");
    let yaml = config
        .to_yaml()
        .map_err(|_| FailureCode::InvalidConfiguration)?;
    RuntimeConfig::from_yaml(&yaml).map_err(|_| FailureCode::InvalidConfiguration)
}

fn install_private_ca_bundle(
    source_path: &Path,
    instance_dir: &Path,
) -> Result<PathBuf, FailureCode> {
    let bytes = read_private_ca_bundle(source_path)?;
    colossus_network::AdditionalRootCertificates::from_pem_bundle(&bytes)
        .map_err(|_| FailureCode::InvalidConfiguration)?;
    let destination = instance_dir.join(MANAGED_CA_BUNDLE_FILENAME);
    persist_private_bytes(instance_dir, &destination, &bytes)?;
    Ok(destination)
}

#[cfg(unix)]
fn read_private_ca_bundle(path: &Path) -> Result<Vec<u8>, FailureCode> {
    let canonical = std::fs::canonicalize(path).map_err(|_| FailureCode::InvalidConfiguration)?;
    let before = std::fs::symlink_metadata(path).map_err(|_| FailureCode::InvalidConfiguration)?;
    if canonical != path
        || !before.file_type().is_file()
        || before.uid() != rustix::process::getuid().as_raw()
        || before.mode() & 0o077 != 0
        || before.len() > MAX_CA_BUNDLE_BYTES
    {
        return Err(FailureCode::InvalidConfiguration);
    }
    let mut source = File::open(path).map_err(|_| FailureCode::InvalidConfiguration)?;
    let opened = source
        .metadata()
        .map_err(|_| FailureCode::InvalidConfiguration)?;
    if opened.dev() != before.dev() || opened.ino() != before.ino() {
        return Err(FailureCode::InvalidConfiguration);
    }
    let mut bytes = Vec::with_capacity(usize::try_from(before.len()).unwrap_or(0));
    std::io::Read::by_ref(&mut source)
        .take(MAX_CA_BUNDLE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| FailureCode::InvalidConfiguration)?;
    if bytes.len() as u64 > MAX_CA_BUNDLE_BYTES {
        return Err(FailureCode::InvalidConfiguration);
    }
    Ok(bytes)
}

#[cfg(windows)]
fn read_private_ca_bundle(path: &Path) -> Result<Vec<u8>, FailureCode> {
    let binding = colossus_windows_native::BoundPath::open_file(path)
        .map_err(|_| FailureCode::InvalidConfiguration)?;
    binding
        .validate_private_owner_dacl()
        .and_then(|()| binding.revalidate())
        .map_err(|_| FailureCode::InvalidConfiguration)?;
    let mut source = binding
        .try_clone_file()
        .map_err(|_| FailureCode::InvalidConfiguration)?;
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut source)
        .take(MAX_CA_BUNDLE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| FailureCode::InvalidConfiguration)?;
    if bytes.len() as u64 > MAX_CA_BUNDLE_BYTES {
        return Err(FailureCode::InvalidConfiguration);
    }
    binding
        .revalidate()
        .map_err(|_| FailureCode::InvalidConfiguration)?;
    Ok(bytes)
}

#[cfg(not(any(unix, windows)))]
fn read_private_ca_bundle(_path: &Path) -> Result<Vec<u8>, FailureCode> {
    Err(FailureCode::InvalidConfiguration)
}

#[cfg(unix)]
fn persist_private_bytes(
    instance_dir: &Path,
    destination: &Path,
    bytes: &[u8],
) -> Result<(), FailureCode> {
    let temporary = instance_dir.join(format!(".ca-bundle.{}.tmp", Uuid::now_v7()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|_| FailureCode::InvalidInstanceDirectory)?;
    if file.write_all(bytes).is_err() || file.sync_all().is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err(FailureCode::InvalidInstanceDirectory);
    }
    drop(file);
    if std::fs::rename(&temporary, destination).is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err(FailureCode::InvalidInstanceDirectory);
    }
    std::fs::File::open(instance_dir)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| FailureCode::InvalidInstanceDirectory)
}

#[cfg(windows)]
fn persist_private_bytes(
    instance_dir: &Path,
    destination: &Path,
    bytes: &[u8],
) -> Result<(), FailureCode> {
    let temporary = instance_dir.join(format!(".ca-bundle.{}.tmp", Uuid::now_v7()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| FailureCode::InvalidInstanceDirectory)?;
    if file.write_all(bytes).is_err() || file.sync_all().is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err(FailureCode::InvalidInstanceDirectory);
    }
    drop(file);
    if std::fs::rename(&temporary, destination).is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err(FailureCode::InvalidInstanceDirectory);
    }
    let binding = colossus_windows_native::BoundPath::open_file(destination)
        .map_err(|_| FailureCode::InvalidInstanceDirectory)?;
    binding
        .validate_private_owner_dacl()
        .and_then(|()| binding.revalidate())
        .map_err(|_| FailureCode::InvalidInstanceDirectory)
}

#[cfg(not(any(unix, windows)))]
fn persist_private_bytes(
    _instance_dir: &Path,
    _destination: &Path,
    _bytes: &[u8],
) -> Result<(), FailureCode> {
    Err(FailureCode::InvalidInstanceDirectory)
}

#[cfg(unix)]
fn persist_managed_config(instance_dir: &Path, config: &RuntimeConfig) -> Result<(), FailureCode> {
    let yaml = config
        .to_yaml()
        .map_err(|_| FailureCode::InvalidConfiguration)?;
    let destination = instance_dir.join(MANAGED_CONFIG_FILENAME);
    let temporary = instance_dir.join(format!(".{MANAGED_CONFIG_FILENAME}.{}.tmp", Uuid::now_v7()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|_| FailureCode::InvalidInstanceDirectory)?;
    if file.write_all(yaml.as_bytes()).is_err() || file.sync_all().is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err(FailureCode::InvalidInstanceDirectory);
    }
    drop(file);
    if std::fs::rename(&temporary, &destination).is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err(FailureCode::InvalidInstanceDirectory);
    }
    std::fs::File::open(instance_dir)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| FailureCode::InvalidInstanceDirectory)
}

#[cfg(windows)]
fn persist_managed_config(instance_dir: &Path, config: &RuntimeConfig) -> Result<(), FailureCode> {
    let yaml = config
        .to_yaml()
        .map_err(|_| FailureCode::InvalidConfiguration)?;
    let destination = instance_dir.join(MANAGED_CONFIG_FILENAME);
    let temporary = instance_dir.join(format!(".{MANAGED_CONFIG_FILENAME}.{}.tmp", Uuid::now_v7()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| FailureCode::InvalidInstanceDirectory)?;
    if file.write_all(yaml.as_bytes()).is_err() || file.sync_all().is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err(FailureCode::InvalidInstanceDirectory);
    }
    drop(file);
    if std::fs::rename(&temporary, &destination).is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err(FailureCode::InvalidInstanceDirectory);
    }
    let binding = colossus_windows_native::BoundPath::open_file(&destination)
        .map_err(|_| FailureCode::InvalidInstanceDirectory)?;
    binding
        .validate_private_owner_dacl()
        .and_then(|()| binding.revalidate())
        .map_err(|_| FailureCode::InvalidInstanceDirectory)
}

fn application_grant(
    grant: colossus_sidecar_protocol::BootstrapGrant,
) -> Result<ApplicationGrant, FailureCode> {
    let scopes = grant
        .scopes
        .into_iter()
        .map(ApiScope::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| FailureCode::InvalidBootstrap)?;
    ApplicationGrant::new(
        grant.application_id,
        ApplicationKind::Sidecar,
        scopes,
        grant.allowed_roles,
        grant.allowed_tools,
    )
    .map_err(|_| FailureCode::InvalidBootstrap)
}

fn approval_broker_grant(
    primary: &BootstrapGrant,
    broker: BootstrapGrant,
) -> Result<ApplicationGrant, FailureCode> {
    let primary_roles = primary
        .allowed_roles
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    if broker.application_id != primary.application_id
        || broker.scopes.as_slice() != [colossus_api::scopes::APPROVALS_RESPOND]
        || !broker.allowed_tools.is_empty()
        || primary
            .scopes
            .iter()
            .any(|scope| scope == colossus_api::scopes::APPROVALS_RESPOND)
        || broker
            .allowed_roles
            .iter()
            .any(|role| !primary_roles.contains(role))
    {
        return Err(FailureCode::InvalidBootstrap);
    }
    application_grant(broker)
}

fn validate_ack(
    ack: &AckRequest,
    exchange_id: &str,
    credential_id: &str,
    approval_broker_credential_id: Option<&str>,
) -> Result<(), FailureCode> {
    if ack.protocol_version != PROTOCOL_VERSION
        || ack.exchange_id != exchange_id
        || ack.credential_id != credential_id
        || ack.approval_broker_credential_id.as_deref() != approval_broker_credential_id
    {
        return Err(FailureCode::CredentialActivation);
    }
    Ok(())
}

fn send_failure(exchange_id: Option<String>, code: FailureCode) {
    let mut output = std::io::stdout().lock();
    let _ = write_frame(
        &mut output,
        &ChildFrame::Failed(FailureResponse {
            protocol_version: PROTOCOL_VERSION,
            exchange_id,
            code,
        }),
    );
    let _ = output.flush();
}

fn map_worker_open_failure(error: colossus_worker::WorkerError) -> FailureCode {
    match error {
        colossus_worker::WorkerError::Runtime(RuntimeError::Store(StoreError::WriterLeaseHeld))
        | colossus_worker::WorkerError::Store(StoreError::WriterLeaseHeld) => {
            FailureCode::WorkspaceBusy
        }
        colossus_worker::WorkerError::Runtime(RuntimeError::Store(
            StoreError::WorkspaceIdentityChanged,
        ))
        | colossus_worker::WorkerError::Store(StoreError::WorkspaceIdentityChanged) => {
            FailureCode::InvalidWorkspace
        }
        colossus_worker::WorkerError::Runtime(RuntimeError::Config(_)) => {
            FailureCode::InvalidConfiguration
        }
        _ => FailureCode::RuntimeFailed,
    }
}

/// Child-side binding of the workspace named by the private bootstrap request.
///
/// This descriptor is independent of the parent's descriptor. Reproducing the opaque
/// identity proves both processes opened the same Unix kernel object; retaining it also
/// lets `fchdir` avoid a second path lookup. Runtime lease acquisition performs the
/// final path reopen with the derived expected token before any adapter is composed.
#[cfg(unix)]
struct BoundWorkspace {
    directory: File,
    canonical_path: PathBuf,
    device: u64,
    inode: u64,
    #[cfg(target_os = "macos")]
    birth_seconds: i64,
    #[cfg(target_os = "macos")]
    birth_nanoseconds: i64,
}

#[cfg(windows)]
struct BoundWorkspace {
    binding: colossus_windows_native::BoundPath,
    canonical_path: PathBuf,
}

#[cfg(unix)]
impl BoundWorkspace {
    fn open(path: &Path, expected: &BootstrapWorkspaceIdentity) -> Result<Self, FailureCode> {
        use rustix::fs::{Mode, OFlags, open};

        expected
            .validate()
            .map_err(|_| FailureCode::InvalidWorkspace)?;
        let canonical_path =
            std::fs::canonicalize(path).map_err(|_| FailureCode::InvalidWorkspace)?;
        if canonical_path.as_os_str() != path.as_os_str() {
            return Err(FailureCode::InvalidWorkspace);
        }
        let before = std::fs::symlink_metadata(&canonical_path)
            .map_err(|_| FailureCode::InvalidWorkspace)?;
        if before.file_type().is_symlink() || !before.is_dir() {
            return Err(FailureCode::InvalidWorkspace);
        }
        let directory = File::from(
            open(
                &canonical_path,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .map_err(|_| FailureCode::InvalidWorkspace)?,
        );
        let opened = directory
            .metadata()
            .map_err(|_| FailureCode::InvalidWorkspace)?;
        let after = std::fs::symlink_metadata(&canonical_path)
            .map_err(|_| FailureCode::InvalidWorkspace)?;
        if !opened.is_dir()
            || after.file_type().is_symlink()
            || !after.is_dir()
            || before.dev() != opened.dev()
            || before.ino() != opened.ino()
            || after.dev() != opened.dev()
            || after.ino() != opened.ino()
        {
            return Err(FailureCode::InvalidWorkspace);
        }
        #[cfg(target_os = "macos")]
        {
            use std::os::macos::fs::MetadataExt as _;

            if opened.st_birthtime() <= 0
                || !(0..1_000_000_000).contains(&opened.st_birthtime_nsec())
                || before.st_birthtime() != opened.st_birthtime()
                || before.st_birthtime_nsec() != opened.st_birthtime_nsec()
                || after.st_birthtime() != opened.st_birthtime()
                || after.st_birthtime_nsec() != opened.st_birthtime_nsec()
            {
                return Err(FailureCode::InvalidWorkspace);
            }
        }
        let device = opened.dev();
        let inode = opened.ino();
        #[cfg(target_os = "macos")]
        let actual = BootstrapWorkspaceIdentity::from_macos_parts(
            device,
            inode,
            {
                use std::os::macos::fs::MetadataExt as _;
                opened.st_birthtime()
            },
            {
                use std::os::macos::fs::MetadataExt as _;
                opened.st_birthtime_nsec()
            },
        )
        .map_err(|_| FailureCode::InvalidWorkspace)?;
        #[cfg(not(target_os = "macos"))]
        let actual = BootstrapWorkspaceIdentity::from_unix_parts(device, inode);
        if actual != *expected {
            return Err(FailureCode::InvalidWorkspace);
        }
        Ok(Self {
            directory,
            canonical_path,
            device,
            inode,
            #[cfg(target_os = "macos")]
            birth_seconds: {
                use std::os::macos::fs::MetadataExt as _;
                opened.st_birthtime()
            },
            #[cfg(target_os = "macos")]
            birth_nanoseconds: {
                use std::os::macos::fs::MetadataExt as _;
                opened.st_birthtime_nsec()
            },
        })
    }

    fn runtime_identity(&self) -> Result<WorkspaceIdentityToken, FailureCode> {
        #[cfg(target_os = "macos")]
        {
            WorkspaceIdentityToken::from_macos_parts(
                self.device,
                self.inode,
                self.birth_seconds,
                self.birth_nanoseconds,
            )
            .ok_or(FailureCode::InvalidWorkspace)
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(WorkspaceIdentityToken::from_unix_parts(
                self.device,
                self.inode,
            ))
        }
    }
}

#[cfg(windows)]
impl BoundWorkspace {
    fn open(path: &Path, expected: &BootstrapWorkspaceIdentity) -> Result<Self, FailureCode> {
        expected
            .validate_current()
            .map_err(|_| FailureCode::InvalidWorkspace)?;
        let binding = colossus_windows_native::BoundPath::open_directory(path)
            .map_err(|_| FailureCode::InvalidWorkspace)?;
        let identity = binding.identity();
        let actual = BootstrapWorkspaceIdentity::from_windows_parts(
            identity.volume_serial_number,
            identity.file_id,
        )
        .map_err(|_| FailureCode::InvalidWorkspace)?;
        if actual != *expected {
            return Err(FailureCode::InvalidWorkspace);
        }
        let canonical_path = binding.canonical_path().to_owned();
        Ok(Self {
            binding,
            canonical_path,
        })
    }

    fn runtime_identity(&self) -> Result<WorkspaceIdentityToken, FailureCode> {
        self.binding
            .revalidate()
            .map_err(|_| FailureCode::InvalidWorkspace)?;
        let identity = self.binding.identity();
        WorkspaceIdentityToken::from_windows_parts(identity.volume_serial_number, identity.file_id)
            .ok_or(FailureCode::InvalidWorkspace)
    }
}

#[cfg(unix)]
fn private_directory(path: &Path) -> Result<PathBuf, FailureCode> {
    let canonical =
        std::fs::canonicalize(path).map_err(|_| FailureCode::InvalidInstanceDirectory)?;
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| FailureCode::InvalidInstanceDirectory)?;
    if canonical != path
        || !metadata.file_type().is_dir()
        || metadata.uid() != rustix::process::getuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(FailureCode::InvalidInstanceDirectory);
    }
    Ok(canonical)
}

#[cfg(windows)]
fn private_directory(path: &Path) -> Result<PathBuf, FailureCode> {
    let binding = colossus_windows_native::BoundPath::open_directory(path)
        .map_err(|_| FailureCode::InvalidInstanceDirectory)?;
    binding
        .validate_private_owner_dacl()
        .and_then(|()| binding.revalidate())
        .map_err(|_| FailureCode::InvalidInstanceDirectory)?;
    Ok(binding.canonical_path().to_owned())
}

#[cfg(not(any(unix, windows)))]
fn private_directory(_path: &Path) -> Result<PathBuf, FailureCode> {
    Err(FailureCode::InvalidInstanceDirectory)
}

#[cfg(unix)]
fn prepare_public_directory(instance_dir: &Path) -> Result<PathBuf, FailureCode> {
    let path = instance_dir.join(PUBLIC_API_DIRECTORY);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir()
                || metadata.uid() != rustix::process::getuid().as_raw()
                || metadata.mode() & 0o077 != 0
            {
                return Err(FailureCode::InvalidInstanceDirectory);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .map_err(|_| FailureCode::InvalidInstanceDirectory)?;
        }
        Err(_) => return Err(FailureCode::InvalidInstanceDirectory),
    }
    let canonical =
        std::fs::canonicalize(&path).map_err(|_| FailureCode::InvalidInstanceDirectory)?;
    if canonical.parent() != Some(instance_dir) {
        return Err(FailureCode::InvalidInstanceDirectory);
    }
    Ok(canonical)
}

#[cfg(windows)]
fn prepare_public_directory(instance_dir: &Path) -> Result<PathBuf, FailureCode> {
    let path = instance_dir.join(PUBLIC_API_DIRECTORY);
    match std::fs::create_dir(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(FailureCode::InvalidInstanceDirectory),
    }
    let binding = colossus_windows_native::BoundPath::open_directory(&path)
        .map_err(|_| FailureCode::InvalidInstanceDirectory)?;
    binding
        .validate_private_owner_dacl()
        .and_then(|()| binding.revalidate())
        .map_err(|_| FailureCode::InvalidInstanceDirectory)?;
    if binding.canonical_path().parent() != Some(instance_dir) {
        return Err(FailureCode::InvalidInstanceDirectory);
    }
    Ok(binding.canonical_path().to_owned())
}

#[cfg(not(any(unix, windows)))]
fn persist_managed_config(
    _instance_dir: &Path,
    _config: &RuntimeConfig,
) -> Result<(), FailureCode> {
    Err(FailureCode::InvalidInstanceDirectory)
}

#[cfg(not(any(unix, windows)))]
fn prepare_public_directory(_instance_dir: &Path) -> Result<PathBuf, FailureCode> {
    Err(FailureCode::InvalidInstanceDirectory)
}

#[cfg(windows)]
fn connect_windows_bootstrap_pipe() -> Result<File, ()> {
    const PIPE_ENVIRONMENT: &str = "COLOSSUS_WINDOWS_BOOTSTRAP_PIPE_V1";
    const PARENT_ENVIRONMENT: &str = "COLOSSUS_WINDOWS_BOOTSTRAP_PARENT_PID_V1";
    const PIPE_PREFIX: &str = r"\\.\pipe\colossus-managed-";

    let pipe_name = std::env::var(PIPE_ENVIRONMENT).map_err(|_| ())?;
    let parent_process = std::env::var(PARENT_ENVIRONMENT)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value != 0)
        .ok_or(())?;
    if pipe_name.len() > 256
        || !pipe_name.starts_with(PIPE_PREFIX)
        || !pipe_name[PIPE_PREFIX.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        return Err(());
    }
    let pipe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(pipe_name)
        .map_err(|_| ())?;
    colossus_windows_native::validate_named_pipe_server(&pipe, parent_process).map_err(|_| ())?;
    colossus_windows_native::install_bootstrap_pipe_as_standard_io(&pipe).map_err(|_| ())?;
    Ok(pipe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_sidecar_protocol::ManagedProviderConfig;

    #[cfg(unix)]
    fn bootstrap_workspace_identity(
        metadata: &std::fs::Metadata,
        inode: u64,
    ) -> BootstrapWorkspaceIdentity {
        #[cfg(target_os = "macos")]
        {
            use std::os::macos::fs::MetadataExt as _;

            BootstrapWorkspaceIdentity::from_macos_parts(
                metadata.dev(),
                inode,
                metadata.st_birthtime(),
                metadata.st_birthtime_nsec(),
            )
            .expect("current workspace identity")
        }
        #[cfg(not(target_os = "macos"))]
        {
            BootstrapWorkspaceIdentity::from_unix_parts(metadata.dev(), inode)
        }
    }

    #[test]
    fn ack_must_bind_exact_exchange_and_credential() {
        let exchange = Uuid::now_v7().to_string();
        let credential = Uuid::now_v7().to_string();
        let broker = Uuid::now_v7().to_string();
        let ack = AckRequest {
            protocol_version: PROTOCOL_VERSION,
            exchange_id: exchange.clone(),
            credential_id: credential.clone(),
            approval_broker_credential_id: Some(broker.clone()),
        };
        assert_eq!(
            validate_ack(&ack, &exchange, &credential, Some(&broker)),
            Ok(())
        );
        assert_eq!(
            validate_ack(
                &ack,
                &Uuid::now_v7().to_string(),
                &credential,
                Some(&broker)
            ),
            Err(FailureCode::CredentialActivation)
        );
        assert_eq!(
            validate_ack(&ack, &exchange, &credential, None),
            Err(FailureCode::CredentialActivation)
        );
    }

    #[test]
    fn approval_broker_grant_cannot_widen_primary_authority() {
        let primary = BootstrapGrant {
            application_id: "app:desktop".into(),
            scopes: vec![
                colossus_api::scopes::RUNS_EXECUTE.into(),
                colossus_api::scopes::RUNS_READ.into(),
                colossus_api::scopes::RUNS_CONTROL.into(),
                colossus_api::scopes::PROMPTS_RESPOND.into(),
            ],
            allowed_roles: vec!["primary".into()],
            allowed_tools: vec!["shell.run".into()],
        };
        let broker = BootstrapGrant {
            application_id: primary.application_id.clone(),
            scopes: vec![colossus_api::scopes::APPROVALS_RESPOND.into()],
            allowed_roles: primary.allowed_roles.clone(),
            allowed_tools: Vec::new(),
        };
        approval_broker_grant(&primary, broker.clone()).expect("bounded broker");

        let mut widened = broker.clone();
        widened
            .scopes
            .push(colossus_api::scopes::RUNS_CONTROL.into());
        assert_eq!(
            approval_broker_grant(&primary, widened).err(),
            Some(FailureCode::InvalidBootstrap)
        );
        let mut tooled = broker.clone();
        tooled.allowed_tools.push("shell.run".into());
        assert_eq!(
            approval_broker_grant(&primary, tooled).err(),
            Some(FailureCode::InvalidBootstrap)
        );
        let mut other_app = broker;
        other_app.application_id = "app:other".into();
        assert_eq!(
            approval_broker_grant(&primary, other_app).err(),
            Some(FailureCode::InvalidBootstrap)
        );
    }

    #[test]
    fn writer_lease_conflict_maps_to_sanitized_busy_code() {
        let error =
            colossus_worker::WorkerError::Runtime(RuntimeError::Store(StoreError::WriterLeaseHeld));
        assert_eq!(map_worker_open_failure(error), FailureCode::WorkspaceBusy);
        assert_eq!(
            map_worker_open_failure(colossus_worker::WorkerError::Store(
                StoreError::WriterLeaseHeld
            )),
            FailureCode::WorkspaceBusy
        );
    }

    #[test]
    fn runtime_workspace_identity_drift_maps_to_sanitized_workspace_code() {
        let error = colossus_worker::WorkerError::Runtime(RuntimeError::Store(
            StoreError::WorkspaceIdentityChanged,
        ));
        assert_eq!(
            map_worker_open_failure(error),
            FailureCode::InvalidWorkspace
        );
        assert_eq!(
            map_worker_open_failure(colossus_worker::WorkerError::Store(
                StoreError::WorkspaceIdentityChanged
            )),
            FailureCode::InvalidWorkspace
        );
    }

    #[test]
    fn runtime_configuration_failure_maps_to_sanitized_configuration_code() {
        let error = colossus_worker::WorkerError::Runtime(RuntimeError::Config(
            "private diagnostic".into(),
        ));
        assert_eq!(
            map_worker_open_failure(error),
            FailureCode::InvalidConfiguration
        );
    }

    #[cfg(unix)]
    #[test]
    fn child_workspace_binding_rejects_wrong_parent_identity() {
        let workspace = tempfile::tempdir().expect("workspace");
        let canonical = std::fs::canonicalize(workspace.path()).expect("canonical workspace");
        let metadata = std::fs::symlink_metadata(&canonical).expect("workspace metadata");
        let wrong = bootstrap_workspace_identity(&metadata, metadata.ino().wrapping_add(1));
        assert!(matches!(
            BoundWorkspace::open(&canonical, &wrong),
            Err(FailureCode::InvalidWorkspace)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn child_workspace_binding_rejects_renamed_workspace_replacement() {
        let root = tempfile::tempdir().expect("root");
        let workspace = root.path().join("workspace");
        let moved = root.path().join("workspace-moved");
        std::fs::create_dir(&workspace).expect("workspace");
        let canonical = std::fs::canonicalize(&workspace).expect("canonical workspace");
        let metadata = std::fs::symlink_metadata(&canonical).expect("workspace metadata");
        let expected = bootstrap_workspace_identity(&metadata, metadata.ino());
        let retained = BoundWorkspace::open(&canonical, &expected).expect("initial binding");

        std::fs::rename(&workspace, &moved).expect("move workspace");
        std::fs::create_dir(&workspace).expect("replacement workspace");

        let retained_metadata = retained
            .directory
            .metadata()
            .expect("retained workspace metadata");
        assert_eq!(retained_metadata.dev(), metadata.dev());
        assert_eq!(retained_metadata.ino(), metadata.ino());
        assert!(matches!(
            BoundWorkspace::open(&canonical, &expected),
            Err(FailureCode::InvalidWorkspace)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn generated_config_is_owner_private_and_contains_only_host_reference() {
        let instance = tempfile::tempdir().expect("instance");
        let managed = ManagedRuntimeConfig {
            access_profile: ManagedAccessProfile::Development,
            providers: vec![ManagedProviderConfig {
                profile: "openrouter".into(),
                kind: ManagedProviderKind::OpenAiCompatible,
                base_url: Some("https://openrouter.ai/api/v1".into()),
                credential_id: Some("provider-main".into()),
                timeout_ms: 120_000,
            }],
            models: vec![colossus_sidecar_protocol::ManagedModelConfig {
                profile: "main".into(),
                provider_profile: "openrouter".into(),
                model: "deepseek/deepseek-v4-flash".into(),
                context_window_tokens: 64_000,
                max_output_tokens: 8_000,
                capabilities: colossus_sidecar_protocol::ManagedModelCapabilities {
                    tool_calls: true,
                    streaming: true,
                },
            }],
            roles: BTreeMap::from([("primary".into(), "main".into())]),
        };
        let config = managed_runtime_config(&managed, Uuid::now_v7(), instance.path(), None)
            .expect("managed config");
        persist_managed_config(instance.path(), &config).expect("persist config");
        let path = instance.path().join(MANAGED_CONFIG_FILENAME);
        let contents = std::fs::read_to_string(&path).expect("read config");
        assert!(contents.contains("host:provider-main"));
        assert!(!contents.contains("provider-secret-value"));
        assert_eq!(
            std::fs::symlink_metadata(path).expect("metadata").mode() & 0o077,
            0
        );
    }
}
