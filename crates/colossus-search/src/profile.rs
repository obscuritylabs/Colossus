use super::*;

/// Supported first-party search adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    /// SearXNG JSON search endpoint.
    Searxng,
    /// SerpAPI Google organic-results endpoint.
    SerpApi,
}

impl SearchKind {
    /// Stable adapter label used in route and audit metadata.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Searxng => "searxng",
            Self::SerpApi => "serp_api",
        }
    }
}

/// Strict normalized search profile composed by the runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchProfile {
    pub(super) name: String,
    pub(super) kind: SearchKind,
    pub(super) endpoint: String,
    pub(super) credential_reference: Option<String>,
    pub(super) auth_header: Option<String>,
    pub(super) user_agent: String,
    pub(super) timeout_ms: u64,
}

impl SearchProfile {
    /// Validate one profile without resolving credentials.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: impl Into<String>,
        kind: SearchKind,
        endpoint: impl Into<String>,
        credential_reference: Option<String>,
        auth_header: Option<String>,
        user_agent: impl Into<String>,
        timeout_ms: u64,
    ) -> Result<Self, SearchAdapterError> {
        Self::new_with_resource_authority(
            name,
            kind,
            endpoint,
            credential_reference,
            auth_header,
            user_agent,
            timeout_ms,
            ResourceAuthority::Declared,
        )
    }

    /// Validate one profile under an explicit runtime resource authority.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_resource_authority(
        name: impl Into<String>,
        kind: SearchKind,
        endpoint: impl Into<String>,
        credential_reference: Option<String>,
        auth_header: Option<String>,
        user_agent: impl Into<String>,
        timeout_ms: u64,
        resource_authority: ResourceAuthority,
    ) -> Result<Self, SearchAdapterError> {
        let name = name.into();
        let endpoint = normalize_endpoint(kind, &endpoint.into(), resource_authority)?;
        let user_agent = user_agent.into();
        if !valid_name(&name) || timeout_ms == 0 {
            return Err(SearchAdapterError::Configuration(
                "profile name and timeout must be valid and nonzero".into(),
            ));
        }
        if user_agent.trim().is_empty() || user_agent.len() > 256 {
            return Err(SearchAdapterError::Configuration(
                "search user agent must contain 1 through 256 bytes".into(),
            ));
        }
        if let Some(reference) = credential_reference.as_deref()
            && !valid_credential_reference(reference)
        {
            return Err(SearchAdapterError::Configuration(
                "search credentials must use an env:VARIABLE reference".into(),
            ));
        }
        match kind {
            SearchKind::Searxng => {
                if credential_reference.is_some()
                    && auth_header.as_deref().is_none_or(str::is_empty)
                {
                    return Err(SearchAdapterError::Configuration(
                        "credentialed SearXNG profiles require authHeader".into(),
                    ));
                }
                if auth_header
                    .as_deref()
                    .is_some_and(|header| !valid_header_name(header))
                {
                    return Err(SearchAdapterError::Configuration(
                        "SearXNG authHeader is invalid".into(),
                    ));
                }
            }
            SearchKind::SerpApi => {
                if credential_reference.is_none() || auth_header.is_some() {
                    return Err(SearchAdapterError::Configuration(
                        "SerpAPI requires a credential reference and does not accept authHeader"
                            .into(),
                    ));
                }
            }
        }
        Ok(Self {
            name,
            kind,
            endpoint,
            credential_reference,
            auth_header,
            user_agent,
            timeout_ms,
        })
    }

    /// Stable profile name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Adapter kind.
    pub fn kind(&self) -> SearchKind {
        self.kind
    }

    /// Exact credential-free endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Canonical endpoint origin for policy validation.
    pub fn network_origin(&self) -> Result<String, SearchAdapterError> {
        Url::parse(&self.endpoint)
            .map(|url| url.origin().ascii_serialization())
            .map_err(SearchAdapterError::from)
    }

    /// Credential reference suitable for policy input.
    pub fn credential_reference(&self) -> Option<CredentialReference> {
        self.credential_reference
            .as_ref()
            .map(|reference| CredentialReference {
                reference: reference.clone(),
                value_hash: None,
            })
    }

    /// Safe profile metadata for diagnostics.
    pub fn summary(&self) -> SearchProfileSummary {
        SearchProfileSummary {
            profile: self.name.clone(),
            provider: self.kind.as_str().into(),
            endpoint: self.endpoint.clone(),
            credential_reference: self.credential_reference.clone(),
            timeout_ms: self.timeout_ms,
        }
    }
}
