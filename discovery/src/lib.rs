//! # Discovery
//!
//! LAN service advertisement (mDNS) for the AgentZero daemon. Provides an
//! `Advertiser` trait so the gateway can stay decoupled from the underlying
//! mDNS implementation and so tests can swap in a no-op or recorder.

#![forbid(unsafe_code)]

pub mod advertiser;
pub mod browser;
pub mod config;
pub mod interfaces;
pub mod mdns;
pub mod mdns_browser;
pub mod network_info;

pub use advertiser::{
    AdvertiseHandle, AdvertiseInner, Advertiser, DiscoveryError, NoopAdvertiser,
    Result as DiscoveryResult, ServiceInfo, noop,
};
pub use browser::{
    A2A_AGENT_CARD_PATH, A2A_AGENT_CARD_PATH_TXT, A2A_NODE_ID_TXT, A2A_VERSION, A2A_VERSION_TXT,
    BrowseConfig, BrowseHandle, BrowseInner, Browser, CandidateEvent, CandidateRecord,
    CandidateRegistry, DEFAULT_A2A_SERVICE_TYPE, DiscoveredService, advertise_a2a_txt_records,
    candidate_from_service, generate_a2a_node_id, start_browser_if_enabled,
};
pub use config::{AdvancedConfig, DiscoveryConfig, DiscoveryDetails};
pub use interfaces::{
    Interface, InterfaceEnumerator, RealEnumerator, filter_interfaces, ipv4_only,
};
pub use mdns::MdnsAdvertiser;
pub use mdns_browser::MdnsBrowser;
pub use network_info::{MdnsStatus, NetworkInfo, collect_network_info, sanitize_for_hostname};
