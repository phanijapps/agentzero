use crate::config::{
    DEFAULT_CREDENTIAL_LIFETIME_DAYS, DEFAULT_PEER_FILE, MAX_ACTIVE_INBOUND_CREDENTIALS,
    MAX_CREDENTIAL_LIFETIME_DAYS, MAX_TRUSTED_PEERS, PEER_STORE_VERSION,
};
use crate::secrets::{CredentialHash, SecretToken};
use crate::url_policy::{PeerOrigin, UrlPolicy, UrlPolicyError};
use chrono::{DateTime, Duration, Utc};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PeerStoreError {
    #[error("peer store IO failed: {0}")]
    Io(String),
    #[error("peer store JSON is invalid: {0}")]
    Json(String),
    #[error("peer store schema is unsupported")]
    UnsupportedVersion,
    #[error("peer store contains too many peers")]
    TooManyPeers,
    #[error("peer record is invalid")]
    InvalidPeer,
    #[error("credential lifetime exceeds maximum")]
    InvalidLifetime,
    #[error("peer already has two active inbound credentials")]
    TooManyActiveCredentials,
    #[error("peer was not found")]
    PeerNotFound,
    #[error("credential was not found")]
    CredentialNotFound,
    #[error("peer store file is unsafe")]
    UnsafeFile,
    #[error(transparent)]
    Url(#[from] UrlPolicyError),
}

#[derive(Debug, Clone)]
pub struct PeerStore {
    path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCredential {
    pub peer_id: String,
    pub display_name: Option<String>,
    pub target_agent_id: String,
    pub lifetime_days: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedCredential {
    pub peer_id: String,
    pub credential_id: String,
    pub expires_at: DateTime<Utc>,
    pub token: SecretToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddPeer {
    pub peer_id: String,
    pub display_name: String,
    pub origin: String,
    pub target_agent_id: String,
    pub outbound_token: Option<String>,
    pub allow_private_http: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerSnapshot {
    peers: BTreeMap<String, TrustedPeer>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PeerStoreFile {
    version: u32,
    peers: Vec<TrustedPeer>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedPeer {
    pub node_id: String,
    pub display_name: String,
    pub origin: Option<PeerOrigin>,
    pub target_agent_id: String,
    pub allow_private_http: bool,
    pub outbound_credential: Option<SecretToken>,
    pub inbound_credentials: Vec<InboundCredential>,
    pub created_at: DateTime<Utc>,
}

impl std::fmt::Debug for TrustedPeer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TrustedPeer")
            .field("node_id", &self.node_id)
            .field("display_name", &self.display_name)
            .field("origin", &self.origin)
            .field("target_agent_id", &self.target_agent_id)
            .field("allow_private_http", &self.allow_private_http)
            .field("outbound_credential", &"[REDACTED]")
            .field("inbound_credentials", &self.inbound_credentials)
            .field("created_at", &self.created_at)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InboundCredential {
    pub credential_id: String,
    pub credential_hash: CredentialHash,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for InboundCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InboundCredential")
            .field("credential_id", &self.credential_id)
            .field("credential_hash", &"[REDACTED]")
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .field("revoked_at", &self.revoked_at)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RedactedPeerSnapshot {
    pub peers: Vec<RedactedPeer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RedactedPeer {
    pub node_id: String,
    pub display_name: String,
    pub origin: Option<String>,
    pub target_agent_id: String,
    pub allow_private_http: bool,
    pub has_outbound_credential: bool,
    pub inbound_credentials: Vec<RedactedInboundCredential>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RedactedInboundCredential {
    pub credential_id: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl PeerStore {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            path: Self::path_for_data_dir(data_dir),
        }
    }

    pub fn path_for_data_dir(data_dir: impl AsRef<Path>) -> PathBuf {
        data_dir.as_ref().join("config").join(DEFAULT_PEER_FILE)
    }

    pub fn load_snapshot(&self) -> Result<PeerSnapshot, PeerStoreError> {
        let file = self.load_file()?;
        PeerSnapshot::from_file(file)
    }

    pub fn issue_credential(
        &self,
        request: IssueCredential,
    ) -> Result<IssuedCredential, PeerStoreError> {
        validate_peer_id(&request.peer_id)?;
        validate_id(&request.target_agent_id)?;
        let days = request
            .lifetime_days
            .unwrap_or(DEFAULT_CREDENTIAL_LIFETIME_DAYS);
        if !(1..=MAX_CREDENTIAL_LIFETIME_DAYS).contains(&days) {
            return Err(PeerStoreError::InvalidLifetime);
        }

        let mut file = self.load_file()?;
        let now = Utc::now();
        let peer = upsert_peer(&mut file, &request, now)?;
        let active = peer
            .inbound_credentials
            .iter()
            .filter(|cred| cred.active(now))
            .count();
        if active >= MAX_ACTIVE_INBOUND_CREDENTIALS {
            return Err(PeerStoreError::TooManyActiveCredentials);
        }

        let token = SecretToken::generate();
        let credential_id = random_id("cred");
        let expires_at = now + Duration::days(days);
        peer.inbound_credentials.push(InboundCredential {
            credential_id: credential_id.clone(),
            credential_hash: token.hash(),
            created_at: now,
            expires_at,
            revoked_at: None,
        });
        self.store_file(&file)?;

        Ok(IssuedCredential {
            peer_id: request.peer_id,
            credential_id,
            expires_at,
            token,
        })
    }

    pub fn add_peer(&self, request: AddPeer) -> Result<(), PeerStoreError> {
        validate_peer_id(&request.peer_id)?;
        validate_id(&request.target_agent_id)?;
        validate_bounded(&request.display_name, 1, 120)?;
        let origin = UrlPolicy::validate_peer_origin(&request.origin, request.allow_private_http)?;
        let mut file = self.load_file()?;
        if file.peers.len() >= MAX_TRUSTED_PEERS
            && !file
                .peers
                .iter()
                .any(|peer| peer.node_id == request.peer_id)
        {
            return Err(PeerStoreError::TooManyPeers);
        }
        let now = Utc::now();
        match file
            .peers
            .iter_mut()
            .find(|peer| peer.node_id == request.peer_id)
        {
            Some(peer) => {
                peer.display_name = request.display_name;
                peer.origin = Some(origin);
                peer.target_agent_id = request.target_agent_id;
                peer.allow_private_http = request.allow_private_http;
                peer.outbound_credential = request.outbound_token.map(SecretToken::from);
            }
            None => file.peers.push(TrustedPeer {
                node_id: request.peer_id,
                display_name: request.display_name,
                origin: Some(origin),
                target_agent_id: request.target_agent_id,
                allow_private_http: request.allow_private_http,
                outbound_credential: request.outbound_token.map(SecretToken::from),
                inbound_credentials: Vec::new(),
                created_at: now,
            }),
        }
        self.store_file(&file)
    }

    pub fn remove_peer(&self, peer_id: &str) -> Result<(), PeerStoreError> {
        let mut file = self.load_file()?;
        let before = file.peers.len();
        file.peers.retain(|peer| peer.node_id != peer_id);
        if file.peers.len() == before {
            return Err(PeerStoreError::PeerNotFound);
        }
        self.store_file(&file)
    }

    pub fn revoke_credential(
        &self,
        peer_id: &str,
        credential_id: &str,
    ) -> Result<(), PeerStoreError> {
        let mut file = self.load_file()?;
        let now = Utc::now();
        let peer = file
            .peers
            .iter_mut()
            .find(|peer| peer.node_id == peer_id)
            .ok_or(PeerStoreError::PeerNotFound)?;
        let credential = peer
            .inbound_credentials
            .iter_mut()
            .find(|cred| cred.credential_id == credential_id)
            .ok_or(PeerStoreError::CredentialNotFound)?;
        credential.revoked_at = Some(now);
        self.store_file(&file)
    }

    fn load_file(&self) -> Result<PeerStoreFile, PeerStoreError> {
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(PeerStoreError::UnsafeFile);
                }
                reject_unsafe_mode(&metadata)?;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(PeerStoreFile {
                    version: PEER_STORE_VERSION,
                    peers: Vec::new(),
                });
            }
            Err(error) => return Err(PeerStoreError::Io(error.to_string())),
        }

        let raw = fs::read_to_string(&self.path)
            .map_err(|error| PeerStoreError::Io(error.to_string()))?;
        let file: PeerStoreFile =
            serde_json::from_str(&raw).map_err(|error| PeerStoreError::Json(error.to_string()))?;
        validate_file(&file)?;
        Ok(file)
    }

    fn store_file(&self, file: &PeerStoreFile) -> Result<(), PeerStoreError> {
        validate_file(file)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| PeerStoreError::Io(error.to_string()))?;
        }
        let tmp = self
            .path
            .with_extension(format!("json.tmp-{}", random_id("write")));
        let raw = serde_json::to_vec_pretty(file)
            .map_err(|error| PeerStoreError::Json(error.to_string()))?;
        fs::write(&tmp, raw).map_err(|error| PeerStoreError::Io(error.to_string()))?;
        set_owner_only(&tmp)?;
        fs::rename(&tmp, &self.path).map_err(|error| PeerStoreError::Io(error.to_string()))?;
        set_owner_only(&self.path)
    }
}

impl PeerSnapshot {
    fn from_file(file: PeerStoreFile) -> Result<Self, PeerStoreError> {
        validate_file(&file)?;
        Ok(Self {
            peers: file
                .peers
                .into_iter()
                .map(|peer| (peer.node_id.clone(), peer))
                .collect(),
        })
    }

    pub fn get(&self, peer_id: &str) -> Option<&TrustedPeer> {
        self.peers.get(peer_id)
    }

    pub fn redacted(&self) -> RedactedPeerSnapshot {
        RedactedPeerSnapshot {
            peers: self.peers.values().map(TrustedPeer::redacted).collect(),
        }
    }

    pub fn verify_inbound_token(&self, peer_id: &str, token: &str) -> bool {
        let now = Utc::now();
        self.peers.get(peer_id).is_some_and(|peer| {
            peer.inbound_credentials
                .iter()
                .filter(|credential| credential.active(now))
                .any(|credential| credential.credential_hash.verify(token))
        })
    }

    /// Resolve a bearer credential to one peer while evaluating every active
    /// credential. Callers never accept a model- or request-supplied peer ID.
    pub fn authenticate_inbound_token(&self, token: &str) -> Option<&TrustedPeer> {
        let now = Utc::now();
        let mut matched = None;
        for peer in self.peers.values() {
            let peer_matches = peer
                .inbound_credentials
                .iter()
                .filter(|credential| credential.active(now))
                .fold(false, |found, credential| {
                    credential.credential_hash.verify(token) || found
                });
            if peer_matches {
                if matched.is_some() {
                    return None;
                }
                matched = Some(peer);
            }
        }
        matched
    }
}

impl TrustedPeer {
    fn redacted(&self) -> RedactedPeer {
        RedactedPeer {
            node_id: self.node_id.clone(),
            display_name: self.display_name.clone(),
            origin: self
                .origin
                .as_ref()
                .map(|origin| origin.origin().to_string()),
            target_agent_id: self.target_agent_id.clone(),
            allow_private_http: self.allow_private_http,
            has_outbound_credential: self.outbound_credential.is_some(),
            inbound_credentials: self
                .inbound_credentials
                .iter()
                .map(InboundCredential::redacted)
                .collect(),
            created_at: self.created_at,
        }
    }
}

impl InboundCredential {
    fn active(&self, now: DateTime<Utc>) -> bool {
        self.revoked_at.is_none() && self.expires_at > now
    }

    fn redacted(&self) -> RedactedInboundCredential {
        RedactedInboundCredential {
            credential_id: self.credential_id.clone(),
            created_at: self.created_at,
            expires_at: self.expires_at,
            revoked_at: self.revoked_at,
        }
    }
}

fn upsert_peer<'a>(
    file: &'a mut PeerStoreFile,
    request: &IssueCredential,
    now: DateTime<Utc>,
) -> Result<&'a mut TrustedPeer, PeerStoreError> {
    if let Some(index) = file
        .peers
        .iter()
        .position(|peer| peer.node_id == request.peer_id)
    {
        return Ok(&mut file.peers[index]);
    }
    if file.peers.len() >= MAX_TRUSTED_PEERS {
        return Err(PeerStoreError::TooManyPeers);
    }
    file.peers.push(TrustedPeer {
        node_id: request.peer_id.clone(),
        display_name: request
            .display_name
            .clone()
            .unwrap_or_else(|| request.peer_id.clone()),
        origin: None,
        target_agent_id: request.target_agent_id.clone(),
        allow_private_http: false,
        outbound_credential: None,
        inbound_credentials: Vec::new(),
        created_at: now,
    });
    Ok(file.peers.last_mut().expect("peer just pushed"))
}

fn validate_file(file: &PeerStoreFile) -> Result<(), PeerStoreError> {
    if file.version != PEER_STORE_VERSION {
        return Err(PeerStoreError::UnsupportedVersion);
    }
    if file.peers.len() > MAX_TRUSTED_PEERS {
        return Err(PeerStoreError::TooManyPeers);
    }
    for peer in &file.peers {
        validate_peer_id(&peer.node_id)?;
        validate_bounded(&peer.display_name, 1, 120)?;
        validate_id(&peer.target_agent_id)?;
        let active = peer
            .inbound_credentials
            .iter()
            .filter(|credential| credential.active(Utc::now()))
            .count();
        if active > MAX_ACTIVE_INBOUND_CREDENTIALS {
            return Err(PeerStoreError::TooManyActiveCredentials);
        }
        for credential in &peer.inbound_credentials {
            validate_id(&credential.credential_id)?;
            if credential.credential_hash.as_str().len() != 64 {
                return Err(PeerStoreError::InvalidPeer);
            }
        }
    }
    Ok(())
}

fn validate_id(value: &str) -> Result<(), PeerStoreError> {
    validate_bounded(value, 1, 128)?;
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':'))
    {
        Ok(())
    } else {
        Err(PeerStoreError::InvalidPeer)
    }
}

