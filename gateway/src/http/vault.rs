//! `GET /api/vault/*` — read-only local ward filesystem browser.

use crate::config::GatewayConfig;
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{ConnectInfo, Path, Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    fs,
    net::SocketAddr,
    path::{Component, Path as FsPath, PathBuf},
    time::SystemTime,
};

const TEXT_READ_LIMIT: u64 = 2 * 1024 * 1024;
const OFFICE_READ_LIMIT: u64 = 15 * 1024 * 1024;
const DIRECTORY_CHILD_LIMIT: usize = 1_000;
const SEARCH_DEFAULT_LIMIT: usize = 30;
const SEARCH_MAX_LIMIT: usize = 50;
const SEARCH_SCAN_LIMIT: usize = 20_000;

const RESERVED_WARD_ROOT_ENTRIES: &[&str] = &["_archive", "_curator_backups"];
const EXCLUDED_DIRS: &[&str] = &[
    ".venv",
    "venv",
    "node_modules",
    "__pycache__",
    ".git",
    "target",
    "dist",
    "build",
    ".next",
    ".cache",
];
const EXCLUDED_CONFIG_FILES: &[&str] = &[
    "config.yaml",
    "config.yml",
    "settings.yaml",
    "settings.yml",
    "secrets.yaml",
    "secrets.yml",
    "credentials.yaml",
    "credentials.yml",
];
const VISIBLE_EXTENSIONS: &[&str] = &[
    "md", "txt", "yaml", "yml", "py", "js", "ts", "tsx", "html", "css", "json", "toml", "docx",
    "pptx", "doc", "ppt",
];
const READABLE_EXTENSIONS: &[&str] = &[
    "md", "txt", "yaml", "yml", "py", "js", "ts", "tsx", "html", "css", "json", "toml", "docx",
    "pptx",
];

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
}

type HandlerError = (StatusCode, Json<ErrorBody>);

#[derive(Debug, Serialize)]
pub struct VaultWard {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct VaultWardsResponse {
    pub wards: Vec<VaultWard>,
}

#[derive(Debug, Serialize)]
pub struct VaultTreeResponse {
    pub ward_id: String,
    pub path: String,
    pub children: Vec<VaultNode>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct VaultSearchResponse {
    pub ward_id: String,
    pub query: String,
    pub matches: Vec<VaultNode>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct VaultNode {
    pub ward_id: String,
    pub path: String,
    pub name: String,
    pub kind: VaultNodeKind,
    pub extension: Option<String>,
    pub size: Option<u64>,
    pub modified_at: Option<String>,
    pub previewable: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VaultNodeKind {
    Directory,
    File,
}

#[derive(Debug, Serialize)]
pub struct VaultTextFileResponse {
    pub ward_id: String,
    pub path: String,
    pub name: String,
    pub kind: &'static str,
    pub extension: String,
    pub size: u64,
    pub modified_at: Option<String>,
    pub content: String,
}

fn error(status: StatusCode, message: impl Into<String>) -> HandlerError {
    (
        status,
        Json(ErrorBody {
            error: message.into(),
        }),
    )
}

pub(super) fn is_local_request(config: &GatewayConfig, peer: Option<SocketAddr>) -> bool {
    if config.host.is_loopback() {
        return true;
    }
    peer.map(|addr| addr.ip().is_loopback()).unwrap_or(false)
}

fn require_local(config: &GatewayConfig, peer: Option<SocketAddr>) -> Result<(), HandlerError> {
    if is_local_request(config, peer) {
        Ok(())
    } else {
        Err(error(
            StatusCode::FORBIDDEN,
            "Vault filesystem access is available only from the local device",
        ))
    }
}

fn casefold(value: &str) -> String {
    value.to_ascii_lowercase()
}

fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

fn is_reserved_ward_root_entry(name: &str) -> bool {
    let folded = casefold(name);
    is_hidden_name(&folded)
        || folded.starts_with('_')
        || RESERVED_WARD_ROOT_ENTRIES.contains(&folded.as_str())
}

fn validate_ward_id(ward_id: &str) -> Result<(), HandlerError> {
    if ward_id.is_empty()
        || ward_id == "."
        || ward_id == ".."
        || ward_id.contains('/')
        || ward_id.contains('\\')
    {
        return Err(error(StatusCode::BAD_REQUEST, "Invalid ward id"));
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<PathBuf, HandlerError> {
    if path.is_empty() {
        return Ok(PathBuf::new());
    }
    let candidate = FsPath::new(path);
    if candidate.is_absolute() {
        return Err(error(StatusCode::BAD_REQUEST, "Invalid path"));
    }
    let mut clean = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(error(StatusCode::BAD_REQUEST, "Invalid path"));
            }
        }
    }
    Ok(clean)
}

fn relative_path_string(path: &FsPath) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn component_names(path: &FsPath) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(casefold(&part.to_string_lossy())),
            _ => None,
        })
        .collect()
}

