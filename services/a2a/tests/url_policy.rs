use gateway_a2a::url_policy::{ResolvedPeerEndpoint, UrlPolicy};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[test]
fn normalizes_https_and_rejects_credential_bearing_or_mutable_urls() {
    let endpoint = UrlPolicy::validate_peer_origin("https://Example.COM:18791/a2a", false).unwrap();
    assert_eq!(endpoint.origin(), "https://example.com:18791");

    for url in [
        "https://user:pass@example.com",
        "https://example.com?token=x",
        "https://example.com/#frag",
        "ftp://example.com",
    ] {
        assert!(
            UrlPolicy::validate_peer_origin(url, false).is_err(),
            "{url}"
        );
    }
}

#[test]
fn http_requires_loopback_or_explicit_private_network_opt_in() {
    assert!(UrlPolicy::validate_peer_origin("http://127.0.0.1:18791", false).is_ok());
    assert!(UrlPolicy::validate_peer_origin("http://[::1]:18791", false).is_ok());
    assert!(UrlPolicy::validate_peer_origin("http://192.168.1.20:18791", false).is_err());
    assert!(UrlPolicy::validate_peer_origin("http://192.168.1.20:18791", true).is_ok());
    assert!(UrlPolicy::validate_peer_origin("http://example.com:18791", false).is_err());
}

#[test]
fn resolution_rejects_prohibited_destinations_and_rebinding() {
    let endpoint = UrlPolicy::validate_peer_origin("https://peer.example:18791", false).unwrap();
    assert!(endpoint
        .validate_resolution([IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))])
        .is_ok());
    assert!(endpoint
        .validate_resolution([IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))])
        .is_err());
    assert!(endpoint
        .validate_resolution([IpAddr::V6(Ipv6Addr::LOCALHOST)])
        .is_err());

    let first =
        ResolvedPeerEndpoint::new(endpoint.clone(), [IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))])
            .unwrap();
    assert!(first
        .reject_rebind([IpAddr::V4(Ipv4Addr::new(8, 8, 4, 4))])
        .is_err());
}
