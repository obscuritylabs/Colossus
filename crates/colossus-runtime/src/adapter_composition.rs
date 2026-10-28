use super::*;

pub(super) fn configured_search_profile_with_authority(
    name: &str,
    config: &SearchProfileConfig,
    resource_authority: ResourceAuthority,
) -> Result<SearchProfile, RuntimeError> {
    let profile = match config {
        SearchProfileConfig::Searxng {
            endpoint,
            credential_reference,
            auth_header,
            user_agent,
            timeout_ms,
        } => SearchProfile::new_with_resource_authority(
            name,
            SearchKind::Searxng,
            endpoint,
            credential_reference.clone(),
            Some(auth_header.clone()),
            user_agent,
            *timeout_ms,
            resource_authority,
        ),
        SearchProfileConfig::SerpApi {
            endpoint,
            credential_reference,
            user_agent,
            timeout_ms,
        } => SearchProfile::new_with_resource_authority(
            name,
            SearchKind::SerpApi,
            endpoint,
            Some(credential_reference.clone()),
            None,
            user_agent,
            *timeout_ms,
            resource_authority,
        ),
    }?;
    Ok(profile)
}

pub(super) fn search_registry(
    config: &RuntimeConfig,
    tls_roots: &AdditionalRootCertificates,
    credentials: Arc<dyn CredentialResolver>,
) -> Result<SearchRegistry, RuntimeError> {
    let resource_authority = configured_resource_authority(&config.sandbox);
    let config = &config.search;
    let profiles = config
        .profiles
        .iter()
        .map(|(name, profile)| {
            configured_search_profile_with_authority(name, profile, resource_authority)
                .map(|profile| SearchExecutor::with_credentials(profile, Arc::clone(&credentials)))
                .map(|executor| executor.with_tls_roots(tls_roots.clone()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    SearchRegistry::new(profiles, config.roles.clone()).map_err(Into::into)
}

pub(super) fn validate_search_config(config: &RuntimeConfig) -> Result<(), RuntimeError> {
    if config
        .search
        .roles
        .keys()
        .any(|role| !matches!(role.as_str(), "agent" | "research"))
    {
        return Err(RuntimeError::Config(
            "search roles must be exactly agent or research".into(),
        ));
    }
    for (name, profile) in &config.search.profiles {
        let profile = configured_search_profile_with_authority(
            name,
            profile,
            configured_resource_authority(&config.sandbox),
        )?;
        let origin = profile.network_origin()?;
        if !sandbox_allows_network(&config.sandbox, &origin)? {
            return Err(RuntimeError::Config(format!(
                "search profile {name} origin {origin} is absent from sandbox.networkDestinations"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn provider_profile(
    name: &str,
    config: &ProviderProfileConfig,
) -> Result<ProviderProfile, RuntimeError> {
    provider_profile_with_authority(name, config, ResourceAuthority::Declared)
}

pub(super) fn provider_profile_with_authority(
    name: &str,
    config: &ProviderProfileConfig,
    resource_authority: ResourceAuthority,
) -> Result<ProviderProfile, RuntimeError> {
    let profile = ProviderProfile::new_with_resource_authority(
        name,
        config.kind,
        config.base_url.clone(),
        config.credential_reference.clone(),
        config.effective_timeout_ms(),
        resource_authority,
    )
    .map_err(RuntimeError::from)?
    .with_generation_timeout_ms(config.effective_generation_timeout_ms())
    .map_err(RuntimeError::from)?;
    match config.chat_completions_output_token_parameter {
        Some(parameter) => profile
            .with_chat_completions_output_token_parameter(parameter)
            .map_err(Into::into),
        None => Ok(profile),
    }
}

pub(super) fn provider_registry(
    providers_config: &ProvidersConfig,
    models_config: &ModelsConfig,
    credentials: Arc<dyn CredentialResolver>,
    codex_auth: Option<CodexAuthStore>,
    tls_roots: &AdditionalRootCertificates,
    resource_authority: ResourceAuthority,
    media: Option<Arc<dyn RunInputMediaResolver>>,
) -> Result<ProviderRegistry, RuntimeError> {
    let profiles = providers_config
        .profiles
        .iter()
        .map(|(name, profile)| {
            provider_profile_with_authority(name, profile, resource_authority).map(|profile| {
                let executor =
                    ProviderExecutor::with_credentials(profile, Arc::clone(&credentials))
                        .with_tls_roots(tls_roots.clone());
                let executor = match &media {
                    Some(media) => executor.with_run_input_media(Arc::clone(media)),
                    None => executor,
                };
                match &codex_auth {
                    Some(store) => executor.with_codex_auth_store(store.clone()),
                    None => executor,
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let models = models_config
        .profiles
        .iter()
        .map(|(name, model)| {
            ModelProfile::new(
                name,
                model.provider_profile.clone(),
                model.model.clone(),
                model.context_window_tokens,
                model.max_output_tokens,
                model.capabilities.capabilities(),
                model.reasoning_effort,
            )
            .map(|profile| profile.with_feature_settings(model.capabilities))
        })
        .collect::<Result<Vec<_>, _>>()?;
    ProviderRegistry::new(profiles, models, models_config.roles.clone()).map_err(Into::into)
}

pub(super) fn compose_memory_indexes(
    config: &RuntimeConfig,
) -> Result<Vec<MemoryIndexRegistration>, RuntimeError> {
    if !config.memory.index_enabled {
        let index: Arc<dyn MemoryIndex> = Arc::new(UnavailableMemoryIndex::new(
            "memory index disabled by configuration",
        ));
        return Ok(vec![MemoryIndexRegistration::new(
            "memory.disabled-v1",
            index,
        )?]);
    }
    let lexical: Arc<dyn MemoryIndex> = if config.storage.adapter == StorageAdapter::Ephemeral
        && config.memory.index_path.is_none()
    {
        Arc::new(TantivyMemoryIndex::in_memory()?)
    } else {
        let path = config
            .memory
            .index_path
            .clone()
            .unwrap_or_else(|| config.storage.path.with_extension("memory-index"));
        config.revalidate_resolved_home_directory(&path)?;
        Arc::new(LazyTantivyMemoryIndex::new(path))
    };
    Ok(vec![MemoryIndexRegistration::new(
        "memory.tantivy-v1",
        lexical,
    )?])
}

pub(super) fn validate_provider_config(config: &RuntimeConfig) -> Result<(), RuntimeError> {
    const ROLES: [&str; 7] = [
        "primary",
        "risk_evaluator",
        "context_summarizer",
        "subagent_default",
        "research_planner",
        "research_worker",
        "research_synthesizer",
    ];
    if config
        .models
        .roles
        .keys()
        .any(|role| !ROLES.contains(&role.as_str()))
    {
        return Err(RuntimeError::Config(
            "model roles contain an unknown role name".into(),
        ));
    }
    let _ = provider_registry(
        &config.providers,
        &config.models,
        Arc::new(EnvironmentCredentialResolver),
        None,
        &AdditionalRootCertificates::default(),
        configured_resource_authority(&config.sandbox),
        None,
    )?;
    for (name, profile) in &config.providers.profiles {
        let profile = provider_profile_with_authority(
            name,
            profile,
            configured_resource_authority(&config.sandbox),
        )?;
        if let Some(origin) = profile.network_origin()?
            && !sandbox_allows_network(&config.sandbox, &origin)?
        {
            return Err(RuntimeError::Config(format!(
                "provider profile {name} origin {origin} is absent from sandbox.networkDestinations"
            )));
        }
        for origin in profile.authentication_origins() {
            if !sandbox_allows_network(&config.sandbox, origin)? {
                return Err(RuntimeError::Config(format!(
                    "provider profile {name} authentication origin {origin} is absent from sandbox.networkDestinations"
                )));
            }
        }
    }
    Ok(())
}