fn extension_for(path: &FsPath) -> Option<String> {
    path.extension()
        .and_then(|v| v.to_str())
        .map(casefold)
        .filter(|v| !v.is_empty())
}

fn file_name_for(path: &FsPath) -> Option<String> {
    path.file_name().map(|v| v.to_string_lossy().to_string())
}

fn is_env_file(name: &str) -> bool {
    name == ".env" || name.starts_with(".env.") || name.ends_with(".env")
}

fn is_excluded_path(relative: &FsPath) -> bool {
    let components = component_names(relative);
    if components.iter().any(|name| is_hidden_name(name)) {
        return true;
    }
    if components
        .iter()
        .any(|name| EXCLUDED_DIRS.contains(&name.as_str()))
    {
        return true;
    }
    let Some(name) = components.last() else {
        return false;
    };
    is_env_file(name) || EXCLUDED_CONFIG_FILES.contains(&name.as_str())
}

fn is_visible_file(relative: &FsPath) -> bool {
    extension_for(relative)
        .map(|ext| VISIBLE_EXTENSIONS.contains(&ext.as_str()))
        .unwrap_or(false)
}

fn is_readable_file(relative: &FsPath) -> bool {
    extension_for(relative)
        .map(|ext| READABLE_EXTENSIONS.contains(&ext.as_str()))
        .unwrap_or(false)
}

fn is_office_open_xml(relative: &FsPath) -> bool {
    matches!(extension_for(relative).as_deref(), Some("docx" | "pptx"))
}

fn is_previewable(relative: &FsPath) -> bool {
    is_readable_file(relative)
}

fn canonical_child(base: &FsPath, child: &FsPath) -> Result<PathBuf, HandlerError> {
    let canonical = child
        .canonicalize()
        .map_err(|_| error(StatusCode::NOT_FOUND, "Path not found"))?;
    if !canonical.starts_with(base) {
        return Err(error(StatusCode::BAD_REQUEST, "Path escapes ward root"));
    }
    Ok(canonical)
}

fn has_symlink_component(base: &FsPath, relative: &FsPath) -> bool {
    let mut current = base.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        current.push(part);
        if current
            .symlink_metadata()
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

fn resolve_ward_root(state: &AppState, ward_id: &str) -> Result<(PathBuf, PathBuf), HandlerError> {
    validate_ward_id(ward_id)?;
    if is_reserved_ward_root_entry(ward_id) {
        return Err(error(StatusCode::FORBIDDEN, "Ward is not available"));
    }
    let wards_dir = state
        .paths
        .wards_dir()
        .canonicalize()
        .map_err(|_| error(StatusCode::NOT_FOUND, "Wards directory not found"))?;
    let root = state.paths.wards_dir().join(ward_id);
    if root
        .symlink_metadata()
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(error(StatusCode::FORBIDDEN, "Ward is not available"));
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|_| error(StatusCode::NOT_FOUND, "Ward not found"))?;
    if !canonical_root.starts_with(&wards_dir)
        || canonical_root.parent() != Some(wards_dir.as_path())
    {
        return Err(error(StatusCode::BAD_REQUEST, "Invalid ward root"));
    }
    if !canonical_root.is_dir() {
        return Err(error(StatusCode::NOT_FOUND, "Ward not found"));
    }
    Ok((wards_dir, canonical_root))
}

