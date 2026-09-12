//! Protocol-neutral registry for untrusted discovered A2A candidates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredPeerCandidate {
    pub node_id: String,
    pub instance_name: String,
    pub addresses: Vec<IpAddr>,
    pub port: u16,
    pub agent_card_path: String,
    pub protocol_version: String,
    pub observed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateRegistryEvent {
    Upsert(DiscoveredPeerCandidate),
    Remove { node_id: String },
}

#[derive(Debug, Default, Clone)]
pub struct CandidateRegistry {
    inner: Arc<RwLock<CandidateRegistryInner>>,
}

#[derive(Debug, Default)]
struct CandidateRegistryInner {
    candidates: BTreeMap<String, DiscoveredPeerCandidate>,
    trusted_peer_ids: BTreeSet<String>,
}

impl CandidateRegistry {
    pub fn apply(&self, event: CandidateRegistryEvent) {
        let mut inner = self.inner.write().expect("candidate registry poisoned");
        match event {
            CandidateRegistryEvent::Upsert(candidate) => {
                inner
                    .candidates
                    .insert(candidate.node_id.clone(), candidate);
            }
            CandidateRegistryEvent::Remove { node_id } => {
                inner.candidates.remove(&node_id);
            }
        }
    }

    pub fn expire(&self, now: DateTime<Utc>) -> Vec<String> {
        let mut inner = self.inner.write().expect("candidate registry poisoned");
        let expired: Vec<String> = inner
            .candidates
            .iter()
            .filter(|(_, candidate)| candidate.expires_at <= now)
            .map(|(node_id, _)| node_id.clone())
            .collect();
        for node_id in &expired {
            inner.candidates.remove(node_id);
        }
        expired
    }

    pub fn candidate(&self, node_id: &str) -> Option<DiscoveredPeerCandidate> {
        self.inner
            .read()
            .expect("candidate registry poisoned")
            .candidates
            .get(node_id)
            .cloned()
    }

    pub fn candidates(&self) -> Vec<DiscoveredPeerCandidate> {
        self.inner
            .read()
            .expect("candidate registry poisoned")
            .candidates
            .values()
            .cloned()
            .collect()
    }

    pub fn remember_trusted_peer(&self, node_id: impl Into<String>) {
        self.inner
            .write()
            .expect("candidate registry poisoned")
            .trusted_peer_ids
            .insert(node_id.into());
    }

    pub fn is_trusted_peer(&self, node_id: &str) -> bool {
        self.inner
            .read()
            .expect("candidate registry poisoned")
            .trusted_peer_ids
            .contains(node_id)
    }
}
