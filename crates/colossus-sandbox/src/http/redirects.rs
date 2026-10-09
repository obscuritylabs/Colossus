//! Redirect parsing and bounded traversal; reqwest automatic redirects stay disabled.

use super::*;
use reqwest::{
    StatusCode,
    header::{HeaderMap, LOCATION},
};

pub(super) const MAX_HTTP_URL_BYTES: usize = 8192;

pub(super) fn is_redirect(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

pub(super) struct RedirectChain {
    limit: usize,
    followed: usize,
    visited: BTreeSet<String>,
}

impl RedirectChain {
    pub(super) fn new(initial: &Url, limit: usize) -> Self {
        Self {
            limit,
            followed: 0,
            visited: BTreeSet::from([initial.as_str().to_owned()]),
        }
    }

    pub(super) fn next(
        &mut self,
        current: &Url,
        headers: &HeaderMap,
    ) -> Result<Url, ExecutionError> {
        if self.followed >= self.limit {
            return Err(adapter_failure(format!(
                "HTTP redirect limit exceeded (maxRedirects: {})",
                self.limit
            )));
        }
        let mut locations = headers.get_all(LOCATION).iter();
        let location = locations
            .next()
            .ok_or_else(|| adapter_failure("HTTP redirect is missing Location"))?;
        if locations.next().is_some() {
            return Err(adapter_failure(
                "HTTP redirect has multiple Location headers",
            ));
        }
        let location = location
            .to_str()
            .map_err(|_| adapter_failure("HTTP redirect Location is not valid text"))?
            .trim();
        if location.is_empty() || location.len() > MAX_HTTP_URL_BYTES {
            return Err(adapter_failure(
                "HTTP redirect Location is empty or exceeds 8192 bytes",
            ));
        }
        let mut next = current
            .join(location)
            .map_err(|_| adapter_failure("HTTP redirect Location is not a valid URL"))?;
        next.set_fragment(None);
        if next.as_str().len() > MAX_HTTP_URL_BYTES {
            return Err(adapter_failure("HTTP redirect URL exceeds 8192 bytes"));
        }
        if current.scheme() == "https" && next.scheme() == "http" {
            return Err(adapter_failure(
                "HTTP redirect cannot downgrade HTTPS to HTTP",
            ));
        }
        // The adapter rechecks authority and pins fresh DNS before sending this URL.
        if !self.visited.insert(next.as_str().to_owned()) {
            return Err(adapter_failure("HTTP redirect loop detected"));
        }
        self.followed += 1;
        Ok(next)
    }
}
