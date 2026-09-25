//! Catalog discovery uses an isolated managed worker before any model is selected.

use colossus_sdk::{
    ApiMajor, AppPrivateInstanceDir, Colossus, ManagedAccessProfile, ManagedExecutionBoundary,
    ManagedProviderConfig, ManagedRuntimeConfig, NativeSidecarLifecycle, Secret,
    SidecarBootstrapConfig, SidecarHostCredential, SidecarOptions,
};
use colossus_worker_protocol::WorkerControlClient;

use super::{
    classify_sdk, managed_worker_endpoint, provider_kind, self_test_grant, self_test_instance_id,
};
use crate::{
    bundle::VerifiedBundle,
    desktop_credentials::DesktopCredentials,
    desktop_dto::RuntimeFailureCodeDto,
    desktop_settings::{DesktopSettings, ProviderKindSetting, ProviderSetting, SettingsStore},
    dto::CommandErrorDto,
    state::AppState,
    terminal::TerminalWorkerAuthentication,
};

pub(crate) async fn discover_provider_models(
    state: &AppState,
    store: &SettingsStore,
    settings: &DesktopSettings,
    provider: &ProviderSetting,
) -> Result<Vec<colossus_contracts::ProviderModelInfo>, CommandErrorDto> {
    let storage = store.self_test_storage()?;
    let workspace = crate::desktop_settings::validate_workspace(&storage.workspace)?;
    let identity = workspace.identity.ok_or_else(catalog_error)?;
    let instance_id = self_test_instance_id(&storage.instance_dir)?;
    let endpoint = managed_worker_endpoint(&storage.instance_dir).map_err(|_| catalog_error())?;
    let bundle = VerifiedBundle::load()?;
    let options = SidecarOptions::new(
        instance_id,
        AppPrivateInstanceDir::new(storage.instance_dir).map_err(CommandErrorDto::from_sdk)?,
        bundle.sidecar,
        ApiMajor::new(1).map_err(CommandErrorDto::from_sdk)?,
    )
    .map_err(CommandErrorDto::from_sdk)?;
    let authentication =
        TerminalWorkerAuthentication::random().map_err(CommandErrorDto::from_terminal)?;
    let credentials = match provider.credential_id.as_deref() {
        Some(id) => vec![
            SidecarHostCredential::new(
                id,
                DesktopCredentials::for_settings(state, store)?
                    .read(id)
                    .await?,
            )
            .map_err(CommandErrorDto::from_sdk)?,
        ],
        None => Vec::new(),
    };
    let colossus_home = store.home_root()?.to_owned();
    let mut bootstrap = SidecarBootstrapConfig::new(
        workspace.path,
        catalog_runtime(provider),
        self_test_grant().map_err(CommandErrorDto::from_sdk)?,
    )
    .and_then(|bootstrap| bootstrap.with_expected_workspace_identity(identity))
    .and_then(|bootstrap| bootstrap.with_colossus_home(colossus_home))
    .and_then(|bootstrap| bootstrap.with_host_credentials(credentials))
    .and_then(|bootstrap| {
        bootstrap
            .with_worker_ipc_authentication(Secret::new(authentication.copy_secret().to_vec())?)
    })
    .map_err(CommandErrorDto::from_sdk)?;
    if let Some(bundle) = settings.additional_ca_bundle.as_ref() {
        bootstrap = bootstrap
            .with_additional_ca_bundle_path(store.ca_bundle_path(bundle)?)
            .map_err(CommandErrorDto::from_sdk)?;
    }
    if provider.kind == ProviderKindSetting::Codex {
        bootstrap = bootstrap
            .with_codex_auth_path(crate::codex_auth::require_codex_auth_path()?)
            .map_err(CommandErrorDto::from_sdk)?;
    }
    let worker = WorkerControlClient::new(endpoint, authentication.copy_secret())
        .map_err(|_| catalog_error())?;
    let lifecycle = NativeSidecarLifecycle::new(bootstrap);
    let client = Colossus::start_sidecar(&lifecycle, options)
        .await
        .map_err(|error| classify_sdk(error, RuntimeFailureCodeDto::Provider).0)?;
    let result = worker
        .provider_models(&provider.profile)
        .await
        .map_err(|_| catalog_error());
    let close = client.close().await.map_err(CommandErrorDto::from_sdk);
    let value = result?;
    close?;
    serde_json::from_value(value).map_err(|_| catalog_error())
}

fn catalog_runtime(provider: &ProviderSetting) -> ManagedRuntimeConfig {
    let mut runtime = ManagedRuntimeConfig::echo(ManagedAccessProfile::Minimal)
        .with_execution_boundary(ManagedExecutionBoundary::OfflineIsolated);
    runtime.providers.push(ManagedProviderConfig {
        profile: provider.profile.clone(),
        kind: provider_kind(provider.kind),
        base_url: (provider.kind != ProviderKindSetting::Codex).then(|| provider.base_url.clone()),
        credential_id: provider.credential_id.clone(),
        timeout_ms: 30_000,
        chat_completions_output_token_parameter: None,
    });
    runtime
}

fn catalog_error() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "provider_catalog_unavailable",
        "Models could not be loaded. Check the provider URL and credential, then retry. You can also enter a model ID manually.",
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_needs_no_selected_model_and_grants_no_extra_tools() {
        let runtime = catalog_runtime(&ProviderSetting {
            profile: "setup-provider".into(),
            kind: ProviderKindSetting::Compatible,
            base_url: "https://models.example.test/v1".into(),
            credential_id: Some("credential-one".into()),
            timeout_ms: None,
        });
        runtime.validate().expect("valid catalog bootstrap");
        assert_eq!(runtime.roles["primary"], "echo");
        assert_eq!(runtime.models.len(), 1);
        assert_eq!(
            runtime.providers[1].credential_id.as_deref(),
            Some("credential-one")
        );
        assert!(runtime.mcp_servers.is_empty());
        assert_eq!(runtime.access_profile, ManagedAccessProfile::Minimal);
        assert_eq!(
            runtime.execution_boundary,
            ManagedExecutionBoundary::OfflineIsolated
        );
    }
}

#[cfg(test)]
#[path = "provider_catalog_tests.rs"]
mod native_tests;
