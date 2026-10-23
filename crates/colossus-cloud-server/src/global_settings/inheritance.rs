//! Resolve explicit project overrides without turning definitions into execution authority.
use super::types::{HostedCatalog, Inheritance, Sections};
use crate::{config::ChatProjectConfig, hosted::HostedExecution};
use colossus_cloud::{CloudError, CloudResult};
use colossus_cloud_chat::ChatAuthentication;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

fn keys<T>(map: &BTreeMap<String, T>) -> BTreeSet<String> {
    map.keys().cloned().collect()
}
fn merge<T: Clone>(
    global: &BTreeMap<String, T>,
    local: &BTreeMap<String, T>,
    disabled: &BTreeSet<String>,
) -> BTreeMap<String, T> {
    global
        .iter()
        .filter(|(id, _)| !disabled.contains(*id))
        .chain(local.iter())
        .map(|(id, value)| (id.clone(), value.clone()))
        .collect()
}
fn sources<T>(
    global: &BTreeMap<String, T>,
    local: &BTreeMap<String, T>,
    disabled: &BTreeSet<String>,
) -> BTreeMap<String, &'static str> {
    global
        .keys()
        .filter(|id| !disabled.contains(*id))
        .map(|id| (id.clone(), "global"))
        .chain(local.keys().map(|id| (id.clone(), "project")))
        .collect()
}
fn authentication<T>(
    global: &BTreeMap<String, ChatAuthentication>,
    local: &BTreeMap<String, ChatAuthentication>,
    local_profiles: &BTreeMap<String, T>,
    profiles: &BTreeMap<String, T>,
) -> BTreeMap<String, ChatAuthentication> {
    profiles
        .keys()
        .map(|id| {
            (
                id.clone(),
                if local_profiles.contains_key(id) {
                    local.get(id)
                } else {
                    global.get(id)
                }
                .copied()
                .unwrap_or_default(),
            )
        })
        .collect()
}

