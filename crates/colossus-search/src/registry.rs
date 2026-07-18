use super::*;

/// Role-to-profile routing over permit-bound search adapters.
pub struct SearchRegistry {
    profiles: BTreeMap<String, Arc<SearchExecutor>>,
    roles: BTreeMap<String, String>,
}

impl SearchRegistry {
    /// Validate unique profiles and every configured role target.
    pub fn new(
        profiles: Vec<SearchExecutor>,
        roles: BTreeMap<String, String>,
    ) -> Result<Self, SearchAdapterError> {
        let mut indexed = BTreeMap::new();
        for executor in profiles {
            let name = executor.profile.name.clone();
            if indexed.insert(name.clone(), Arc::new(executor)).is_some() {
                return Err(SearchAdapterError::Configuration(format!(
                    "duplicate search profile {name}"
                )));
            }
        }
        for (role, profile) in &roles {
            if role.is_empty() || !indexed.contains_key(profile) {
                return Err(SearchAdapterError::Configuration(format!(
                    "search role {role} references unknown profile {profile}"
                )));
            }
        }
        Ok(Self {
            profiles: indexed,
            roles,
        })
    }

    /// Resolve one exact configured role without fallback.
    pub fn resolve(&self, role: &str) -> Result<Arc<SearchExecutor>, SearchAdapterError> {
        let profile = self.roles.get(role).ok_or_else(|| {
            SearchAdapterError::Configuration(format!("search role {role} is not configured"))
        })?;
        self.profiles.get(profile).cloned().ok_or_else(|| {
            SearchAdapterError::Configuration(format!("search profile {profile} is absent"))
        })
    }

    /// Stable role mappings for diagnostics.
    pub fn routes(&self) -> &BTreeMap<String, String> {
        &self.roles
    }

    /// Sorted safe profile summaries.
    pub fn profiles(&self) -> Vec<SearchProfileSummary> {
        self.profiles
            .values()
            .map(|executor| executor.profile.summary())
            .collect()
    }
}