fn resolve_relative(
    ward_root: &FsPath,
    relative: &str,
    require_dir: bool,
) -> Result<(PathBuf, PathBuf), HandlerError> {
    let clean = validate_relative_path(relative)?;
    if is_excluded_path(&clean) {
        return Err(error(StatusCode::FORBIDDEN, "Path is excluded"));
    }
    if has_symlink_component(ward_root, &clean) {
        return Err(error(
            StatusCode::FORBIDDEN,
            "Symlink paths are not available",
        ));
    }
    let candidate = ward_root.join(&clean);
    let canonical = canonical_child(ward_root, &candidate)?;
    if require_dir && !canonical.is_dir() {
        return Err(error(StatusCode::NOT_FOUND, "Directory not found"));
    }
    if !require_dir && !canonical.is_file() {
        return Err(error(StatusCode::NOT_FOUND, "File not found"));
    }
    Ok((clean, canonical))
}

fn modified_at(metadata: &fs::Metadata) -> Option<String> {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|duration| DateTime::<Utc>::from(SystemTime::UNIX_EPOCH + duration).to_rfc3339())
}

fn node_from_path(ward_id: &str, ward_root: &FsPath, path: PathBuf) -> Option<VaultNode> {
    let metadata = path.metadata().ok()?;
    let relative = path.strip_prefix(ward_root).ok()?.to_path_buf();
    let name = file_name_for(&relative)?;
    let is_dir = metadata.is_dir();
    Some(VaultNode {
        ward_id: ward_id.to_string(),
        path: relative_path_string(&relative),
        name,
        kind: if is_dir {
            VaultNodeKind::Directory
        } else {
            VaultNodeKind::File
        },
        extension: if is_dir {
            None
        } else {
            extension_for(&relative)
        },
        size: if is_dir { None } else { Some(metadata.len()) },
        modified_at: modified_at(&metadata),
        previewable: !is_dir && is_previewable(&relative),
    })
}

fn query_path(query: &HashMap<String, String>) -> &str {
    query.get("path").map(String::as_str).unwrap_or("")
}

fn query_limit(query: &HashMap<String, String>) -> usize {
    query
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(SEARCH_DEFAULT_LIMIT)
        .min(SEARCH_MAX_LIMIT)
}

fn normalize_search(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn fuzzy_score(query: &str, candidate: &str, name: &str) -> Option<i64> {
    let query = normalize_search(query);
    if query.is_empty() {
        return None;
    }
    let candidate = candidate.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    let mut score = 0_i64;
    let mut last_index: Option<usize> = None;
    let mut search_from = 0_usize;

    for query_char in query.chars() {
        let found = candidate[search_from..]
            .char_indices()
            .find(|(_, candidate_char)| *candidate_char == query_char)
            .map(|(offset, _)| search_from + offset)?;
        score += 10;
        if last_index.map(|last| found == last + 1).unwrap_or(false) {
            score += 7;
        }
        last_index = Some(found);
        search_from = found + query_char.len_utf8();
    }

    if candidate.contains(&query) {
        score += 40;
    }
    if name.contains(&query) {
        score += 30;
    }
    if name.starts_with(&query) {
        score += 20;
    }
    score -= candidate.len().min(200) as i64 / 8;
    Some(score)
}

pub async fn list_vault_wards(
    State(state): State<AppState>,
    Extension(config): Extension<GatewayConfig>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
) -> Result<Json<VaultWardsResponse>, HandlerError> {
    require_local(&config, connect_info.map(|info| info.0))?;
    let wards_dir = state.paths.wards_dir();
    let mut wards = Vec::new();
    let entries = fs::read_dir(&wards_dir)
        .map_err(|_| error(StatusCode::NOT_FOUND, "Wards directory not found"))?;
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if is_reserved_ward_root_entry(&name) {
            continue;
        }
        wards.push(VaultWard {
            id: name.clone(),
            name,
        });
    }
    wards.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(VaultWardsResponse { wards }))
}

