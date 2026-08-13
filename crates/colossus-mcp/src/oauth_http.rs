use colossus_contracts::ResourceAuthority;
use colossus_network::{AdditionalRootCertificates, pinned_reqwest_client};
use colossus_policy::{
    NetworkDestinationMatch, canonical_network_origin, network_destination_match,
    non_public_network_address,
};
use futures::StreamExt as _;
use reqwest::Url;
use rmcp::transport::auth::{
    OAuthHttpClient, OAuthHttpClientError, OAuthHttpClientFuture, OAuthHttpRequest,
};
use std::time::Duration;

#[derive(Clone)]
pub(super) struct HardenedOAuthHttpClient {
    resource_authority: ResourceAuthority,
    destinations: Vec<String>,
    tls_roots: AdditionalRootCertificates,
    timeout_ms: u64,
    max_response_bytes: usize,
}

impl HardenedOAuthHttpClient {
    pub(super) fn new(
        resource_authority: ResourceAuthority,
        destinations: Vec<String>,
        tls_roots: AdditionalRootCertificates,
        timeout_ms: u64,
        max_response_bytes: usize,
    ) -> Self {
        Self {
            resource_authority,
            destinations,
            tls_roots,
            timeout_ms,
            max_response_bytes,
        }
    }
}

impl OAuthHttpClient for HardenedOAuthHttpClient {
    fn execute(&self, request: OAuthHttpRequest) -> OAuthHttpClientFuture<'_> {
        Box::pin(async move {
            let url = Url::parse(&request.request.uri().to_string())
                .map_err(|_| oauth_error("OAuth request URL is invalid"))?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
            {
                return Err(oauth_error("OAuth request URL is unsafe"));
            }
            let host = url
                .host_str()
                .ok_or_else(|| oauth_error("OAuth request URL has no host"))?;
            let loopback = host.eq_ignore_ascii_case("localhost")
                || colossus_network::parse_host_ip(host)
                    .is_some_and(|address| address.is_loopback());
            if self.resource_authority != ResourceAuthority::Ambient
                && url.scheme() != "https"
                && !loopback
            {
                return Err(oauth_error("OAuth request requires HTTPS"));
            }
            let origin = url.origin().ascii_serialization();
            let matched = if self.resource_authority == ResourceAuthority::Ambient {
                canonical_network_origin(&origin)
                    .map_err(|_| oauth_error("OAuth request origin is invalid"))?;
                NetworkDestinationMatch::Ambient
            } else {
                network_destination_match(&self.destinations, &origin)
                    .map_err(|_| oauth_error("OAuth request origin is invalid"))?
                    .ok_or_else(|| oauth_error("OAuth request origin is not permitted"))?
            };
            let allow_non_public = matched == NetworkDestinationMatch::Ambient
                || (matched == NetworkDestinationMatch::Exact
                    && (host.eq_ignore_ascii_case("localhost")
                        || colossus_network::parse_host_ip(host)
                            .is_some_and(non_public_network_address)));
            let timeout_ms = request
                .timeout
                .map(|value| {
                    u64::try_from(value.as_millis())
                        .unwrap_or(u64::MAX)
                        .min(self.timeout_ms)
                })
                .unwrap_or(self.timeout_ms);
            let client = pinned_reqwest_client(&url, &self.tls_roots, timeout_ms, allow_non_public)
                .await
                .map_err(|_| oauth_error("OAuth HTTP client construction failed"))?;
            let method = reqwest::Method::from_bytes(request.request.method().as_str().as_bytes())
                .map_err(|_| oauth_error("OAuth request method is invalid"))?;
            let mut builder = client.request(method, url);
            for (name, value) in request.request.headers() {
                builder = builder.header(name, value);
            }
            if !request.request.body().is_empty() {
                builder = builder.body(request.request.body().clone());
            }
            let response = tokio::time::timeout(Duration::from_millis(timeout_ms), builder.send())
                .await
                .map_err(|_| oauth_error("OAuth request timed out"))?
                .map_err(|_| oauth_error("OAuth request failed"))?;
            if response.status().is_redirection() {
                return Err(oauth_error("OAuth HTTP redirects are disabled"));
            }
            let mut response_builder = http::Response::builder()
                .status(response.status())
                .version(response.version());
            for (name, value) in response.headers() {
                response_builder = response_builder.header(name, value);
            }
            let mut body = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| oauth_error("OAuth response read failed"))?;
                if body.len().saturating_add(chunk.len()) > self.max_response_bytes {
                    return Err(oauth_error("OAuth response exceeded its permitted bound"));
                }
                body.extend_from_slice(&chunk);
            }
            response_builder
                .body(body)
                .map_err(|_| oauth_error("OAuth response was invalid"))
        })
    }
}

fn oauth_error(message: &str) -> OAuthHttpClientError {
    OAuthHttpClientError::new(message)
}
