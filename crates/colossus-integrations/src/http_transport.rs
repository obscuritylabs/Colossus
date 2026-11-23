use super::*;
use reqwest::{Client, Method, Response};

/// A single authorized integration origin, with pinned DNS and one credential binding
/// for all requests in the effect. The gateway owns the outer execution deadline.
pub(super) struct IntegrationHttp<'a> {
    client: Client,
    permit: &'a ExecutionPermit,
    origin: url::Origin,
    authorization: Option<(HeaderName, HeaderValue)>,
    sensitive_values: Vec<String>,
    github: bool,
}

impl<'a> IntegrationHttp<'a> {
    pub(super) async fn new(
        connection: &IntegrationConnection,
        url: &Url,
        permit: &'a ExecutionPermit,
        tls_roots: &AdditionalRootCertificates,
    ) -> Result<Self, ExecutionError> {
        let matched = require_origin(url, permit)?;
        let host = url
            .host_str()
            .ok_or_else(|| execution("integration URL has no host"))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| execution("integration URL has no port"))?;
        let allow_non_public = matched == NetworkDestinationMatch::Ambient
            || (matched == NetworkDestinationMatch::Exact
                && (host.eq_ignore_ascii_case("localhost")
                    || host.parse::<IpAddr>().is_ok_and(non_public_network_address)));
        let addresses = resolve_integration_addresses(host, port, allow_non_public).await?;
        let client = tls_roots
            .configure_reqwest(Client::builder())
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(host, &addresses)
            .timeout(Duration::from_millis(permit.obligations().timeout_ms))
            .build()
            .map_err(execution)?;
        let credential_value = connection
            .credential_reference
            .as_deref()
            .map(resolve_environment)
            .transpose()?;
        let authorization = auth_header(&connection.auth, credential_value.as_deref())?;
        let mut sensitive_values = credential_value.into_iter().collect::<Vec<_>>();
        if let Some(value) = authorization
            .as_ref()
            .and_then(|(_, value)| value.to_str().ok())
        {
            sensitive_values.push(value.into());
            if let Some((_, token)) = value.split_once(' ') {
                sensitive_values.push(token.into());
            }
        }
        Ok(Self {
            client,
            permit,
            origin: url.origin(),
            authorization,
            sensitive_values,
            github: connection.name == "github",
        })
    }

    pub(super) async fn send(
        &self,
        prepared: PreparedHttpRequest,
    ) -> Result<Response, ExecutionError> {
        let PreparedHttpRequest { method, url, body } = prepared;
        if url.origin() != self.origin {
            return Err(execution("integration request changed its bound origin"));
        }
        require_origin(&url, self.permit)?;
        let mut request = self
            .client
            .request(method.clone(), url)
            .header("accept", "application/json")
            .header("user-agent", "colossus/0.6");
        if let Some((name, value)) = &self.authorization {
            request = request.header(name, value);
        }
        if self.github {
            request = request.header("x-github-api-version", "2022-11-28");
        }
        if let Some(body) = body.as_ref() {
            request = request.json(body);
        }
        request.send().await.map_err(|error| {
            if method == Method::GET {
                execution(format!("integration request failed: {}", error.classify()))
            } else {
                ExecutionError::OutcomeUnknown(format!(
                    "integration transport failed after a potentially mutating request: {}",
                    error.classify()
                ))
            }
        })
    }

    pub(super) fn redact(&self, bytes: &mut Vec<u8>) {
        for secret in &self.sensitive_values {
            *bytes = super::executor::redact_exact_secret(bytes, secret.as_bytes());
        }
    }

    pub(super) fn redact_text(&self, text: &str) -> String {
        let mut bytes = text.as_bytes().to_vec();
        self.redact(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    }
}