pub async fn get_vault_tree(
    State(state): State<AppState>,
    Extension(config): Extension<GatewayConfig>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    Path(ward_id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<VaultTreeResponse>, HandlerError> {
    require_local(&config, connect_info.map(|info| info.0))?;
    let (_wards_dir, ward_root) = resolve_ward_root(&state, &ward_id)?;
    let relative = query_path(&query);
    let (clean, dir) = resolve_relative(&ward_root, relative, true)?;
    let mut children = Vec::new();
    let mut truncated = false;
    let entries =
        fs::read_dir(&dir).map_err(|_| error(StatusCode::NOT_FOUND, "Directory not found"))?;
    for entry_result in entries {
        let Ok(entry) = entry_result else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let child_path = entry.path();
        let Ok(child_canonical) = child_path.canonicalize() else {
            continue;
        };
        if !child_canonical.starts_with(&ward_root) {
            continue;
        }
        let Ok(relative_child) = child_canonical.strip_prefix(&ward_root) else {
            continue;
        };
        if is_excluded_path(relative_child) {
            continue;
        }
        if file_type.is_file() && !is_visible_file(relative_child) {
            continue;
        }
        if children.len() >= DIRECTORY_CHILD_LIMIT {
            truncated = true;
            break;
        }
        if let Some(node) = node_from_path(&ward_id, &ward_root, child_canonical) {
            children.push(node);
        }
    }

    children.sort_by(|a, b| match (&a.kind, &b.kind) {
        (VaultNodeKind::Directory, VaultNodeKind::File) => std::cmp::Ordering::Less,
        (VaultNodeKind::File, VaultNodeKind::Directory) => std::cmp::Ordering::Greater,
        _ => a
            .name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase()),
    });

    Ok(Json(VaultTreeResponse {
        ward_id,
        path: relative_path_string(&clean),
        children,
        truncated,
    }))
}

pub async fn search_vault_files(
    State(state): State<AppState>,
    Extension(config): Extension<GatewayConfig>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    Path(ward_id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<VaultSearchResponse>, HandlerError> {
    require_local(&config, connect_info.map(|info| info.0))?;
    let (_wards_dir, ward_root) = resolve_ward_root(&state, &ward_id)?;
    let query_text = query.get("q").map(String::as_str).unwrap_or("").trim();
    let limit = query_limit(&query);
    if normalize_search(query_text).is_empty() {
        return Ok(Json(VaultSearchResponse {
            ward_id,
            query: query_text.to_string(),
            matches: Vec::new(),
            truncated: false,
        }));
    }

    let mut scanned = 0_usize;
    let mut truncated = false;
    let mut scored: Vec<(i64, VaultNode)> = Vec::new();
    let mut queue = VecDeque::from([ward_root.clone()]);

    while let Some(dir) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry_result in entries {
            scanned += 1;
            if scanned > SEARCH_SCAN_LIMIT {
                truncated = true;
                break;
            }

            let Ok(entry) = entry_result else {
                continue;
            };
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            let child_path = entry.path();
            let Ok(child_canonical) = child_path.canonicalize() else {
                continue;
            };
            if !child_canonical.starts_with(&ward_root) {
                continue;
            }
            let Ok(relative_child) = child_canonical.strip_prefix(&ward_root) else {
                continue;
            };
            if is_excluded_path(relative_child) {
                continue;
            }

            if file_type.is_dir() {
                queue.push_back(child_canonical);
                continue;
            }
            if !file_type.is_file() || !is_visible_file(relative_child) {
                continue;
            }

            let Some(node) = node_from_path(&ward_id, &ward_root, child_canonical) else {
                continue;
            };
            if let Some(score) = fuzzy_score(query_text, &node.path, &node.name) {
                scored.push((score, node));
            }
        }
        if truncated {
            break;
        }
    }

    scored.sort_by(|(score_a, node_a), (score_b, node_b)| {
        score_b.cmp(score_a).then_with(|| {
            node_a
                .path
                .to_ascii_lowercase()
                .cmp(&node_b.path.to_ascii_lowercase())
        })
    });
    if scored.len() > limit {
        truncated = true;
    }
    let matches = scored
        .into_iter()
        .take(limit)
        .map(|(_, node)| node)
        .collect();

    Ok(Json(VaultSearchResponse {
        ward_id,
        query: query_text.to_string(),
        matches,
        truncated,
    }))
}

pub async fn get_vault_file(
    State(state): State<AppState>,
    Extension(config): Extension<GatewayConfig>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    Path(ward_id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response, HandlerError> {
    require_local(&config, connect_info.map(|info| info.0))?;
    let (_wards_dir, ward_root) = resolve_ward_root(&state, &ward_id)?;
    let relative_param = query_path(&query);
    let (relative, file) = resolve_relative(&ward_root, relative_param, false)?;
    if !is_readable_file(&relative) {
        return Err(error(StatusCode::FORBIDDEN, "File type is not readable"));
    }
    let metadata = file
        .metadata()
        .map_err(|_| error(StatusCode::NOT_FOUND, "File not found"))?;
    let limit = if is_office_open_xml(&relative) {
        OFFICE_READ_LIMIT
    } else {
        TEXT_READ_LIMIT
    };
    if metadata.len() > limit {
        return Err(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "File is too large to preview",
        ));
    }

    if is_office_open_xml(&relative) {
        let bytes =
            fs::read(&file).map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let content_type = match extension_for(&relative).as_deref() {
            Some("docx") => {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            }
            Some("pptx") => {
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            }
            _ => "application/octet-stream",
        };
        let mut response = Body::from(bytes).into_response();
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
        response.headers_mut().insert(
            "x-vault-ward-id",
            HeaderValue::from_str(&ward_id).unwrap_or_else(|_| HeaderValue::from_static("")),
        );
        response.headers_mut().insert(
            "x-vault-path",
            HeaderValue::from_str(&relative_path_string(&relative))
                .unwrap_or_else(|_| HeaderValue::from_static("")),
        );
        return Ok(response);
    }

    let content = fs::read_to_string(&file)
        .map_err(|_| error(StatusCode::BAD_REQUEST, "File is not valid UTF-8 text"))?;
    let name = file_name_for(&relative).unwrap_or_else(|| relative_path_string(&relative));
    let body = VaultTextFileResponse {
        ward_id,
        path: relative_path_string(&relative),
        name,
        kind: "text",
        extension: extension_for(&relative).unwrap_or_default(),
        size: metadata.len(),
        modified_at: modified_at(&metadata),
        content,
    };
    Ok(Json(body).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn excludes_are_case_insensitive() {
        assert!(is_excluded_path(FsPath::new(".ENV")));
        assert!(is_excluded_path(FsPath::new("Secrets.yaml")));
        assert!(is_excluded_path(FsPath::new("CONFIG.YML")));
        assert!(is_excluded_path(FsPath::new(".Venv/package.py")));
    }

    #[test]
    fn direct_read_allowlist_is_narrower_than_visibility_allowlist() {
        assert!(is_visible_file(FsPath::new("slides.ppt")));
        assert!(!is_readable_file(FsPath::new("slides.ppt")));
        assert!(is_visible_file(FsPath::new("deck.pptx")));
        assert!(is_readable_file(FsPath::new("deck.pptx")));
    }

    #[test]
    fn invalid_ward_ids_are_rejected() {
        assert!(validate_ward_id("../secret").is_err());
        assert!(validate_ward_id("foo/bar").is_err());
        assert!(validate_ward_id("foo\\bar").is_err());
        assert!(validate_ward_id(".").is_err());
    }

    #[test]
    fn relative_path_rejects_escape() {
        assert!(validate_relative_path("../secret.txt").is_err());
        assert!(validate_relative_path("/tmp/secret.txt").is_err());
        assert_eq!(
            validate_relative_path("./reports/valuation.md").unwrap(),
            PathBuf::from("reports/valuation.md")
        );
    }

    #[test]
    fn local_access_allows_loopback_bind_without_peer() {
        let cfg = GatewayConfig::default();
        assert!(is_local_request(&cfg, None));
    }

    #[test]
    fn local_access_denies_unspecified_bind_without_peer() {
        let cfg = GatewayConfig {
            host: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            ..GatewayConfig::default()
        };
        assert!(!is_local_request(&cfg, None));
    }

    #[test]
    fn local_access_on_unspecified_bind_requires_loopback_peer() {
        let cfg = GatewayConfig {
            host: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            ..GatewayConfig::default()
        };
        assert!(is_local_request(
            &cfg,
            Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 1234)))
        ));
        assert!(!is_local_request(
            &cfg,
            Some(SocketAddr::from((Ipv4Addr::new(192, 168, 1, 20), 1234)))
        ));
    }
}
