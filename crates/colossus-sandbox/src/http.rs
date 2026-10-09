//! Permit-bound HTTP fetching with explicitly validated redirect hops.

use super::*;
use colossus_network::{parse_host_ip, pinned_reqwest_client};

mod redirects;
mod request;
mod response;

#[cfg(test)]
mod tests;

/// Default number of redirects followed by bodyless brokered HTTP GET/HEAD requests.
pub const DEFAULT_HTTP_MAX_REDIRECTS: usize = 10;
/// Hard ceiling for the configurable brokered HTTP redirect limit.
pub const MAX_HTTP_REDIRECTS: usize = 20;

/// Permit-bound HTTP adapter with DNS pinning and bounded response streaming.
///
/// Only bodyless GET/HEAD requests can follow redirects. Every hop must satisfy the
/// original permit's transport and destination obligations; HTTPS cannot downgrade.
/// The gateway's deadline covers the complete chain, including DNS and the final body.
pub struct HttpExecutor {
    tls_roots: AdditionalRootCertificates,
    max_redirects: usize,
}

impl Default for HttpExecutor {
    fn default() -> Self {
        Self {
            tls_roots: AdditionalRootCertificates::default(),
            max_redirects: DEFAULT_HTTP_MAX_REDIRECTS,
        }
    }
}

impl HttpExecutor {
    /// Construct the brokered HTTP adapter with the default redirect limit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the redirect limit, from zero (disabled) through [`MAX_HTTP_REDIRECTS`].
    ///
    /// Mutating or body-bearing requests and WORM writes never follow redirects.
    pub fn with_max_redirects(mut self, max_redirects: usize) -> Result<Self, ExecutionError> {
        if max_redirects > MAX_HTTP_REDIRECTS {
            return Err(adapter_failure(format!(
                "HTTP maxRedirects must be at most {MAX_HTTP_REDIRECTS}"
            )));
        }
        self.max_redirects = max_redirects;
        Ok(self)
    }

    /// Add validated runtime-wide CA roots to HTTP clients' built-in public roots.
    #[must_use]
    pub fn with_tls_roots(mut self, tls_roots: AdditionalRootCertificates) -> Self {
        self.tls_roots = tls_roots;
        self
    }

    async fn client(
        &self,
        url: &Url,
        obligations: &PolicyObligations,
    ) -> Result<reqwest::Client, ExecutionError> {
        let matched = http_transport_authority_match(obligations, url.as_str())
            .map_err(adapter_failure)?
            .ok_or_else(|| adapter_failure("HTTP origin is not permitted"))?;
        let allow_non_public = matched == NetworkDestinationMatch::Ambient
            || (matched == NetworkDestinationMatch::Exact
                && url.host_str().is_some_and(|host| {
                    host.eq_ignore_ascii_case("localhost")
                        || parse_host_ip(host).is_some_and(non_public_network_address)
                }));
        pinned_reqwest_client(
            url,
            &self.tls_roots,
            obligations.timeout_ms,
            allow_non_public,
        )
        .await
        .map_err(adapter_failure)
    }
}

#[async_trait]
impl EffectExecutor for HttpExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let worm_write = request.action == "audit.export.worm.write";
        if request.action != "network.http" && !worm_write {
            return Err(adapter_failure("HTTP executor received another action"));
        }
        if request.resource.len() > redirects::MAX_HTTP_URL_BYTES {
            return Err(adapter_failure("HTTP destination URL exceeds 8192 bytes"));
        }
        let mut url = Url::parse(&request.resource)
            .map_err(|_| adapter_failure("invalid HTTP destination URL"))?;
        if worm_write && url.scheme() != "https" {
            return Err(adapter_failure("WORM audit export requires HTTPS"));
        }
        if worm_write
            && (!url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some())
        {
            return Err(adapter_failure(
                "WORM audit export URL must not contain credentials, a query, or a fragment",
            ));
        }
        url.set_fragment(None);
        let method = request
            .content
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("GET");
        let follow_redirects = !worm_write
            && matches!(method, "GET" | "HEAD")
            && request.content.get("body_base64").is_none();
        let mut redirects = redirects::RedirectChain::new(&url, self.max_redirects);
        loop {
            let client = self.client(&url, permit.obligations()).await?;
            let response = request::build_request(&client, &url, request, permit.obligations())?
                .send()
                .await
                .map_err(|error| {
                    // Redirect URLs can carry SAML assertions or other query secrets.
                    let error = error.without_url();
                    if worm_write {
                        ExecutionError::OutcomeUnknown(format!(
                            "WORM audit delivery transport failed: {error}"
                        ))
                    } else {
                        adapter_failure(error)
                    }
                })?;
            if follow_redirects && redirects::is_redirect(response.status()) {
                url = redirects.next(&url, response.headers())?;
                continue;
            }
            return response::quarantine(
                response,
                worm_write,
                permit.obligations().max_output_bytes,
            )
            .await;
        }
    }
}