fn validate_peer_id(value: &str) -> Result<(), PeerStoreError> {
    validate_bounded(value, 1, 120)?;
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':'))
    {
        Ok(())
    } else {
        Err(PeerStoreError::InvalidPeer)
    }
}

fn validate_bounded(value: &str, min: usize, max: usize) -> Result<(), PeerStoreError> {
    let len = value.chars().count();
    if (min..=max).contains(&len) {
        Ok(())
    } else {
        Err(PeerStoreError::InvalidPeer)
    }
}

fn random_id(prefix: &str) -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    let mut out = String::with_capacity(prefix.len() + 33);
    out.push_str(prefix);
    out.push('-');
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(unix)]
fn reject_unsafe_mode(metadata: &fs::Metadata) -> Result<(), PeerStoreError> {
    use std::os::unix::fs::PermissionsExt;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(PeerStoreError::UnsafeFile);
    }
    Ok(())
}

#[cfg(not(unix))]
fn reject_unsafe_mode(_metadata: &fs::Metadata) -> Result<(), PeerStoreError> {
    Ok(())
}

#[cfg(unix)]
fn set_owner_only(path: &Path) -> Result<(), PeerStoreError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| PeerStoreError::Io(error.to_string()))
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) -> Result<(), PeerStoreError> {
    Ok(())
}
