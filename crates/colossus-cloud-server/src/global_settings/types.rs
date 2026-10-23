//! Typed instance catalogs and released project inheritance metadata.
use crate::config::ChatSettingsLabels;
use colossus_cloud_chat::ChatAuthentication;
use colossus_mcp::McpConfig;
use colossus_runtime::{ModelsConfig, ObservabilityConfig, ProvidersConfig, SearchConfig};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Reusable definitions, independent of a project's immutable execution ceiling.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostedCatalog {
    pub revision: u64,
    #[serde(default)]
    pub labels: ChatSettingsLabels,
    pub providers: ProvidersConfig,
    pub models: ModelsConfig,
    pub mcp: McpConfig,
    #[serde(default)]
    pub provider_authentication: BTreeMap<String, ChatAuthentication>,
    #[serde(default)]
    pub provider_generations: BTreeMap<String, u64>,
    #[serde(default)]
    pub mcp_authentication: BTreeMap<String, ChatAuthentication>,
    #[serde(default)]
    pub mcp_generations: BTreeMap<String, u64>,
    #[serde(default)]
    pub search: SearchConfig,
    #[serde(default)]
    pub search_authentication: BTreeMap<String, ChatAuthentication>,
    #[serde(default)]
    pub search_generations: BTreeMap<String, u64>,
    #[serde(default)]
    pub observability: ObservabilityConfig,
    /// Explicit nonsecret reusable values; never injected into a process environment.
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
}
impl Default for HostedCatalog {
    fn default() -> Self {
        Self {
            revision: 0,
            labels: ChatSettingsLabels::default(),
            providers: ProvidersConfig {
                profiles: BTreeMap::new(),
            },
            models: ModelsConfig {
                profiles: BTreeMap::new(),
                roles: BTreeMap::new(),
            },
            mcp: McpConfig::default(),
            provider_authentication: BTreeMap::new(),
            provider_generations: BTreeMap::new(),
            mcp_authentication: BTreeMap::new(),
            mcp_generations: BTreeMap::new(),
            search: SearchConfig::default(),
            search_authentication: BTreeMap::new(),
            search_generations: BTreeMap::new(),
            observability: ObservabilityConfig::default(),
            variables: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Sections<T> {
    pub providers: T,
    pub models: T,
    pub mcp: T,
    pub search: T,
    pub variables: T,
    pub roles: T,
}

#[derive(Clone, Default, Serialize)]
pub(crate) struct Inheritance {
    pub global_revision: u64,
    pub overrides: Sections<BTreeSet<String>>,
    pub disabled: Sections<BTreeSet<String>>,
    pub global_entries: Sections<BTreeSet<String>>,
    pub provenance: Sections<BTreeMap<String, &'static str>>,
    #[serde(skip)]
    pub global_generations: Sections<BTreeMap<String, u64>>,
}

#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InheritanceInput {
    pub global_revision: u64,
    #[serde(default)]
    pub reset: Sections<BTreeSet<String>>,
    // Accept known GET metadata on round trips; only the revision and explicit
    // reset command above influence a write. Provenance is always recomputed.
    #[serde(default, rename = "overrides")]
    _overrides: Sections<BTreeSet<String>>,
    #[serde(default, rename = "disabled")]
    _disabled: Sections<BTreeSet<String>>,
    #[serde(default, rename = "global_entries")]
    _global_entries: Sections<BTreeSet<String>>,
    #[serde(default, rename = "provenance")]
    _provenance: Sections<BTreeMap<String, String>>,
}
