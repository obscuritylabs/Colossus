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

pub(super) fn validate_memory_config(
    memory: &MemoryConfig,
    sandbox: &SandboxConfig,
) -> Result<(), RuntimeError> {
    let SemanticMemoryConfig::Chroma {
        base_url,
        tenant,
        database,
        collection,
        credential_reference,
        timeout_ms,
        position_path: _,
        embedding,
    } = &memory.semantic
    else {
        return Ok(());
    };
    if !memory.index_enabled {
        return Err(RuntimeError::Config(
            "memory semantic Chroma requires indexEnabled: true".into(),
        ));
    }
    let resource_authority = configured_resource_authority(sandbox);
    let chroma = ChromaProfile::new_with_resource_authority(
        base_url,
        tenant,
        database,
        collection,
        credential_reference.clone(),
        *timeout_ms,
        resource_authority,
    )?;
    let chroma_origin = chroma.network_origin()?;
    if !sandbox_allows_network(sandbox, &chroma_origin)? {
        return Err(RuntimeError::Config(format!(
            "Chroma origin {chroma_origin} is absent from sandbox.networkDestinations"
        )));
    }
    match embedding.as_ref() {
        MemoryEmbeddingConfig::Local { dimensions } => {
            let _ = LocalHashEmbeddingProvider::new(*dimensions)?;
        }
        MemoryEmbeddingConfig::OpenAiCompatible {
            profile,
            model,
            base_url,
            credential_reference,
            timeout_ms,
            dimensions,
        } => {
            let profile = OpenAiEmbeddingProfile::new_with_resource_authority(
                profile,
                model,
                base_url,
                credential_reference.clone(),
                *timeout_ms,
                *dimensions,
                resource_authority,
            )?;
            let embedding_origin = profile.network_origin()?;
            if !sandbox_allows_network(sandbox, &embedding_origin)? {
                return Err(RuntimeError::Config(format!(
                    "embedding origin {embedding_origin} is absent from sandbox.networkDestinations"
                )));
            }
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
                model.capabilities,
                model.reasoning_effort,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    ProviderRegistry::new(profiles, models, models_config.roles.clone()).map_err(Into::into)
}

pub(super) fn compose_memory_indexes(
    config: &RuntimeConfig,
    gateway: Arc<EffectGateway>,
    tls_roots: &AdditionalRootCertificates,
) -> Result<Vec<MemoryIndexRegistration>, RuntimeError> {
    let resource_authority = configured_resource_authority(&config.sandbox);
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
    let mut indexes = vec![MemoryIndexRegistration::new("memory.tantivy-v1", lexical)?];
    let SemanticMemoryConfig::Chroma {
        base_url,
        tenant,
        database,
        collection,
        credential_reference,
        timeout_ms,
        position_path,
        embedding,
    } = &config.memory.semantic
    else {
        return Ok(indexes);
    };
    let embedding: Arc<dyn EmbeddingProvider> = match embedding.as_ref() {
        MemoryEmbeddingConfig::Local { dimensions } => {
            Arc::new(LocalHashEmbeddingProvider::new(*dimensions)?)
        }
        MemoryEmbeddingConfig::OpenAiCompatible {
            profile,
            model,
            base_url,
            credential_reference,
            timeout_ms,
            dimensions,
        } => {
            let profile = OpenAiEmbeddingProfile::new_with_resource_authority(
                profile,
                model,
                base_url,
                credential_reference.clone(),
                *timeout_ms,
                *dimensions,
                resource_authority,
            )?;
            let executor = Arc::new(
                OpenAiEmbeddingExecutor::new(profile.clone()).with_tls_roots(tls_roots.clone()),
            );
            Arc::new(GatewayOpenAiEmbeddingProvider::new(
                Arc::clone(&gateway),
                executor,
                profile,
            ))
        }
    };
    let profile = ChromaProfile::new_with_resource_authority(
        base_url,
        tenant,
        database,
        collection,
        credential_reference.clone(),
        *timeout_ms,
        resource_authority,
    )?;
    let executor = Arc::new(ChromaExecutor::new(profile.clone()).with_tls_roots(tls_roots.clone()));
    let position_path = position_path
        .clone()
        .unwrap_or_else(|| config.storage.path.with_extension("chroma-position.json"));
    config.revalidate_resolved_home_file(&position_path)?;
    let semantic: Arc<dyn MemoryIndex> =
        match ChromaMemoryIndex::open(gateway, executor, embedding, profile, &position_path) {
            Ok(index) => Arc::new(index),
            Err(error) => Arc::new(UnavailableMemoryIndex::new(format!(
                "Chroma projection metadata {} could not open: {error}",
                position_path.display()
            ))),
        };
    indexes.push(MemoryIndexRegistration::new("memory.chroma-v1", semantic)?);
    Ok(indexes)
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
