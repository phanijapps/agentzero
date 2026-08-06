//! Browser abstractions for untrusted A2A discovery candidates.

use crate::{DiscoveryError, DiscoveryResult};
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const A2A_VERSION: &str = "1.0";
pub const A2A_AGENT_CARD_PATH: &str = "/.well-known/agent-card.json";
pub const A2A_NODE_ID_TXT: &str = "nodeId";
pub const A2A_VERSION_TXT: &str = "a2aVersion";
pub const A2A_AGENT_CARD_PATH_TXT: &str = "agentCardPath";
pub const DEFAULT_A2A_SERVICE_TYPE: &str = "_zbot._tcp.local.";

const MAX_NODE_ID_BYTES: usize = 128;
const MAX_INSTANCE_NAME_BYTES: usize = 80;
const MAX_AGENT_CARD_PATH_BYTES: usize = 128;
const MAX_TXT_VALUE_BYTES: usize = 512;
const MAX_ADDRESSES: usize = 16;

#[derive(Debug, Clone)]
pub struct BrowseConfig {
    pub enabled: bool,
    pub service_type: String,
    pub candidate_ttl: Duration,
}

impl BrowseConfig {
    pub fn enabled(service_type: impl Into<String>) -> Self {
        Self {
            enabled: true,
            service_type: normalize_service_type(service_type.into()),
            candidate_ttl: Duration::from_secs(120),
        }
    }

