//! Search definitions remain beneath the deployment ceiling and purpose-bound custody.
use crate::config::ChatProjectConfig;
use colossus_cloud::{CloudError, CloudResult};
use colossus_cloud_chat::ChatAuthentication;
use colossus_runtime::{SearchConfig, SearchProfileConfig};

const MANAGED_REFERENCE: &str = "host:managed";

pub(crate) fn profile_authentication(
    profile: &SearchProfileConfig,
    configured: Option<ChatAuthentication>,
) -> ChatAuthentication {
    configured.unwrap_or_else(|| {
        if reference(profile).is_none() {
            ChatAuthentication::None
        } else {
            ChatAuthentication::Personal
        }
    })
}

pub(crate) fn authentication(config: &ChatProjectConfig, id: &str) -> ChatAuthentication {
    config
        .runtime
        .search
        .profiles
        .get(id)
        .map_or_else(ChatAuthentication::default, |profile| {
            profile_authentication(profile, config.search_authentication.get(id).copied())
        })
}

pub(crate) fn endpoint(profile: &SearchProfileConfig) -> &str {
    match profile {
        SearchProfileConfig::Searxng { endpoint, .. }
        | SearchProfileConfig::SerpApi { endpoint, .. } => endpoint,
    }
}

pub(crate) fn reference(profile: &SearchProfileConfig) -> Option<&str> {
    match profile {
        SearchProfileConfig::Searxng {
            credential_reference,
            ..
        } => credential_reference.as_deref(),
        SearchProfileConfig::SerpApi {
            credential_reference,
            ..
        } => Some(credential_reference),
    }
}

pub(crate) fn set_reference(
    profile: &mut SearchProfileConfig,
    value: Option<String>,
) -> CloudResult<()> {
    match profile {
        SearchProfileConfig::Searxng {
            credential_reference,
            ..
        } => *credential_reference = value,
        SearchProfileConfig::SerpApi {
            credential_reference,
            ..
        } => {
            *credential_reference = value.ok_or(CloudError::InvalidArgument)?;
        }
    }
    Ok(())
}

/// Browser reads never disclose environment variable names or internal vault handles.
pub(crate) fn public_search(search: &SearchConfig) -> SearchConfig {
    let mut output = search.clone();
    for profile in output.profiles.values_mut() {
        if reference(profile).is_some() {
            // The SerpAPI variant always contains a reference, so this cannot fail.
            let _ = set_reference(profile, Some(MANAGED_REFERENCE.into()));
        }
    }
    output
}

