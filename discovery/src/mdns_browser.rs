//! Real mDNS browser backed by the pure-Rust `mdns-sd` crate.

use crate::browser::{
    candidate_from_service, BrowseConfig, BrowseHandle, BrowseInner, Browser, CandidateEvent,
    CandidateRegistry, DiscoveredService,
};
use crate::{DiscoveryError, DiscoveryResult};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

pub struct MdnsBrowser {
    daemon: ServiceDaemon,
    started: AtomicBool,
}

impl MdnsBrowser {
    pub fn new() -> DiscoveryResult<Self> {
        let daemon =
            ServiceDaemon::new().map_err(|e| DiscoveryError::ResponderStart(e.to_string()))?;
        Ok(Self {
            daemon,
            started: AtomicBool::new(false),
        })
    }
}

impl Browser for MdnsBrowser {
    fn browse(
        &self,
        config: BrowseConfig,
        registry: CandidateRegistry,
    ) -> DiscoveryResult<BrowseHandle> {
        if !config.enabled {
            return Ok(BrowseHandle::new(Box::new(NoopBrowseInner)));
        }

        let already_started = self.started.swap(true, Ordering::SeqCst);
        debug_assert!(
            !already_started,
            "MdnsBrowser::browse called more than once; construct a fresh browser per lifecycle"
        );
        if already_started {
            return Err(DiscoveryError::Invalid(
                "mDNS browser already started".to_string(),
            ));
        }

        let receiver = self
            .daemon
            .browse(&config.service_type)
            .map_err(|e| DiscoveryError::ResponderStart(e.to_string()))?;
        let daemon = self.daemon.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let service_type = config.service_type.clone();
        let candidate_ttl = config.candidate_ttl;
        let thread = std::thread::spawn(move || {
            while !stop_thread.load(Ordering::SeqCst) {
                match receiver.recv_timeout(Duration::from_millis(200)) {
                    Ok(ServiceEvent::ServiceResolved(info)) => {
                        let service = DiscoveredService {
                            full_name: info.get_fullname().to_string(),
                            instance_name: instance_name(info.get_fullname(), &service_type),
                            addresses: info.get_addresses().iter().copied().collect(),
                            port: info.get_port(),
                            txt: txt_records(&info),
                            observed_at: Instant::now(),
                        };
                        if let Some(candidate) = candidate_from_service(service, candidate_ttl) {
                            debug!(
                                target: "discovery",
                                node_id = %candidate.node_id,
                                "observed untrusted A2A candidate"
                            );
                            registry.apply_event(CandidateEvent::Upsert(candidate));
                        } else {
                            debug!(
                                target: "discovery",
                                fullname = %info.get_fullname(),
                                "dropping malformed A2A discovery record"
                            );
                        }
                    }
                    Ok(ServiceEvent::ServiceRemoved(_, full_name)) => {
                        registry.apply_event(CandidateEvent::Remove { full_name });
                    }
                    Ok(ServiceEvent::SearchStarted(_))
                    | Ok(ServiceEvent::ServiceFound(_, _))
                    | Ok(ServiceEvent::SearchStopped(_)) => {}
                    Err(_) => {
                        registry.expire(Instant::now());
                    }
                }
            }
        });

        info!(
            target: "discovery",
            service_type = %config.service_type,
            "browsing for untrusted A2A candidates"
        );

        Ok(BrowseHandle::new(Box::new(MdnsBrowseInner {
            daemon,
            stop,
            thread: Some(thread),
        })))
    }
}

struct MdnsBrowseInner {
    daemon: ServiceDaemon,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl BrowseInner for MdnsBrowseInner {
    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                warn!(target: "discovery", "mDNS browser thread panicked during shutdown");
            }
        }
        if let Err(error) = self.daemon.shutdown() {
            warn!(target: "discovery", "mDNS browser shutdown failed: {}", error);
        }
        info!(target: "discovery", "stopped browsing for A2A candidates");
    }
}

struct NoopBrowseInner;

impl BrowseInner for NoopBrowseInner {
    fn shutdown(&mut self) {}
}

fn txt_records(info: &mdns_sd::ServiceInfo) -> BTreeMap<String, String> {
    info.get_properties()
        .iter()
        .map(|property| (property.key().to_string(), property.val_str().to_string()))
        .collect()
}

fn instance_name(full_name: &str, service_type: &str) -> String {
    full_name
        .strip_suffix(service_type)
        .and_then(|value| value.strip_suffix('.'))
        .unwrap_or(full_name)
        .to_string()
}