    pub fn disabled() -> Self {
        Self {
            enabled: false,
            service_type: DEFAULT_A2A_SERVICE_TYPE.to_string(),
            candidate_ttl: Duration::from_secs(120),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredService {
    pub full_name: String,
    pub instance_name: String,
    pub addresses: Vec<IpAddr>,
    pub port: u16,
    pub txt: BTreeMap<String, String>,
    pub observed_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRecord {
    pub node_id: String,
    pub instance_name: String,
    pub full_name: String,
    pub addresses: Vec<IpAddr>,
    pub port: u16,
    pub agent_card_path: String,
    pub protocol_version: String,
    pub observed_at: Instant,
    pub expires_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateEvent {
    Upsert(CandidateRecord),
    Remove { full_name: String },
}

#[derive(Debug, Default, Clone)]
pub struct CandidateRegistry {
    inner: Arc<Mutex<CandidateRegistryInner>>,
}

#[derive(Debug, Default)]
struct CandidateRegistryInner {
    candidates: BTreeMap<String, CandidateRecord>,
    full_name_to_node_id: BTreeMap<String, String>,
    trusted_node_ids: BTreeSet<String>,
}

impl CandidateRegistry {
    pub fn apply_event(&self, event: CandidateEvent) {
        let mut inner = self.inner.lock().expect("candidate registry poisoned");
        match event {
            CandidateEvent::Upsert(candidate) => {
                inner
                    .full_name_to_node_id
                    .insert(candidate.full_name.clone(), candidate.node_id.clone());
                inner
                    .candidates
                    .insert(candidate.node_id.clone(), candidate);
            }
            CandidateEvent::Remove { full_name } => {
                if let Some(node_id) = inner.full_name_to_node_id.remove(&full_name) {
                    inner.candidates.remove(&node_id);
                }
            }
        }
    }

    pub fn expire(&self, now: Instant) -> Vec<String> {
        let mut inner = self.inner.lock().expect("candidate registry poisoned");
        let expired: Vec<String> = inner
            .candidates
            .iter()
            .filter(|(_, candidate)| candidate.expires_at <= now)
            .map(|(node_id, _)| node_id.clone())
            .collect();
        for node_id in &expired {
            if let Some(candidate) = inner.candidates.remove(node_id) {
                inner.full_name_to_node_id.remove(&candidate.full_name);
            }
        }
        expired
    }

    pub fn candidate(&self, node_id: &str) -> Option<CandidateRecord> {
        self.inner
            .lock()
            .expect("candidate registry poisoned")
            .candidates
            .get(node_id)
            .cloned()
    }

    pub fn candidates(&self) -> Vec<CandidateRecord> {
        self.inner
            .lock()
            .expect("candidate registry poisoned")
            .candidates
            .values()
            .cloned()
            .collect()
    }

    pub fn mark_trusted(&self, node_id: impl Into<String>) {
        self.inner
            .lock()
            .expect("candidate registry poisoned")
            .trusted_node_ids
            .insert(node_id.into());
    }

    pub fn is_trusted(&self, node_id: &str) -> bool {
        self.inner
            .lock()
            .expect("candidate registry poisoned")
            .trusted_node_ids
            .contains(node_id)
    }
}

pub struct BrowseHandle {
    inner: Box<dyn BrowseInner + Send + Sync>,
}

pub trait BrowseInner: Send + Sync {
    fn shutdown(&mut self);
}

impl BrowseHandle {
    pub fn new(inner: Box<dyn BrowseInner + Send + Sync>) -> Self {
        Self { inner }
    }
}

impl Drop for BrowseHandle {
    fn drop(&mut self) {
        self.inner.shutdown();
    }
}

pub trait Browser: Send + Sync {
    fn browse(
        &self,
        config: BrowseConfig,
        registry: CandidateRegistry,
    ) -> DiscoveryResult<BrowseHandle>;
}

pub fn start_browser_if_enabled(
    config: BrowseConfig,
    browser: &dyn Browser,
    registry: CandidateRegistry,
) -> DiscoveryResult<Option<BrowseHandle>> {
    if !config.enabled {
        return Ok(None);
    }
    browser.browse(config, registry).map(Some)
}

pub fn generate_a2a_node_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn advertise_a2a_txt_records(
    node_id: &str,
    agent_card_path: &str,
) -> DiscoveryResult<BTreeMap<String, String>> {
    if !valid_node_id(node_id) {
        return Err(DiscoveryError::Invalid("invalid A2A node ID".to_string()));
    }
    if !valid_agent_card_path(agent_card_path) {
        return Err(DiscoveryError::Invalid(
            "invalid A2A Agent Card path".to_string(),
        ));
    }

    Ok(BTreeMap::from([
        (A2A_NODE_ID_TXT.to_string(), node_id.to_string()),
        (A2A_VERSION_TXT.to_string(), A2A_VERSION.to_string()),
        (
            A2A_AGENT_CARD_PATH_TXT.to_string(),
            agent_card_path.to_string(),
        ),
    ]))
}

pub fn candidate_from_service(
    service: DiscoveredService,
    candidate_ttl: Duration,
) -> Option<CandidateRecord> {
    if !bounded(&service.full_name, 1, 255)
        || !bounded(&service.instance_name, 1, MAX_INSTANCE_NAME_BYTES)
        || service.addresses.is_empty()
        || service.addresses.len() > MAX_ADDRESSES
    {
        return None;
    }

    let node_id = required_txt(&service.txt, A2A_NODE_ID_TXT)?;
    let protocol_version = required_txt(&service.txt, A2A_VERSION_TXT)?;
    let agent_card_path = required_txt(&service.txt, A2A_AGENT_CARD_PATH_TXT)?;

    if !valid_node_id(node_id)
        || protocol_version != A2A_VERSION
        || !valid_agent_card_path(agent_card_path)
    {
        return None;
    }

    Some(CandidateRecord {
        node_id: node_id.to_string(),
        instance_name: service.instance_name,
        full_name: service.full_name,
        addresses: service.addresses,
        port: service.port,
        agent_card_path: agent_card_path.to_string(),
        protocol_version: protocol_version.to_string(),
        observed_at: service.observed_at,
        expires_at: service.observed_at + candidate_ttl,
    })
}

fn required_txt<'a>(txt: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    let value = txt.get(key)?;
    bounded(value, 1, MAX_TXT_VALUE_BYTES).then_some(value.as_str())
}

fn valid_node_id(value: &str) -> bool {
    bounded(value, 1, MAX_NODE_ID_BYTES)
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        && Uuid::parse_str(value).is_ok()
}

fn valid_agent_card_path(value: &str) -> bool {
    bounded(value, 1, MAX_AGENT_CARD_PATH_BYTES)
        && value.starts_with('/')
        && !value.starts_with("//")
        && !value.contains('?')
        && !value.contains('#')
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn bounded(value: &str, min: usize, max: usize) -> bool {
    let len = value.len();
    len >= min && len <= max && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn normalize_service_type(mut service_type: String) -> String {
    if !service_type.ends_with('.') {
        service_type.push('.');
    }
    service_type
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_unknown_fullname_is_noop() {
        let registry = CandidateRegistry::default();
        registry.apply_event(CandidateEvent::Remove {
            full_name: "missing._zbot._tcp.local.".to_string(),
        });
        assert!(registry.candidates().is_empty());
    }
}
