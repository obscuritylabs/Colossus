use super::*;

/// Role-to-model routing layered over permit-bound provider connections.
pub struct ProviderRegistry {
    profiles: BTreeMap<String, Arc<ProviderExecutor>>,
    models: BTreeMap<String, ModelProfile>,
    roles: BTreeMap<String, String>,
}

impl ProviderRegistry {
    /// Validate unique provider/model profiles and role targets.
    pub fn new(
        profiles: Vec<ProviderExecutor>,
        models: Vec<ModelProfile>,
        roles: BTreeMap<String, String>,
    ) -> Result<Self, ProviderError> {
        let mut indexed = BTreeMap::new();
        for provider in profiles {
            let name = provider.profile.name.clone();
            if indexed.insert(name.clone(), Arc::new(provider)).is_some() {
                return Err(ProviderError::Configuration(format!(
                    "duplicate provider profile {name}"
                )));
            }
        }
        let mut indexed_models = BTreeMap::new();
        for model in models {
            let name = model.name.clone();
            if !indexed.contains_key(&model.provider_profile) {
                return Err(ProviderError::Configuration(format!(
                    "model profile {name} references unknown provider profile {}",
                    model.provider_profile
                )));
            }
            if indexed_models.insert(name.clone(), model).is_some() {
                return Err(ProviderError::Configuration(format!(
                    "duplicate model profile {name}"
                )));
            }
        }
        if indexed.is_empty() || indexed_models.is_empty() || !roles.contains_key("primary") {
            return Err(ProviderError::Configuration(
                "provider profiles, model profiles, and the primary model role are required".into(),
            ));
        }
        for (role, model) in &roles {
            if role.is_empty() || !indexed_models.contains_key(model) {
                return Err(ProviderError::Configuration(format!(
                    "model role {role} references unknown model profile {model}"
                )));
            }
        }
        Ok(Self {
            profiles: indexed,
            models: indexed_models,
            roles,
        })
    }

    /// Resolve a role, falling back to `primary` for an unconfigured specialized role.
    pub fn resolve(&self, role: &str) -> Result<ResolvedModel, ProviderError> {
        let model_name = self
            .roles
            .get(role)
            .or_else(|| self.roles.get("primary"))
            .ok_or_else(|| ProviderError::Configuration("primary role is absent".into()))?;
        self.resolve_model(model_name, role)
    }

    /// Resolve one exact model profile without role fallback.
    pub fn model(&self, model_profile: &str) -> Result<ResolvedModel, ProviderError> {
        self.resolve_model(model_profile, "")
    }

    fn resolve_model(&self, model_name: &str, role: &str) -> Result<ResolvedModel, ProviderError> {
        let model = self.models.get(model_name).cloned().ok_or_else(|| {
            ProviderError::Configuration(format!("model profile {model_name} is absent"))
        })?;
        let provider = self
            .profiles
            .get(&model.provider_profile)
            .cloned()
            .ok_or_else(|| {
                ProviderError::Configuration(format!(
                    "provider profile {} is absent",
                    model.provider_profile
                ))
            })?;
        Ok(ResolvedModel {
            role: role.into(),
            model,
            provider,
        })
    }

    /// Resolve an exact profile without role fallback.
    pub fn profile(&self, profile: &str) -> Result<Arc<ProviderExecutor>, ProviderError> {
        self.profiles.get(profile).cloned().ok_or_else(|| {
            ProviderError::Configuration(format!("provider profile {profile} is absent"))
        })
    }

    /// Stable role mapping for diagnostics.
    pub fn routes(&self) -> &BTreeMap<String, String> {
        &self.roles
    }

    /// Sorted configured model routes without credentials.
    pub fn models(&self) -> Vec<ModelRoute> {
        self.models
            .values()
            .filter_map(|model| {
                let provider = self.profiles.get(&model.provider_profile)?;
                Some(model_route("", model, provider.profile()))
            })
            .collect()
    }

    /// Sorted configured model profiles using one provider connection.
    pub fn models_for_provider(&self, provider_profile: &str) -> Vec<ModelProfile> {
        self.models
            .values()
            .filter(|model| model.provider_profile == provider_profile)
            .cloned()
            .collect()
    }

    /// Sorted profile readiness without making network calls.
    pub fn profiles(&self) -> Vec<ProviderReadiness> {
        self.profiles
            .values()
            .map(|provider| provider.static_readiness())
            .collect()
    }
}

/// One fully resolved model and its permit-bound provider connection.
#[derive(Clone)]
pub struct ResolvedModel {
    role: String,
    model: ModelProfile,
    provider: Arc<ProviderExecutor>,
}

impl ResolvedModel {
    /// Safe route metadata.
    pub fn route(&self) -> ModelRoute {
        model_route(&self.role, &self.model, self.provider.profile())
    }

    /// Explicit model profile.
    pub fn model_profile(&self) -> &ModelProfile {
        &self.model
    }

    /// Permit-bound provider connection.
    pub fn provider(&self) -> &Arc<ProviderExecutor> {
        &self.provider
    }
}

fn model_route(role: &str, model: &ModelProfile, provider: &ProviderProfile) -> ModelRoute {
    ModelRoute {
        role: role.into(),
        profile: model.name.clone(),
        model_profile: model.name.clone(),
        provider_profile: provider.name.clone(),
        provider: provider.kind.as_str().into(),
        model: model.model.clone(),
        limits: model.limits,
        capabilities: model.capabilities,
        reasoning_effort: model.reasoning_effort,
    }
}
