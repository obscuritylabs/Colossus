use super::*;

type FeatureRoute = (String, Option<String>, String, String);
type FeatureObservations =
    BTreeMap<(FeatureRoute, crate::ProviderFeature), (std::time::Instant, &'static str)>;

/// Role-to-model routing layered over permit-bound provider connections.
pub struct ProviderRegistry {
    profiles: BTreeMap<String, Arc<ProviderExecutor>>,
    models: BTreeMap<String, ModelProfile>,
    roles: BTreeMap<String, String>,
    observations: std::sync::Mutex<FeatureObservations>,
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
            observations: Default::default(),
        })
    }

    /// Bind private Responses state before publishing the registry.
    pub fn with_continuations(
        mut self,
        repository: Arc<dyn colossus_ports::ProviderContinuationRepository>,
    ) -> Result<Self, ProviderError> {
        for provider in self.profiles.values_mut() {
            Arc::get_mut(provider)
                .ok_or_else(|| {
                    ProviderError::Configuration("provider registry already shared".into())
                })?
                .continuations = Some(Arc::clone(&repository));
        }
        Ok(self)
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
        let mut model = self.models.get(model_name).cloned().ok_or_else(|| {
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
        let settings = model.feature_settings;
        model.capabilities = ModelCapabilities {
            tool_calls: settings.tool_calls.eligible(
                settings.declared.tool_calls,
                self.unsupported(&model, &provider, crate::ProviderFeature::Tools),
            ),
            streaming: settings.streaming.eligible(
                settings.declared.streaming,
                self.unsupported(&model, &provider, crate::ProviderFeature::Streaming),
            ),
            image_inputs: settings.image_inputs.eligible(
                settings.declared.image_inputs,
                self.unsupported(&model, &provider, crate::ProviderFeature::Images),
            ),
        };
        Ok(ResolvedModel {
            role: role.into(),
            model,
            provider,
        })
    }

    fn observation_key(model: &ModelProfile, provider: &ProviderExecutor) -> FeatureRoute {
        (
            provider.profile.kind.as_str().into(),
            provider.profile.base_url.clone(),
            provider.profile.name.clone(),
            model.model.clone(),
        )
    }

    fn unsupported(
        &self,
        model: &ModelProfile,
        provider: &ProviderExecutor,
        feature: crate::ProviderFeature,
    ) -> bool {
        self.observations
            .lock()
            .ok()
            .and_then(|entries| {
                entries
                    .get(&(Self::observation_key(model, provider), feature))
                    .copied()
            })
            .is_some_and(|(at, status)| {
                at.elapsed() < Duration::from_secs(15 * 60) && status == "unsupported"
            })
    }

    /// Whether the optional field is eligible for this exact Responses route.
    pub fn server_compaction_enabled(&self, resolved: &ResolvedModel) -> bool {
        let settings = resolved.model.feature_settings;
        resolved.provider.profile.kind == ProviderKind::OpenAiResponses
            && settings.server_compaction.eligible(
                settings.declared.server_compaction,
                self.unsupported(
                    &resolved.model,
                    &resolved.provider,
                    crate::ProviderFeature::ServerCompaction,
                ),
            )
    }

    /// Record categorical transport evidence for this registry/configuration generation.
    pub fn observe_feature(
        &self,
        resolved: &ResolvedModel,
        feature: crate::ProviderFeature,
        status: &'static str,
    ) {
        if let Ok(mut observations) = self.observations.lock() {
            observations.insert(
                (
                    Self::observation_key(&resolved.model, &resolved.provider),
                    feature,
                ),
                (std::time::Instant::now(), status),
            );
        }
    }

    /// Safe operator diagnostics for saved modes, declarations and observed evidence.
    pub fn feature_checks(&self, resolved: &ResolvedModel) -> Vec<ProviderReadinessCheck> {
        use crate::ProviderFeature;
        [
            ProviderFeature::Tools,
            ProviderFeature::Streaming,
            ProviderFeature::Images,
            ProviderFeature::ServerCompaction,
        ]
        .into_iter()
        .map(|feature| {
            let settings = resolved.model.feature_settings;
            let mode = feature.mode(settings);
            let declared = feature.declared(settings);
            let evidence = self
                .observations
                .lock()
                .ok()
                .and_then(|entries| {
                    entries
                        .get(&(Self::observation_key(&resolved.model, &resolved.provider), feature))
                        .copied()
                })
                .filter(|(at, _)| at.elapsed() < Duration::from_secs(15 * 60))
                .map_or("unknown", |(_, status)| status);
            let adapter_compatible = match feature {
                ProviderFeature::ServerCompaction => {
                    resolved.provider.profile.kind == ProviderKind::OpenAiResponses
                }
                ProviderFeature::Images => resolved.provider.profile.kind != ProviderKind::Echo,
                _ => true,
            };
            let (effective, reason) = if !adapter_compatible {
                ("disabled", "adapter does not support this feature")
            } else if mode == colossus_contracts::ModelFeatureMode::Off {
                ("disabled", "operator selected Off")
            } else if mode == colossus_contracts::ModelFeatureMode::On {
                ("eligible", "operator selected On")
            } else if evidence == "unsupported" {
                ("disabled", "provider explicitly rejected this feature")
            } else if declared == Some(false) {
                ("disabled", "model card reports unsupported")
            } else {
                ("eligible", "Auto permits supported or unknown features")
            };
            let declaration = declared.map_or("unknown", |value| if value { "true" } else { "false" });
            ProviderReadinessCheck {
                name: format!("feature_{feature:?}"),
                status: "not_checked".into(),
                detail: format!(
                    "mode={mode:?}; declared={declaration}; effective={effective} ({reason}); observed={evidence}; evidence expires after 15 minutes or configuration reload"
                ),
                provider_response: None,
            }
        })
        .collect()
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