pub(crate) fn resolve(
    raw: &ChatProjectConfig,
    global: &HostedCatalog,
    ceiling: &ChatProjectConfig,
) -> CloudResult<ChatProjectConfig> {
    let local = &raw.runtime;
    let disabled = &raw.catalog_disabled;
    let mut output = raw.clone();
    output.inheritance = Inheritance {
        global_revision: global.revision,
        overrides: Sections {
            providers: keys(&local.providers.profiles),
            models: keys(&local.models.profiles),
            mcp: keys(&local.mcp.servers),
            search: keys(&local.search.profiles),
            variables: keys(&raw.variables),
            roles: keys(&local.models.roles),
        },
        disabled: disabled.clone(),
        global_entries: Sections {
            providers: keys(&global.providers.profiles),
            models: keys(&global.models.profiles),
            mcp: keys(&global.mcp.servers),
            search: keys(&global.search.profiles),
            variables: keys(&global.variables),
            roles: keys(&global.models.roles),
        },
        provenance: Sections {
            providers: sources(
                &global.providers.profiles,
                &local.providers.profiles,
                &disabled.providers,
            ),
            models: sources(
                &global.models.profiles,
                &local.models.profiles,
                &disabled.models,
            ),
            mcp: sources(&global.mcp.servers, &local.mcp.servers, &disabled.mcp),
            search: sources(
                &global.search.profiles,
                &local.search.profiles,
                &disabled.search,
            ),
            variables: sources(&global.variables, &raw.variables, &disabled.variables),
            roles: sources(&global.models.roles, &local.models.roles, &disabled.roles),
        },
        global_generations: Sections {
            providers: global.provider_generations.clone(),
            mcp: global.mcp_generations.clone(),
            search: global.search_generations.clone(),
            ..Default::default()
        },
    };
    output.runtime.providers.profiles = merge(
        &global.providers.profiles,
        &local.providers.profiles,
        &disabled.providers,
    );
    output.runtime.models.profiles = merge(
        &global.models.profiles,
        &local.models.profiles,
        &disabled.models,
    );
    output.runtime.models.roles = merge(&global.models.roles, &local.models.roles, &disabled.roles);
    output.runtime.mcp.servers = merge(&global.mcp.servers, &local.mcp.servers, &disabled.mcp);
    output.runtime.search.profiles = merge(
        &global.search.profiles,
        &local.search.profiles,
        &disabled.search,
    );
    let disabled_search_roles = disabled
        .roles
        .iter()
        .filter_map(|id| id.strip_prefix("search:").map(str::to_owned))
        .collect();
    output.runtime.search.roles = merge(
        &global.search.roles,
        &local.search.roles,
        &disabled_search_roles,
    );
    output
        .inheritance
        .overrides
        .roles
        .extend(local.search.roles.keys().map(|id| format!("search:{id}")));
    output
        .inheritance
        .global_entries
        .roles
        .extend(global.search.roles.keys().map(|id| format!("search:{id}")));
    output.inheritance.provenance.roles.extend(
        sources(
            &global.search.roles,
            &local.search.roles,
            &disabled_search_roles,
        )
        .into_iter()
        .map(|(id, source)| (format!("search:{id}"), source)),
    );
    output.provider_authentication = authentication(
        &global.provider_authentication,
        &raw.provider_authentication,
        &local.providers.profiles,
        &output.runtime.providers.profiles,
    );
    output.mcp_authentication = authentication(
        &global.mcp_authentication,
        &raw.mcp_authentication,
        &local.mcp.servers,
        &output.runtime.mcp.servers,
    );
    output.search_authentication = output
        .runtime
        .search
        .profiles
        .keys()
        .filter_map(|id| {
            let selected = if local.search.profiles.contains_key(id) {
                raw.search_authentication.get(id)
            } else {
                global.search_authentication.get(id)
            };
            selected.map(|authentication| (id.clone(), *authentication))
        })
        .collect();
    output.labels.providers = merge(
        &global.labels.providers,
        &raw.labels.providers,
        &disabled.providers,
    );
    output.labels.models = merge(&global.labels.models, &raw.labels.models, &disabled.models);
    output.labels.search = merge(&global.labels.search, &raw.labels.search, &disabled.search);
    output.variables = merge(&global.variables, &raw.variables, &disabled.variables);
    output.runtime.observability = global.observability.clone();
    crate::chat_settings::review_effective(&output, ceiling)
}

/// Persist only local rows. Shared catalog updates remain visible without rewriting tenant custody.
pub(crate) fn local_configuration(config: &ChatProjectConfig) -> ChatProjectConfig {
    let mut local = config.clone();
    let keep = &config.inheritance.overrides;
    local
        .runtime
        .providers
        .profiles
        .retain(|id, _| keep.providers.contains(id));
    local
        .runtime
        .models
        .profiles
        .retain(|id, _| keep.models.contains(id));
    local
        .runtime
        .models
        .roles
        .retain(|id, _| keep.roles.contains(id));
    local
        .runtime
        .mcp
        .servers
        .retain(|id, _| keep.mcp.contains(id));
    local
        .runtime
        .search
        .profiles
        .retain(|id, _| keep.search.contains(id));
    local
        .runtime
        .search
        .roles
        .retain(|id, _| keep.roles.contains(&format!("search:{id}")));
    local
        .provider_authentication
        .retain(|id, _| keep.providers.contains(id));
    local
        .mcp_authentication
        .retain(|id, _| keep.mcp.contains(id));
    local
        .search_authentication
        .retain(|id, _| keep.search.contains(id));
    local
        .labels
        .providers
        .retain(|id, _| keep.providers.contains(id));
    local.labels.models.retain(|id, _| keep.models.contains(id));
    local.labels.search.retain(|id, _| keep.search.contains(id));
    local.variables.retain(|id, _| keep.variables.contains(id));
    local
}

