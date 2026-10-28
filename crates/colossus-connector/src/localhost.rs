//! RFC 6761 localhost routing without ambient DNS, URI or TLS-name changes.
use hyper_util::rt::TokioIo;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use tokio::net::TcpStream;
use tonic::transport::{Channel, Endpoint};

/// Only the reserved localhost label and its subdomains receive loopback routing.
/// One optional DNS root dot and ASCII case differences do not change that label.
fn addresses(host: &str, port: u16) -> Option<[SocketAddr; 2]> {
    let host = host.strip_suffix('.').unwrap_or(host);
    if host.split('.').any(str::is_empty)
        || !host
            .rsplit('.')
            .next()
            .is_some_and(|label| label.eq_ignore_ascii_case("localhost"))
    {
        return None;
    }
    Some([
        SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        SocketAddr::from((Ipv6Addr::LOCALHOST, port)),
    ])
}

fn loopback_ip(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    }
}
/// Configure a single-origin enrollment client. Localhost never consults ambient
/// DNS or a proxy; reqwest still verifies the original hostname and certificate.
pub(crate) fn http(builder: reqwest::ClientBuilder, url: &url::Url) -> reqwest::ClientBuilder {
    if loopback_ip(url) {
        return builder.no_proxy();
    }
    let Some((host, port)) = url.host_str().zip(url.port_or_known_default()) else {
        return builder;
    };
    match addresses(host, port) {
        Some(addresses) => builder.no_proxy().resolve_to_addrs(host, &addresses),
        None => builder,
    }
}

/// Dial reserved localhost names directly, retaining Endpoint's URI, TLS config,
/// SNI, and connection timeout. All other endpoints keep their ordinary resolver.
pub(crate) async fn connect(endpoint: Endpoint) -> Result<Channel, tonic::transport::Error> {
    let uri = endpoint.uri();
    let port = uri
        .port_u16()
        .unwrap_or(if uri.scheme_str() == Some("https") {
            443
        } else {
            80
        });
    let Some(addresses) = uri.host().and_then(|host| addresses(host, port)) else {
        return endpoint.connect().await;
    };
    endpoint
        .connect_with_connector(tower::service_fn(
            move |_: tonic::codegen::http::Uri| async move {
                TcpStream::connect(addresses.as_slice())
                    .await
                    .map(TokioIo::new)
            },
        ))
        .await
}

#[cfg(test)]
mod tests;
