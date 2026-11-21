use super::{PinnedHttpClientError, non_public_network_address, resolve_destinations};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

#[test]
fn reserved_ipv4_addresses_are_non_public() {
    // IANA's Reserved 240.0.0.0/4 allocation is not globally reachable.
    for address in [
        Ipv4Addr::new(240, 0, 0, 0),
        Ipv4Addr::new(240, 0, 0, 1),
        Ipv4Addr::new(248, 1, 2, 3),
        Ipv4Addr::new(255, 255, 255, 254),
    ] {
        assert!(non_public_network_address(IpAddr::V4(address)), "{address}");
    }
}

#[test]
fn reserved_ipv4_mapped_addresses_are_non_public() {
    for address in [
        Ipv4Addr::new(240, 0, 0, 0),
        Ipv4Addr::new(240, 0, 0, 1),
        Ipv4Addr::new(248, 1, 2, 3),
        Ipv4Addr::new(255, 255, 255, 254),
    ] {
        let mapped = address.to_ipv6_mapped();
        assert!(non_public_network_address(IpAddr::V6(mapped)), "{mapped}");
    }
}

#[test]
fn reserved_ipv4_classification_preserves_public_and_adjacent_ranges() {
    for (address, non_public) in [
        (Ipv4Addr::new(223, 255, 255, 254), false),
        (Ipv4Addr::new(192, 0, 0, 9), false),
        (Ipv4Addr::new(192, 0, 0, 10), false),
        (Ipv4Addr::new(239, 255, 255, 255), true),
        (Ipv4Addr::BROADCAST, true),
    ] {
        assert_eq!(
            non_public_network_address(IpAddr::V4(address)),
            non_public,
            "{address}"
        );
        let mapped = address.to_ipv6_mapped();
        assert_eq!(
            non_public_network_address(IpAddr::V6(mapped)),
            non_public,
            "{mapped}"
        );
    }
}

#[tokio::test]
async fn reserved_ipv4_numeric_resolution_requires_non_public_authority() {
    // Numeric literals use local address parsing without DNS or a connection.
    for host in ["240.0.0.1", "::ffff:240.0.0.1"] {
        assert!(
            matches!(
                resolve_destinations(host, 443, false).await,
                Err(PinnedHttpClientError::Resolution(message))
                    if message == "destination resolved to no permitted address"
            ),
            "{host}"
        );
        let expected = SocketAddr::new(host.parse().expect("numeric IP literal"), 443);
        assert_eq!(
            resolve_destinations(host, 443, true)
                .await
                .expect("explicit non-public authority"),
            vec![expected],
            "{host}"
        );
    }
}
