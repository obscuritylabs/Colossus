use crate::AdditionalRootCertificates;
use reqwest::{Client, redirect::Policy as RedirectPolicy};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use thiserror::Error;
use tokio::net::lookup_host;
use url::Url;

const MAX_PINNED_ADDRESSES: usize = 16;

/// Failure while constructing a DNS-pinned Colossus HTTP client.
#[derive(Debug, Error)]
pub enum PinnedHttpClientError {
    /// The URL cannot be used for a bounded HTTP request.
    #[error("invalid HTTP destination: {0}")]
    InvalidDestination(String),
    /// DNS lookup failed or resolved only to denied addresses.
    #[error("HTTP destination resolution failed: {0}")]
    Resolution(String),
    /// The hardened reqwest client could not be constructed.
    #[error("HTTP client construction failed: {0}")]
    Client(String),
}

/// Construct a no-proxy, no-redirect, DNS-pinned HTTP client for one exact URL.
///
/// Callers decide whether non-public addresses are permitted from their already
/// validated policy match. The client pins all requests for the URL's hostname
/// to the addresses resolved here and applies the supplied timeout and CA roots.
pub async fn pinned_reqwest_client(
    url: &Url,
    tls_roots: &AdditionalRootCertificates,
    timeout_ms: u64,
    allow_non_public: bool,
) -> Result<Client, PinnedHttpClientError> {
    let host = url
        .host_str()
        .ok_or_else(|| PinnedHttpClientError::InvalidDestination("URL has no host".into()))?;
    let resolution_host = unbracketed_host(host);
    let port = url
        .port_or_known_default()
        .ok_or_else(|| PinnedHttpClientError::InvalidDestination("URL has no known port".into()))?;
    let addresses = resolve_destinations(resolution_host, port, allow_non_public).await?;
    tls_roots
        .configure_reqwest(Client::builder())
        .redirect(RedirectPolicy::none())
        .no_proxy()
        .resolve_to_addrs(resolution_host, &addresses)
        .timeout(Duration::from_millis(timeout_ms))
        .build()
        .map_err(|error| PinnedHttpClientError::Client(error.to_string()))
}

/// Parse a URL host string as an IP address, accepting bracketed IPv6 serialization.
pub fn parse_host_ip(host: &str) -> Option<IpAddr> {
    unbracketed_host(host).parse().ok()
}

fn unbracketed_host(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host)
}

/// Resolve and bound the addresses pinned for one network operation.
pub async fn resolve_destinations(
    host: &str,
    port: u16,
    allow_non_public: bool,
) -> Result<Vec<SocketAddr>, PinnedHttpClientError> {
    let mut addresses = lookup_host((host, port))
        .await
        .map_err(|error| PinnedHttpClientError::Resolution(error.to_string()))?
        .filter(|address| allow_non_public || !non_public_network_address(address.ip()))
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(PinnedHttpClientError::Resolution(
            "destination resolved to no permitted address".into(),
        ));
    }
    addresses.sort_by_key(|address| usize::from(address.is_ipv6()));
    addresses.dedup();
    addresses.truncate(MAX_PINNED_ADDRESSES);
    Ok(addresses)
}

/// Return whether an address is outside public Internet routing.
pub fn non_public_network_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
                || ip.octets()[0] == 0
                || matches!(ip.octets(), [100, second, _, _] if (64..=127).contains(&second))
                || matches!(ip.octets(), [198, second, _, _] if matches!(second, 18 | 19))
        }
        IpAddr::V6(ip) => {
            ip.to_ipv4_mapped()
                .is_some_and(|mapped| non_public_network_address(IpAddr::V4(mapped)))
                || ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
                || matches!(ip.segments(), [0x2001, 0x0db8, ..])
        }
    }
}
