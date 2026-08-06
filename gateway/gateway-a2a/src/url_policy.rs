use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UrlPolicyError {
    #[error("invalid peer URL")]
    InvalidUrl,
    #[error("peer URL must use HTTPS or explicitly allowed private HTTP")]
    DisallowedScheme,
    #[error("peer URL must not contain userinfo, query, or fragment")]
    MutableUrlComponents,
    #[error("peer URL resolves to a prohibited destination")]
    ProhibitedDestination,
    #[error("peer URL resolution changed after trust")]
    DnsRebinding,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerOrigin {
    origin: String,
    scheme: String,
    host: String,
    port: Option<u16>,
    allow_private_http: bool,
}

impl std::fmt::Debug for PeerOrigin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PeerOrigin")
            .field("origin", &self.origin)
            .field("allow_private_http", &self.allow_private_http)
            .finish()
    }
}

impl PeerOrigin {
    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn validate_resolution(
        &self,
        ips: impl IntoIterator<Item = IpAddr>,
    ) -> Result<ResolvedPeerEndpoint, UrlPolicyError> {
        ResolvedPeerEndpoint::new(self.clone(), ips)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPeerEndpoint {
    endpoint: PeerOrigin,
    ips: BTreeSet<IpAddr>,
}

impl ResolvedPeerEndpoint {
    pub fn new(
        endpoint: PeerOrigin,
        ips: impl IntoIterator<Item = IpAddr>,
    ) -> Result<Self, UrlPolicyError> {
        let ips: BTreeSet<_> = ips.into_iter().collect();
        if ips.is_empty() || ips.iter().any(|ip| !endpoint.allows_resolved_ip(*ip)) {
            return Err(UrlPolicyError::ProhibitedDestination);
        }
        Ok(Self { endpoint, ips })
    }

    pub fn reject_rebind(
        &self,
        ips: impl IntoIterator<Item = IpAddr>,
    ) -> Result<(), UrlPolicyError> {
        let next = Self::new(self.endpoint.clone(), ips)?;
        if next.ips != self.ips {
            return Err(UrlPolicyError::DnsRebinding);
        }
        Ok(())
    }
}

impl PeerOrigin {
    fn allows_resolved_ip(&self, ip: IpAddr) -> bool {
        match self.host.parse::<IpAddr>() {
            Ok(configured) => ip == configured && (!prohibited_ip(ip) || ip.is_loopback()),
            Err(_) => !prohibited_ip(ip),
        }
    }
}

pub struct UrlPolicy;

impl UrlPolicy {
    pub fn validate_peer_origin(
        raw: &str,
        allow_private_http: bool,
    ) -> Result<PeerOrigin, UrlPolicyError> {
        let url = Url::parse(raw).map_err(|_| UrlPolicyError::InvalidUrl)?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "/" | "/a2a" | "/a2a/")
        {
            return Err(UrlPolicyError::MutableUrlComponents);
        }

        let host = url.host_str().ok_or(UrlPolicyError::InvalidUrl)?;
        let normalized_host = host
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .unwrap_or(host)
            .to_ascii_lowercase();
        let scheme = url.scheme();
        match scheme {
            "https" => {}
            "http" => {
                let ip = normalized_host.parse::<IpAddr>().ok();
                let is_loopback = ip.is_some_and(|ip| ip.is_loopback());
                let is_private = ip.is_some_and(private_ip);
                let private_http_allowed = allow_private_http && is_private;
                if !(is_loopback || private_http_allowed) {
                    return Err(UrlPolicyError::DisallowedScheme);
                }
            }
            _ => return Err(UrlPolicyError::DisallowedScheme),
        }

        let origin_host = if normalized_host.parse::<Ipv6Addr>().is_ok() {
            format!("[{normalized_host}]")
        } else {
            normalized_host.clone()
        };
        let mut origin = format!("{}://{}", scheme, origin_host);
        if let Some(port) = url.port() {
            origin.push(':');
            origin.push_str(&port.to_string());
        }

        Ok(PeerOrigin {
            origin,
            scheme: scheme.to_string(),
            host: normalized_host,
            port: url.port(),
            allow_private_http,
        })
    }
}

fn private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback() || unique_local_v6(ip),
    }
}

fn prohibited_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_loopback()
                || ip.is_link_local()
                || ip.octets() == [169, 254, 169, 254]
                || ip == Ipv4Addr::new(0, 0, 0, 0)
                || ip.is_broadcast()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unspecified() || link_local_v6(ip),
    }
}

fn unique_local_v6(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xfe00) == 0xfc00
}

fn link_local_v6(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
}
