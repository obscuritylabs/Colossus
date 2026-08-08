use crate::{ReleaseFetch, ReleaseMetadata, ReleaseSource, ReleaseSourceFailure};
use async_trait::async_trait;
use colossus_network::{AdditionalRootCertificates, pinned_reqwest_client};
use futures::StreamExt as _;
use reqwest::{StatusCode, header};
use serde::Deserialize;
use std::time::Duration;
use url::Url;

const LATEST_STABLE_URL: &str =
    "https://api.github.com/repos/obscuritylabs/Colossus/releases/latest";
const RELEASE_PAGE_PREFIX: &str = "https://github.com/obscuritylabs/Colossus/releases/tag/";
const MAX_METADATA_BYTES: usize = 1024 * 1024;
const MAX_ASSETS: usize = 64;
const MAX_ASSET_NAME_BYTES: usize = 255;
const MAX_ETAG_BYTES: usize = 256;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

/// Anonymous, no-proxy, no-redirect adapter for the one official release endpoint.
#[derive(Clone)]
pub struct GitHubReleaseSource {
    endpoint: Url,
    allow_non_public: bool,
    timeout: Duration,
}

impl Default for GitHubReleaseSource {
    fn default() -> Self {
        Self::new()
    }
}

impl GitHubReleaseSource {
    /// Construct the production fixed-origin release source.
    pub fn new() -> Self {
        Self {
            endpoint: Url::parse(LATEST_STABLE_URL).expect("constant release URL is valid"),
            allow_non_public: false,
            timeout: REQUEST_TIMEOUT,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(endpoint: Url, timeout: Duration) -> Self {
        Self {
            endpoint,
            allow_non_public: true,
            timeout,
        }
    }

    async fn fetch(&self, etag: Option<&str>) -> Result<ReleaseFetch, ReleaseSourceFailure> {
        let timeout_ms = u64::try_from(self.timeout.as_millis()).unwrap_or(u64::MAX);
        let client = pinned_reqwest_client(
            &self.endpoint,
            &AdditionalRootCertificates::default(),
            timeout_ms,
            self.allow_non_public,
        )
        .await
        .map_err(|_| ReleaseSourceFailure::Offline)?;
        let mut request = client
            .get(self.endpoint.clone())
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header(header::USER_AGENT, "colossus-update-check/1");
        if let Some(etag) = etag {
            if !valid_etag(etag) {
                return Err(ReleaseSourceFailure::InvalidMetadata);
            }
            request = request.header(header::IF_NONE_MATCH, etag);
        }
        let response = request
            .send()
            .await
            .map_err(|_| ReleaseSourceFailure::Offline)?;
        if response.status() == StatusCode::NOT_MODIFIED {
            return Ok(ReleaseFetch::NotModified);
        }
        if matches!(
            response.status(),
            StatusCode::TOO_MANY_REQUESTS | StatusCode::FORBIDDEN
        ) {
            return Err(ReleaseSourceFailure::RateLimited {
                retry_after_seconds: retry_after_seconds(response.headers()),
            });
        }
        if response.status().is_server_error() {
            return Err(ReleaseSourceFailure::ServiceUnavailable);
        }
        if response.status() != StatusCode::OK {
            return Err(ReleaseSourceFailure::InvalidMetadata);
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_METADATA_BYTES as u64)
        {
            return Err(ReleaseSourceFailure::InvalidMetadata);
        }
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !content_type
            .split(';')
            .next()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
        {
            return Err(ReleaseSourceFailure::InvalidMetadata);
        }
        let etag = response
            .headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .filter(|value| valid_etag(value));
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| ReleaseSourceFailure::Offline)?;
            if body.len().saturating_add(chunk.len()) > MAX_METADATA_BYTES {
                return Err(ReleaseSourceFailure::InvalidMetadata);
            }
            body.extend_from_slice(&chunk);
        }
        parse_release(&body, etag)
    }
}

#[async_trait]
impl ReleaseSource for GitHubReleaseSource {
    async fn latest_stable(
        &self,
        etag: Option<&str>,
    ) -> Result<ReleaseFetch, ReleaseSourceFailure> {
        match tokio::time::timeout(self.timeout, self.fetch(etag)).await {
            Ok(result) => result,
            Err(_) => Err(ReleaseSourceFailure::Offline),
        }
    }
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    html_url: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
}

fn parse_release(body: &[u8], etag: Option<String>) -> Result<ReleaseFetch, ReleaseSourceFailure> {
    let release: GitHubRelease =
        serde_json::from_slice(body).map_err(|_| ReleaseSourceFailure::InvalidMetadata)?;
    if release.draft || release.prerelease || release.assets.len() > MAX_ASSETS {
        return Err(ReleaseSourceFailure::InvalidMetadata);
    }
    let version = release
        .tag_name
        .strip_prefix('v')
        .filter(|version| !version.is_empty())
        .ok_or(ReleaseSourceFailure::InvalidMetadata)?;
    let expected_page = format!("{RELEASE_PAGE_PREFIX}{}", release.tag_name);
    if release.html_url != expected_page {
        return Err(ReleaseSourceFailure::InvalidMetadata);
    }
    let mut asset_names = Vec::with_capacity(release.assets.len());
    for asset in release.assets {
        if asset.name.is_empty()
            || asset.name.len() > MAX_ASSET_NAME_BYTES
            || asset
                .name
                .bytes()
                .any(|byte| byte.is_ascii_control() || !byte.is_ascii())
            || asset_names.contains(&asset.name)
        {
            return Err(ReleaseSourceFailure::InvalidMetadata);
        }
        asset_names.push(asset.name);
    }
    Ok(ReleaseFetch::Modified(ReleaseMetadata {
        version: version.to_owned(),
        release_url: release.html_url,
        asset_names,
        etag,
    }))
}

fn valid_etag(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ETAG_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii() && !byte.is_ascii_control())
}

fn retry_after_seconds(headers: &header::HeaderMap) -> Option<u64> {
    headers
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= 20 && value.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| seconds.min(24 * 60 * 60))
}