pub(crate) fn equal<T: Serialize>(left: &T, right: &T) -> CloudResult<bool> {
    Ok(serde_json::to_value(left).map_err(|_| CloudError::Storage)?
        == serde_json::to_value(right).map_err(|_| CloudError::Storage)?)
}

pub(super) fn review_catalog(
    host: &HostedExecution,
    current: &HostedCatalog,
    next: &mut HostedCatalog,
) -> CloudResult<()> {
    if next.providers.profiles.len() > 64
        || next.models.profiles.len() > 256
        || next.models.roles.len() > 32
        || next.mcp.servers.len() > 64
        || next.search.profiles.len() > 64
    {
        return Err(CloudError::ResourceExhausted);
    }
    validate_variables(&next.variables)?;
    super::telemetry::validate(&next.observability)?;
    if next.models.profiles.values().any(|model| {
        !next
            .providers
            .profiles
            .contains_key(&model.provider_profile)
    }) || next
        .models
        .roles
        .values()
        .any(|id| !next.models.profiles.contains_key(id))
        || next
            .provider_authentication
            .keys()
            .any(|id| !next.providers.profiles.contains_key(id))
        || next
            .mcp_authentication
            .keys()
            .any(|id| !next.mcp.servers.contains_key(id))
        || next
            .search_authentication
            .keys()
            .any(|id| !next.search.profiles.contains_key(id))
        || next
            .search
            .roles
            .values()
            .any(|id| !next.search.profiles.contains_key(id))
        || next
            .labels
            .providers
            .keys()
            .any(|id| !next.providers.profiles.contains_key(id))
        || next
            .labels
            .models
            .keys()
            .any(|id| !next.models.profiles.contains_key(id))
        || next
            .labels
            .search
            .keys()
            .any(|id| !next.search.profiles.contains_key(id))
    {
        return Err(CloudError::InvalidArgument);
    }
    for profile in next.providers.profiles.values_mut() {
        if profile.kind.generation_action() == "provider.openai.codex" {
            return Err(CloudError::InvalidArgument);
        }
        if profile
            .credential_reference
            .as_deref()
            .is_some_and(|reference| reference != "host:managed")
        {
            return Err(CloudError::InvalidArgument);
        }
        profile.credential_reference = None;
    }
    if next.mcp.servers.values().any(|server| {
        server
            .credential_headers
            .values()
            .any(|header| header.reference != "host:managed")
            || server
                .oauth
                .as_ref()
                .is_some_and(|oauth| oauth.client_secret_reference.is_some())
    }) || next.search.profiles.values().any(|profile| {
        super::search::reference(profile).is_some_and(|reference| reference != "host:managed")
    }) {
        return Err(CloudError::InvalidArgument);
    }
    normalize_search(next)?;
    super::intrinsic::review(next)?;
    next.provider_generations = generations(
        &current.providers.profiles,
        &next.providers.profiles,
        &current.provider_authentication,
        &next.provider_authentication,
        &current.provider_generations,
        &["kind", "baseUrl"],
    )?;
    next.mcp_generations = generations(
        &current.mcp.servers,
        &next.mcp.servers,
        &current.mcp_authentication,
        &next.mcp_authentication,
        &current.mcp_generations,
        &["transport", "url", "oauth", "credentialHeaders"],
    )?;
    next.search_generations = generations(
        &current.search.profiles,
        &next.search.profiles,
        &search_authentication(current),
        &search_authentication(next),
        &current.search_generations,
        &["kind", "endpoint", "authHeader"],
    )?;
    // Every consuming project's immutable policy is checked; a definition cannot add origins.
    for (project, ceiling) in &host.deployment.projects {
        // Validate every catalog row even when a legacy local row currently shadows it.
        let mut definitions = ceiling.clone();
        definitions
            .runtime
            .providers
            .profiles
            .extend(next.providers.profiles.clone());
        definitions
            .runtime
            .models
            .profiles
            .extend(next.models.profiles.clone());
        definitions
            .runtime
            .models
            .roles
            .extend(next.models.roles.clone());
        definitions
            .runtime
            .mcp
            .servers
            .extend(next.mcp.servers.clone());
        definitions
            .runtime
            .search
            .profiles
            .extend(next.search.profiles.clone());
        definitions
            .runtime
            .search
            .roles
            .extend(next.search.roles.clone());
        definitions
            .provider_authentication
            .extend(next.provider_authentication.clone());
        definitions
            .mcp_authentication
            .extend(next.mcp_authentication.clone());
        definitions
            .search_authentication
            .extend(next.search_authentication.clone());
        definitions
            .labels
            .providers
            .extend(next.labels.providers.clone());
        definitions.labels.models.extend(next.labels.models.clone());
        definitions.labels.search.extend(next.labels.search.clone());
        crate::chat_settings::review_effective(&definitions, ceiling)?;
        let raw = crate::chat_settings::retained_configuration(host, project, ceiling)?;
        resolve(&raw, next, ceiling)?;
    }
    Ok(())
}
pub(super) fn normalize_search(catalog: &mut HostedCatalog) -> CloudResult<()> {
    for (id, profile) in &mut catalog.search.profiles {
        let authentication = super::search::profile_authentication(
            profile,
            catalog.search_authentication.get(id).copied(),
        );
        if authentication == ChatAuthentication::None {
            if super::search::reference(profile).is_some() {
                return Err(CloudError::InvalidArgument);
            }
        } else {
            super::search::set_reference(profile, Some("host:managed".into()))?;
        }
    }
    Ok(())
}
fn search_authentication(catalog: &HostedCatalog) -> BTreeMap<String, ChatAuthentication> {
    catalog
        .search
        .profiles
        .iter()
        .map(|(id, profile)| {
            (
                id.clone(),
                super::search::profile_authentication(
                    profile,
                    catalog.search_authentication.get(id).copied(),
                ),
            )
        })
        .collect()
}
fn generations<T: Serialize>(
    before: &BTreeMap<String, T>,
    after: &BTreeMap<String, T>,
    before_auth: &BTreeMap<String, ChatAuthentication>,
    after_auth: &BTreeMap<String, ChatAuthentication>,
    saved: &BTreeMap<String, u64>,
    identity_fields: &[&str],
) -> CloudResult<BTreeMap<String, u64>> {
    let mut generations = saved.clone();
    for (id, value) in after {
        let unchanged = before
            .get(id)
            .map(|previous| -> CloudResult<bool> {
                Ok(connection_identity(previous, identity_fields)?
                    == connection_identity(value, identity_fields)?)
            })
            .transpose()?
            .unwrap_or(false)
            && before_auth.get(id).copied().unwrap_or_default()
                == after_auth.get(id).copied().unwrap_or_default();
        if !unchanged {
            generations.insert(
                id.clone(),
                saved
                    .get(id)
                    .copied()
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or(CloudError::ResourceExhausted)?,
            );
        }
    }
    Ok(generations)
}

