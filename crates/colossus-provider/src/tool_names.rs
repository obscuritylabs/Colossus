use super::*;

const MAX_PROVIDER_TOOL_NAME_BYTES: usize = 64;

/// Per-request aliases between canonical Colossus tool identities and the
/// portable function names accepted by OpenAI-compatible provider APIs.
#[derive(Clone, Debug, Default)]
pub(super) struct ProviderToolNames {
    canonical_to_provider: BTreeMap<String, String>,
    provider_to_canonical: BTreeMap<String, String>,
}

impl ProviderToolNames {
    pub(super) fn from_request(request: &ModelRequest) -> Result<Self, ProviderError> {
        let canonical_names = request
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .chain(
                request
                    .messages
                    .iter()
                    .flat_map(|message| message.tool_calls.iter())
                    .map(|call| call.name.as_str()),
            )
            .collect::<BTreeSet<_>>();
        let mut names = Self::default();
        for canonical in canonical_names {
            let provider = portable_provider_name(canonical)?;
            if let Some(existing) = names
                .provider_to_canonical
                .insert(provider.clone(), canonical.to_owned())
                && existing != canonical
            {
                return Err(ProviderError::Configuration(
                    "provider tool names collide after portable aliasing".into(),
                ));
            }
            names
                .canonical_to_provider
                .insert(canonical.to_owned(), provider);
        }
        Ok(names)
    }

    pub(super) fn provider_name<'a>(&'a self, canonical: &str) -> Result<&'a str, ProviderError> {
        self.canonical_to_provider
            .get(canonical)
            .map(String::as_str)
            .ok_or_else(|| {
                ProviderError::Configuration(
                    "provider tool name is absent from the request alias map".into(),
                )
            })
    }

    pub(super) fn canonical_name<'a>(&'a self, provider: &'a str) -> &'a str {
        self.provider_to_canonical
            .get(provider)
            .map_or(provider, String::as_str)
    }
}

fn portable_provider_name(canonical: &str) -> Result<String, ProviderError> {
    let provider = canonical.replace('.', "_");
    if provider.is_empty()
        || provider.len() > MAX_PROVIDER_TOOL_NAME_BYTES
        || !provider
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(ProviderError::Configuration(
            "provider tool name cannot be represented by the portable function-name contract"
                .into(),
        ));
    }
    Ok(provider)
}
