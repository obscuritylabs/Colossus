use super::*;

/// Strict OPA REST/mTLS configuration.
pub struct OpaConfig {
    /// OPA base URL, without the fixed decision path.
    pub base_url: String,
    /// Fixed data decision path, such as `colossus/effect`.
    pub decision_path: String,
    /// Optional PEM CA bundle. Required for remote OPA unless runtime roots are supplied.
    pub ca_pem: Option<Vec<u8>>,
    /// Runtime-wide CA roots used when OPA does not configure its own pinned CA.
    pub tls_roots: AdditionalRootCertificates,
    /// PEM client certificate plus private key. Required for remote OPA.
    pub identity_pem: Option<Vec<u8>>,
    /// Explicit acknowledgement that full logical content is sent.
    pub full_content_disclosure_acknowledged: bool,
    /// Operator assertion that OPA decision logs are disabled or safely masked.
    pub decision_log_masking_verified: bool,
    /// Transport timeout.
    pub timeout: Duration,
}

/// OPA policy decision point. Colossus still enforces every returned obligation.
pub struct OpaPolicy {
    client: Client,
    decision_url: Url,
    ready_url: Url,
    decision_log_masking_verified: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpaDecisionResponse {
    result: PolicyDecision,
}

impl OpaPolicy {
    /// Validate disclosure/TLS invariants and construct the OPA client.
    pub fn new(config: OpaConfig) -> Result<Self, PolicyError> {
        if !config.full_content_disclosure_acknowledged {
            return Err(PolicyError::InvalidDecision(
                "full-content OPA disclosure acknowledgement is required".into(),
            ));
        }
        if config.decision_path.is_empty()
            || config.decision_path.starts_with('/')
            || config.decision_path.contains("..")
        {
            return Err(PolicyError::InvalidDecision(
                "OPA decision path must be fixed, relative, and non-traversing".into(),
            ));
        }
        let base = Url::parse(&config.base_url)
            .map_err(|error| PolicyError::InvalidDecision(error.to_string()))?;
        let local = base
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"));
        if !local && base.scheme() != "https" {
            return Err(PolicyError::InvalidDecision(
                "remote OPA requires HTTPS".into(),
            ));
        }
        if !local
            && ((config.ca_pem.is_none() && config.tls_roots.is_empty())
                || config.identity_pem.is_none())
        {
            return Err(PolicyError::InvalidDecision(
                "remote OPA requires pinned CA trust and mTLS identity".into(),
            ));
        }
        let mut builder = Client::builder().timeout(config.timeout);
        if let Some(ca_pem) = config.ca_pem {
            let certificates = Certificate::from_pem_bundle(&ca_pem)
                .map_err(|error| PolicyError::InvalidDecision(error.to_string()))?;
            if certificates.is_empty() {
                return Err(PolicyError::InvalidDecision(
                    "OPA CA bundle contains no certificates".into(),
                ));
            }
            builder = builder.tls_built_in_root_certs(false);
            for certificate in certificates {
                builder = builder.add_root_certificate(certificate);
            }
        } else if !local {
            builder = config
                .tls_roots
                .configure_reqwest(builder.tls_built_in_root_certs(false));
        } else {
            builder = config.tls_roots.configure_reqwest(builder);
        }
        if let Some(identity_pem) = config.identity_pem {
            let identity = Identity::from_pem(&identity_pem)
                .map_err(|error| PolicyError::InvalidDecision(error.to_string()))?;
            builder = builder.identity(identity);
        }
        let client = builder
            .build()
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        let decision_url = base
            .join(&format!("v1/data/{}", config.decision_path))
            .map_err(|error| PolicyError::InvalidDecision(error.to_string()))?;
        let ready_url = base
            .join("health?bundles=true&plugins=true")
            .map_err(|error| PolicyError::InvalidDecision(error.to_string()))?;
        Ok(Self {
            client,
            decision_url,
            ready_url,
            decision_log_masking_verified: config.decision_log_masking_verified,
        })
    }
}

#[async_trait]
impl PolicyDecisionPoint for OpaPolicy {
    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, PolicyError> {
        let response = self
            .client
            .post(self.decision_url.clone())
            .json(&json!({"input": request}))
            .send()
            .await
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        if !response.status().is_success() {
            return Err(PolicyError::Unavailable(format!(
                "OPA decision endpoint returned {}",
                response.status()
            )));
        }
        response
            .json::<OpaDecisionResponse>()
            .await
            .map(|response| response.result)
            .map_err(|error| PolicyError::InvalidDecision(error.to_string()))
    }

    async fn doctor(&self) -> Result<Value, PolicyError> {
        let response = self
            .client
            .get(self.ready_url.clone())
            .send()
            .await
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        if !response.status().is_success() {
            return Err(PolicyError::Unavailable(format!(
                "OPA readiness returned {}",
                response.status()
            )));
        }
        Ok(json!({
            "ready": true,
            "kind": "opa",
            "decision_url": self.decision_url.as_str(),
            "decision_log_masking_verified": self.decision_log_masking_verified,
            "warning": if self.decision_log_masking_verified {
                Value::Null
            } else {
                Value::String("OPA decision-log masking could not be verified".into())
            }
        }))
    }
}