fn connection_identity<T: Serialize>(value: &T, fields: &[&str]) -> CloudResult<serde_json::Value> {
    let value = serde_json::to_value(value).map_err(|_| CloudError::Storage)?;
    Ok(serde_json::Value::Object(
        fields
            .iter()
            .map(|field| {
                (
                    (*field).into(),
                    value.get(*field).cloned().unwrap_or_default(),
                )
            })
            .collect(),
    ))
}
pub(crate) fn validate_variables(values: &BTreeMap<String, String>) -> CloudResult<()> {
    if values.len() > 256
        || values.iter().any(|(key, value)| {
            key.is_empty()
                || key.len() > 128
                || !key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                || value.len() > 4096
                || value.contains('\0')
        })
        || values.values().map(String::len).sum::<usize>() > 65_536
    {
        return Err(CloudError::InvalidArgument);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_cloud_chat::{ChatConnectionBinding, ChatConnectionKind};
    use serde_json::json;

    #[test]
    fn search_authentication_transitions_cannot_resurrect_an_earlier_binding() {
        let mut personal = HostedCatalog {
            search: serde_json::from_value(json!({"profiles":{"search":{"kind":"searxng","endpoint":"https://search.example/search","credentialReference":"host:managed"}}})).unwrap(),
            ..HostedCatalog::default()
        };
        personal
            .search_authentication
            .insert("search".into(), ChatAuthentication::Personal);
        let mut anonymous = personal.clone();
        anonymous.search_authentication.clear();
        super::super::search::set_reference(
            anonymous.search.profiles.get_mut("search").unwrap(),
            None,
        )
        .unwrap();
        let first = BTreeMap::from([("search".into(), 4)]);
        let anonymous_generations = generations(
            &personal.search.profiles,
            &anonymous.search.profiles,
            &search_authentication(&personal),
            &search_authentication(&anonymous),
            &first,
            &["kind", "endpoint", "authHeader"],
        )
        .unwrap();
        let restored = generations(
            &anonymous.search.profiles,
            &personal.search.profiles,
            &search_authentication(&anonymous),
            &search_authentication(&personal),
            &anonymous_generations,
            &["kind", "endpoint", "authHeader"],
        )
        .unwrap();
        assert_eq!(anonymous_generations["search"], 5);
        assert_eq!(restored["search"], 6);
        let binding = ChatConnectionBinding::new(
            "project-a",
            "alice",
            ChatConnectionKind::Search,
            "search",
            1,
            "https://search.example/search",
            ChatAuthentication::Personal,
        )
        .unwrap()
        .unwrap();
        assert_ne!(
            binding
                .clone()
                .with_catalog_generation(first["search"])
                .unwrap()
                .record_id(),
            binding
                .with_catalog_generation(restored["search"])
                .unwrap()
                .record_id()
        );
    }

    #[test]
    fn nonidentity_edits_preserve_credentials_but_endpoint_changes_rotate() {
        let before = BTreeMap::from([(
            "main".into(),
            json!({"kind":"open_ai_compatible","baseUrl":"https://models.example/v1","timeoutMs":30000}),
        )]);
        let mut after = before.clone();
        after.get_mut("main").unwrap()["timeoutMs"] = json!(60000);
        let saved = BTreeMap::from([("main".into(), 3)]);
        let auth = BTreeMap::new();
        assert_eq!(
            generations(&before, &after, &auth, &auth, &saved, &["kind", "baseUrl"]).unwrap()["main"],
            3
        );
        after.get_mut("main").unwrap()["baseUrl"] = json!("https://other.example/v1");
        assert_eq!(
            generations(&before, &after, &auth, &auth, &saved, &["kind", "baseUrl"]).unwrap()["main"],
            4
        );
    }

    #[test]
    fn authenticated_search_uses_the_same_canonical_profile_as_project_reads() {
        let mut catalog = HostedCatalog {
            search: serde_json::from_value(json!({"profiles":{"search":{"kind":"searxng","endpoint":"https://search.example/search"}}})).unwrap(),
            ..HostedCatalog::default()
        };
        catalog
            .search_authentication
            .insert("search".into(), ChatAuthentication::Personal);
        normalize_search(&mut catalog).unwrap();
        assert!(
            equal(
                &catalog.search,
                &super::super::search::public_search(&catalog.search)
            )
            .unwrap()
        );
        assert_eq!(
            super::super::search::reference(&catalog.search.profiles["search"]),
            Some("host:managed")
        );
        catalog
            .search_authentication
            .insert("search".into(), ChatAuthentication::None);
        assert_eq!(
            normalize_search(&mut catalog),
            Err(CloudError::InvalidArgument)
        );
    }
}