/// Review one effective project update before its canonical configuration commits.
pub(crate) fn review_search(
    current: &ChatProjectConfig,
    updated: &mut ChatProjectConfig,
    ceiling: &ChatProjectConfig,
) -> CloudResult<()> {
    let search = &mut updated.runtime.search;
    if search.profiles.len() > 64
        || search.roles.len() > 2
        || search.roles.iter().any(|(role, profile)| {
            !matches!(role.as_str(), "agent" | "research")
                || (!search.profiles.contains_key(profile)
                    && current.runtime.search.roles.get(role) != Some(profile))
        })
        || updated
            .search_authentication
            .keys()
            .any(|id| !search.profiles.contains_key(id))
    {
        return Err(CloudError::InvalidArgument);
    }
    for (id, profile) in &mut search.profiles {
        colossus_cloud::validate_identifier(id)?;
        let authentication =
            profile_authentication(profile, updated.search_authentication.get(id).copied());
        let prior = current.runtime.search.profiles.get(id);
        let same_destination = prior.is_some_and(|old| {
            std::mem::discriminant(old) == std::mem::discriminant(profile)
                && endpoint(old) == endpoint(profile)
                && authentication
                    == profile_authentication(old, current.search_authentication.get(id).copied())
                && match (old, &*profile) {
                    (
                        SearchProfileConfig::Searxng {
                            auth_header: old, ..
                        },
                        SearchProfileConfig::Searxng {
                            auth_header: new, ..
                        },
                    ) => old == new,
                    _ => true,
                }
        });
        let supplied = reference(profile);
        if supplied.is_some_and(|value| {
            value != MANAGED_REFERENCE
                && !(same_destination && prior.and_then(reference) == Some(value))
        }) {
            return Err(CloudError::InvalidArgument);
        }
        if authentication == ChatAuthentication::None {
            if supplied.is_some() {
                return Err(CloudError::InvalidArgument);
            }
        } else {
            let retained = if same_destination {
                prior.and_then(reference).map(str::to_owned)
            } else {
                None
            };
            set_reference(
                profile,
                Some(retained.unwrap_or_else(|| MANAGED_REFERENCE.into())),
            )?;
        }
        let url = endpoint(profile);
        let existing_seed = prior.is_some_and(|old| endpoint(old) == url)
            && ceiling
                .runtime
                .search
                .profiles
                .get(id)
                .is_some_and(|seed| endpoint(seed) == url);
        crate::chat_settings::allowed_endpoint(url, ceiling, existing_seed)?;
        let timeout = match profile {
            SearchProfileConfig::Searxng { timeout_ms, .. }
            | SearchProfileConfig::SerpApi { timeout_ms, .. } => *timeout_ms,
        };
        if !(1..=300_000).contains(&timeout) {
            return Err(CloudError::InvalidArgument);
        }
        if !same_destination {
            let generation =
                current.generation(colossus_cloud_chat::ChatConnectionKind::Search, id);
            updated.search_generations.insert(
                id.clone(),
                generation
                    .checked_add(1)
                    .ok_or(CloudError::ResourceExhausted)?,
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn configuration() -> ChatProjectConfig {
        let mut runtime = colossus_runtime::RuntimeConfig::controlled_chat_template("unused.redb");
        runtime.sandbox.network_destinations = vec!["https://search.example".into()];
        serde_json::from_value(json!({"runtime":runtime,"revision":1})).unwrap()
    }

    fn search(endpoint: &str, reference: Option<&str>) -> SearchConfig {
        serde_json::from_value(json!({
            "profiles":{"web":{"kind":"searxng","endpoint":endpoint,"credentialReference":reference,"authHeader":"X-Search-Key","userAgent":"colossus","timeoutMs":30000}},
            "roles":{"agent":"web","research":"web"}
        })).unwrap()
    }

    #[test]
    fn search_updates_admit_new_origins_but_keep_routes_and_credentials_purpose_bound() {
        let current = configuration();
        let mut next = current.clone();
        next.runtime.search = search("https://search.example/search", None);
        next.search_authentication
            .insert("web".into(), ChatAuthentication::None);
        review_search(&current, &mut next, &current).unwrap();
        assert_eq!(reference(&next.runtime.search.profiles["web"]), None);
        assert_eq!(next.search_generations["web"], 2);
        next.runtime.search = search("https://foreign.example/search", None);
        review_search(&current, &mut next, &current).unwrap();
        next.runtime.search = search("https://search.example/search", Some("env:UNREVIEWED"));
        next.search_authentication
            .insert("web".into(), ChatAuthentication::Shared);
        assert_eq!(
            review_search(&current, &mut next, &current),
            Err(CloudError::InvalidArgument)
        );
        next.runtime.search = search("https://search.example/search", None);
        next.search_authentication
            .insert("web".into(), ChatAuthentication::Personal);
        review_search(&current, &mut next, &current).unwrap();
        assert_eq!(
            reference(&next.runtime.search.profiles["web"]),
            Some(MANAGED_REFERENCE)
        );
        next.runtime
            .search
            .roles
            .insert("unreviewed".into(), "web".into());
        assert_eq!(
            review_search(&current, &mut next, &current),
            Err(CloudError::InvalidArgument)
        );
    }

    #[test]
    fn unchanged_seed_credentials_stay_private_and_repointing_retires_them() {
        let mut current = configuration();
        current.runtime.search = search("https://search.example/search", Some("env:SEED_SEARCH"));
        current
            .search_authentication
            .insert("web".into(), ChatAuthentication::Shared);
        current.search_generations.insert("web".into(), 8);
        let mut next = current.clone();
        next.runtime.search = public_search(&current.runtime.search);
        review_search(&current, &mut next, &current).unwrap();
        assert_eq!(
            reference(&next.runtime.search.profiles["web"]),
            Some("env:SEED_SEARCH")
        );
        assert_eq!(next.search_generations["web"], 8);
        next.runtime.search = search("https://search.example/v2/search", Some(MANAGED_REFERENCE));
        review_search(&current, &mut next, &current).unwrap();
        assert_eq!(
            reference(&next.runtime.search.profiles["web"]),
            Some(MANAGED_REFERENCE)
        );
        assert_eq!(next.search_generations["web"], 9);
    }
}
