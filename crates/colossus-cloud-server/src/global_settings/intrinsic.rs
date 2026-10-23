//! Reuse strict hosted definition validation even before a deployment enables projects.
use super::types::HostedCatalog;
use crate::config::ChatProjectConfig;
use colossus_cloud::{CloudError, CloudResult};
use colossus_runtime::{KeyConfig, RuntimeConfig};

pub(super) fn review(catalog: &HostedCatalog) -> CloudResult<()> {
    // This configuration is only parsed/reviewed. It is never opened, persisted,
    // granted to a project, or used as a replacement for an operator ceiling.
    let mut runtime = RuntimeConfig::controlled_chat_template("/catalog-review/state.redb");
    runtime.storage.keys = KeyConfig::Environment {
        journal_variable: "CATALOG_REVIEW_JOURNAL".into(),
        journal_key_id: "catalog-review".into(),
        signing_variable: "CATALOG_REVIEW_SIGNING".into(),
        anchor_path: "/catalog-review/state.anchor".into(),
    };
    if !catalog.providers.profiles.is_empty() {
        runtime.providers = catalog.providers.clone();
    }
    if catalog.models.profiles.is_empty() {
        let provider = runtime
            .providers
            .profiles
            .keys()
            .next()
            .ok_or(CloudError::InvalidArgument)?;
        for model in runtime.models.profiles.values_mut() {
            model.provider_profile = provider.clone();
        }
    } else {
        runtime.models.profiles = catalog.models.profiles.clone();
        let model = catalog
            .models
            .roles
            .get("primary")
            .or_else(|| catalog.models.profiles.keys().next())
            .ok_or(CloudError::InvalidArgument)?;
        runtime.models.roles = [("primary".into(), model.clone())].into_iter().collect();
    }
    for role in catalog.models.roles.keys() {
        colossus_cloud::validate_identifier(role)?;
    }
    runtime.mcp = catalog.mcp.clone();
    runtime.search = catalog.search.clone();
    runtime.observability = catalog.observability.clone();
    let endpoints = catalog
        .providers
        .profiles
        .values()
        .filter_map(|profile| profile.base_url.as_deref())
        .chain(
            catalog
                .mcp
                .servers
                .values()
                .filter_map(|server| server.url.as_deref()),
        )
        .chain(
            catalog
                .search
                .profiles
                .values()
                .map(super::search::endpoint),
        );
    for endpoint in endpoints {
        let url = url::Url::parse(endpoint).map_err(|_| CloudError::InvalidArgument)?;
        let origin = url.origin().ascii_serialization();
        if !runtime.sandbox.network_destinations.contains(&origin) {
            runtime.sandbox.network_destinations.push(origin);
        }
    }
    let mut configuration = ChatProjectConfig::new(runtime);
    configuration.labels = catalog.labels.clone();
    configuration.provider_authentication = catalog.provider_authentication.clone();
    configuration.mcp_authentication = catalog.mcp_authentication.clone();
    configuration.search_authentication = catalog.search_authentication.clone();
    crate::chat_settings::review_effective(&configuration, &configuration)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_and_partial_libraries_are_reviewed_without_project_authority() {
        let mut catalog = HostedCatalog::default();
        assert!(review(&catalog).is_ok());
        catalog.providers = serde_json::from_value(json!({"profiles":{"main":{"kind":"open_ai_compatible","baseUrl":"https://models.example/v1"}}})).unwrap();
        assert!(review(&catalog).is_ok());
        catalog
            .providers
            .profiles
            .get_mut("main")
            .unwrap()
            .timeout_ms = Some(0);
        assert!(matches!(review(&catalog), Err(CloudError::InvalidArgument)));
        catalog
            .providers
            .profiles
            .get_mut("main")
            .unwrap()
            .timeout_ms = None;
        catalog
            .labels
            .providers
            .insert("main".into(), "bad\nlabel".into());
        assert!(matches!(review(&catalog), Err(CloudError::InvalidArgument)));
        catalog.labels.providers.clear();
        catalog.providers.profiles.get_mut("main").unwrap().base_url =
            Some("https://user:secret@models.example/v1".into());
        assert!(matches!(review(&catalog), Err(CloudError::InvalidArgument)));
    }
}
