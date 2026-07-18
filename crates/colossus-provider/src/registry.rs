use super::*;

/// Role-to-profile routing and permit-bound adapters.
pub struct ProviderRegistry {
    profiles: BTreeMap<String, Arc<ProviderExecutor>>,
    roles: BTreeMap<String, String>,
}

impl ProviderRegistry {
    /// Validate unique profiles and role targets.
    pub fn new(
        profiles: Vec<ProviderExecutor>,
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
        if indexed.is_empty() || !roles.contains_key("primary") {
            return Err(ProviderError::Configuration(
                "provider profiles and the primary role are required".into(),
            ));
        }
        for (role, profile) in &roles {
            if role.is_empty() || !indexed.contains_key(profile) {
                return Err(ProviderError::Configuration(format!(
                    "provider role {role} references unknown profile {profile}"
                )));
            }
        }
        Ok(Self {
            profiles: indexed,
            roles,
        })
    }

    /// Resolve a role, falling back to `primary` for an unconfigured specialized role.
    pub fn resolve(&self, role: &str) -> Result<Arc<ProviderExecutor>, ProviderError> {
        let profile = self
            .roles
            .get(role)
            .or_else(|| self.roles.get("primary"))
            .ok_or_else(|| ProviderError::Configuration("primary role is absent".into()))?;
        self.profiles.get(profile).cloned().ok_or_else(|| {
            ProviderError::Configuration(format!("provider profile {profile} is absent"))
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

    /// Sorted profile readiness without making network calls.
    pub fn profiles(&self) -> Vec<ProviderReadiness> {
        self.profiles
            .values()
            .map(|provider| provider.static_readiness())
            .collect()
    }
}
