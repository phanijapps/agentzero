use discovery::{
    advertise_a2a_txt_records, candidate_from_service, start_browser_if_enabled, BrowseConfig,
    Browser, CandidateEvent, CandidateRegistry, DiscoveredService, DiscoveryResult,
};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

fn service(node_id: &str, now: Instant) -> DiscoveredService {
    let mut txt = BTreeMap::new();
    txt.insert("nodeId".to_string(), node_id.to_string());
    txt.insert("a2aVersion".to_string(), "1.0".to_string());
    txt.insert(
        "agentCardPath".to_string(),
        "/.well-known/agent-card.json".to_string(),
    );

    DiscoveredService {
        full_name: format!("zbot-a._zbot._tcp.local.{node_id}"),
        instance_name: "zbot-a".to_string(),
        addresses: vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))],
        port: 18791,
        txt,
        observed_at: now,
    }
}

#[test]
fn candidate_events_are_untrusted_and_expire() {
    let now = Instant::now();
    let registry = CandidateRegistry::default();
    registry.mark_trusted("trusted-peer");

    let candidate = candidate_from_service(
        service("11111111-1111-4111-8111-111111111111", now),
        Duration::from_secs(2),
    )
    .expect("valid candidate");
    registry.apply_event(CandidateEvent::Upsert(candidate));

    assert!(registry
        .candidate("11111111-1111-4111-8111-111111111111")
        .is_some());
    assert!(!registry.is_trusted("11111111-1111-4111-8111-111111111111"));
    assert!(registry.is_trusted("trusted-peer"));

    let updated = candidate_from_service(
        DiscoveredService {
            port: 18792,
            observed_at: now + Duration::from_secs(1),
            ..service("11111111-1111-4111-8111-111111111111", now)
        },
        Duration::from_secs(2),
    )
    .expect("valid update");
    registry.apply_event(CandidateEvent::Upsert(updated));
    assert_eq!(
        registry
            .candidate("11111111-1111-4111-8111-111111111111")
            .expect("candidate exists")
            .port,
        18792
    );

    registry.expire(now + Duration::from_secs(4));
    assert!(registry
        .candidate("11111111-1111-4111-8111-111111111111")
        .is_none());
    assert!(registry.is_trusted("trusted-peer"));
}

#[test]
fn malformed_txt_records_are_rejected() {
    let now = Instant::now();
    let valid = service("22222222-2222-4222-8222-222222222222", now);

    for (key, value) in [
        ("nodeId", "host-alice-ward"),
        ("a2aVersion", "2.0"),
        ("agentCardPath", "https://example.test/card"),
    ] {
        let mut malformed = valid.clone();
        malformed.txt.insert(key.to_string(), value.to_string());
        assert!(
            candidate_from_service(malformed, Duration::from_secs(30)).is_none(),
            "{key}={value} should be rejected"
        );
    }

    let mut missing = valid;
    missing.txt.remove("nodeId");
    assert!(candidate_from_service(missing, Duration::from_secs(30)).is_none());
}

#[test]
fn random_node_id_contains_no_local_identity() {
    let host = "alice-mbp";
    let ward = "research-ward";
    let agent = "root-agent";

    let first = discovery::generate_a2a_node_id();
    let second = discovery::generate_a2a_node_id();

    assert_ne!(first, second);
    for forbidden in [host, "alice", "mbp", ward, "research", agent, "root"] {
        assert!(
            !first.contains(forbidden) && !second.contains(forbidden),
            "node id leaked local identifier fragment: {forbidden}"
        );
    }
}

#[test]
fn default_off_lifecycle_does_not_start_browser() {
    static STARTS: AtomicUsize = AtomicUsize::new(0);

    struct CountingBrowser;
    impl Browser for CountingBrowser {
        fn browse(
            &self,
            _config: BrowseConfig,
            _registry: CandidateRegistry,
        ) -> DiscoveryResult<discovery::BrowseHandle> {
            STARTS.fetch_add(1, Ordering::SeqCst);
            unreachable!("disabled browser must not start");
        }
    }

    let handle = start_browser_if_enabled(
        BrowseConfig::disabled(),
        &CountingBrowser,
        CandidateRegistry::default(),
    )
    .expect("disabled lifecycle succeeds");

    assert!(handle.is_none());
    assert_eq!(STARTS.load(Ordering::SeqCst), 0);
}

#[test]
fn advertised_a2a_txt_records_are_bootstrap_only() {
    let txt = advertise_a2a_txt_records(
        "33333333-3333-4333-8333-333333333333",
        "/.well-known/agent-card.json",
    )
    .expect("valid txt records");

    assert_eq!(
        txt.get("nodeId").map(String::as_str),
        Some("33333333-3333-4333-8333-333333333333")
    );
    assert_eq!(txt.get("a2aVersion").map(String::as_str), Some("1.0"));
    assert_eq!(
        txt.get("agentCardPath").map(String::as_str),
        Some("/.well-known/agent-card.json")
    );
    assert!(!txt.contains_key("credential"));
    assert!(!txt.contains_key("token"));
}

#[test]
#[ignore]
fn loopback_goodbye_removes_candidate() {
    let registry = CandidateRegistry::default();
    let browser = discovery::MdnsBrowser::new().expect("browser starts");
    let _browse = browser
        .browse(BrowseConfig::enabled("_zbot._tcp.local."), registry.clone())
        .expect("browse starts");

    let advertiser = discovery::MdnsAdvertiser::new().expect("advertiser starts");
    let mut txt = advertise_a2a_txt_records(
        "44444444-4444-4444-8444-444444444444",
        "/.well-known/agent-card.json",
    )
    .expect("valid txt");
    txt.insert("displayName".to_string(), "local test".to_string());

    let handle = discovery::advertiser::Advertiser::advertise(
        &advertiser,
        discovery::ServiceInfo {
            instance_name: "a2a-loopback".to_string(),
            service_type: "_zbot._tcp.local.".to_string(),
            hostname_alias: "a2a-loopback.local".to_string(),
            port: 19191,
            txt,
            addrs: vec![("lo".to_string(), Ipv4Addr::LOCALHOST)],
        },
    )
    .expect("advertise succeeds");

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline
        && registry
            .candidate("44444444-4444-4444-8444-444444444444")
            .is_none()
    {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(registry
        .candidate("44444444-4444-4444-8444-444444444444")
        .is_some());

    drop(handle);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline
        && registry
            .candidate("44444444-4444-4444-8444-444444444444")
            .is_some()
    {
        std::thread::sleep(Duration::from_millis(100));
    }

    assert!(registry
        .candidate("44444444-4444-4444-8444-444444444444")
        .is_none());
}
