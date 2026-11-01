//! Credential-free choices captured from the authenticated application's server info.

/// One configured model choice; it carries no endpoint, credential, or new authority.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serialization",
    derive(serde::Serialize, serde::Deserialize)
)]
#[cfg_attr(feature = "serialization", serde(deny_unknown_fields))]
pub struct ConfiguredModelProfile {
    /// Existing profile accepted by an explicitly supported run preference.
    pub profile: String,
    /// Provider model identifier, for display only.
    pub model: String,
    /// Provider adapter kind, without an endpoint or credentials.
    pub provider: String,
    /// Configured provider profile label.
    pub provider_profile: String,
    /// Configured reasoning default, when explicitly set.
    pub reasoning_effort: Option<String>,
    /// Only caller-authorized roles whose default route selects this profile.
    pub default_roles: Vec<String>,
    /// Explicitly enabled native document MIME types for this model/API route.
    #[cfg_attr(feature = "serialization", serde(default))]
    pub document_media_types: Vec<String>,
}

#[cfg(feature = "daemon")]
pub(crate) fn decode(
    profiles: Vec<colossus_api_proto::v1alpha1::ConfiguredModelProfile>,
    capabilities: &crate::ServerCapabilities,
) -> crate::SdkResult<Vec<ConfiguredModelProfile>> {
    if profiles.len() > 256 {
        return Err(crate::SdkError::IdentityMismatch);
    }
    let bounded = |value: &str, maximum| {
        !value.is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
    };
    let mut names = std::collections::BTreeSet::new();
    let mut default_roles = std::collections::BTreeSet::new();
    if profiles.iter().any(|profile| {
        !bounded(&profile.profile, 128)
            || !names.insert(profile.profile.as_str())
            || !bounded(&profile.model, 1024)
            || !bounded(&profile.provider, 512)
            || !bounded(&profile.provider_profile, 512)
            || profile.default_roles.len() > 64
            || !valid_document_media_types(&profile.document_media_types)
            || profile.default_roles.iter().any(|role| {
                !bounded(role, 128)
                    || !default_roles.insert(role.as_str())
                    || default_roles.len() > 64
            })
            || profile.reasoning_effort.as_deref().is_some_and(|effort| {
                !matches!(
                    effort,
                    "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
                )
            })
    }) {
        return Err(crate::SdkError::IdentityMismatch);
    }
    if !capabilities.contains("agent_runs.model_selection") {
        return Ok(Vec::new());
    }
    Ok(profiles
        .into_iter()
        .map(|profile| ConfiguredModelProfile {
            profile: profile.profile,
            model: profile.model,
            provider: profile.provider,
            provider_profile: profile.provider_profile,
            reasoning_effort: profile.reasoning_effort,
            default_roles: profile.default_roles,
            document_media_types: if capabilities.contains("attachments.document_input") {
                profile.document_media_types
            } else {
                Vec::new()
            },
        })
        .collect())
}

#[cfg(feature = "daemon")]
fn valid_document_media_types(media_types: &[String]) -> bool {
    media_types.len() <= 4
        && media_types.iter().all(|media| {
            matches!(
                media.as_str(),
                "application/pdf"
                    | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                    | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                    | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            )
        })
        && media_types
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == media_types.len()
}

#[cfg(all(test, feature = "daemon"))]
mod tests;
