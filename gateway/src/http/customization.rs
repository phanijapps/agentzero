//! GET / PUT endpoints for editing canonical agent markdown under `<vault>/config/`.
//! Used by the Settings → Customization UI tab.

use crate::state::AppState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Validate a relative path supplied by the UI.
///
/// Allowed shapes (server only ever reads/writes files matching these):
/// - `agent/<file>.md`         → exact-case agent contract or chat instructions
/// - `agent-prompts/<file>.md` → lowercase-kebab prompt
///
/// Rejected:
/// - empty
/// - absolute path (`/...` or `\...`)
/// - parent traversal (`..`)
/// - non-`.md` files
/// - any nested path beyond the two canonical directories
pub(crate) fn validate_customization_path(p: &str) -> Result<PathBuf, &'static str> {
    if p.is_empty() || p.starts_with('/') || p.starts_with('\\') {
        return Err("invalid path");
    }
    if p.contains("..") {
        return Err("invalid path");
    }
    if !p.ends_with(".md") {
        return Err("only markdown files allowed");
    }
    let parts: Vec<&str> = p.split('/').collect();
    match parts.as_slice() {
        ["agent", file]
            if matches!(
                *file,
                "SOUL.md" | "INSTRUCTIONS.md" | "OS.md" | "chat-instructions.md"
            ) =>
        {
            Ok(PathBuf::from(p))
        }
        ["agent-prompts", file] if is_canonical_prompt_filename(file) => Ok(PathBuf::from(p)),
        _ => Err("invalid path"),
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub path: String,
    pub kind: FileKind,
    pub size: u64,
    pub modified_at: String,
    pub auto_generated: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Agent,
    Prompt,
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<FileEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

const AUTO_GENERATED_NAMES: &[&str] = &["OS.md"];

/// Walk canonical agent contracts and prompts under `config_dir/`.
pub(crate) fn enumerate_customization_files(config_dir: &Path) -> std::io::Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    let agent_dir = config_dir.join("agent");
    if agent_dir.is_dir() {
        push_md_files(&agent_dir, FileKind::Agent, "agent/", &mut entries)?;
    }
    let prompts_dir = config_dir.join("agent-prompts");
    if prompts_dir.is_dir() {
        push_md_files(
            &prompts_dir,
            FileKind::Prompt,
            "agent-prompts/",
            &mut entries,
        )?;
    }
    entries.sort_by(|a, b| {
        a.kind_order()
            .cmp(&b.kind_order())
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(entries)
}

fn push_md_files(
    dir: &Path,
    kind: FileKind,
    prefix: &str,
    out: &mut Vec<FileEntry>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.ends_with(".md") => n.to_string(),
            _ => continue,
        };
        let metadata = entry.metadata()?;
        let modified_at: DateTime<Utc> = metadata.modified()?.into();
        let auto_generated = AUTO_GENERATED_NAMES.contains(&name.as_str());
        out.push(FileEntry {
            path: format!("{}{}", prefix, name),
            kind,
            size: metadata.len(),
            modified_at: modified_at.to_rfc3339(),
            auto_generated,
        });
    }
    Ok(())
}

impl FileEntry {
    fn kind_order(&self) -> u8 {
        match self.kind {
            FileKind::Agent => 0,
            FileKind::Prompt => 1,
        }
    }
}

fn is_canonical_prompt_filename(filename: &str) -> bool {
    let Some(stem) = filename.strip_suffix(".md") else {
        return false;
    };
    !stem.is_empty()
        && !stem.starts_with('-')
        && !stem.ends_with('-')
        && stem
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// `GET /api/customization/files` — list editable markdowns.
pub async fn list_files(State(state): State<AppState>) -> (StatusCode, Json<ListResponse>) {
    let config_dir = state.paths.config_dir();
    match enumerate_customization_files(&config_dir) {
        Ok(files) => (
            StatusCode::OK,
            Json(ListResponse {
                success: true,
                files: Some(files),
                error: None,
            }),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ListResponse {
                success: false,
                files: None,
                error: Some(e.to_string()),
            }),
        ),
    }
}

const MAX_CONTENT_BYTES: usize = 1_000_000; // 1 MB cap

#[derive(Debug, Deserialize)]
pub struct PathQuery {
    pub path: String,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_generated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Populated only on 409 conflict
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveRequest {
    pub path: String,
    pub content: String,
    pub expected_version: String,
}

#[derive(Debug)]
pub(crate) enum SaveOutcome {
    Ok(String /* new version */),
    Conflict {
        current_content: String,
        current_version: String,
    },
    NotFound,
    Io(String),
}

pub(crate) fn resolve_path(config_dir: &Path, p: &str) -> Result<std::path::PathBuf, &'static str> {
    let validated = validate_customization_path(p)?;
    Ok(config_dir.join(validated))
}

pub(crate) fn file_version(path: &Path) -> std::io::Result<String> {
    let mtime: DateTime<Utc> = std::fs::metadata(path)?.modified()?.into();
    Ok(mtime.to_rfc3339())
}

pub(crate) fn save_file_with_check(
    config_dir: &Path,
    rel_path: &str,
    new_content: &str,
    expected_version: &str,
) -> SaveOutcome {
    let resolved = match resolve_path(config_dir, rel_path) {
        Ok(p) => p,
        Err(e) => return SaveOutcome::Io(format!("invalid path: {}", e)),
    };
    if !resolved.exists() {
        return SaveOutcome::NotFound;
    }
    let current_version = match file_version(&resolved) {
        Ok(v) => v,
        Err(e) => return SaveOutcome::Io(e.to_string()),
    };
    if current_version != expected_version {
        let current_content = match std::fs::read_to_string(&resolved) {
            Ok(c) => c,
            Err(e) => return SaveOutcome::Io(e.to_string()),
        };
        return SaveOutcome::Conflict {
            current_content,
            current_version,
        };
    }
    if let Err(e) = std::fs::write(&resolved, new_content) {
        return SaveOutcome::Io(e.to_string());
    }
    match file_version(&resolved) {
        Ok(v) => SaveOutcome::Ok(v),
        Err(e) => SaveOutcome::Io(e.to_string()),
    }
}

/// `GET /api/customization/file?path=<relative>`
pub async fn get_file(
    State(state): State<AppState>,
    Query(q): Query<PathQuery>,
) -> (StatusCode, Json<FileResponse>) {
    let config_dir = state.paths.config_dir();
    let resolved = match resolve_path(&config_dir, &q.path) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(FileResponse {
                    error: Some(e.to_string()),
                    ..Default::default()
                }),
            );
        }
    };
    if !resolved.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(FileResponse {
                error: Some("file not found".to_string()),
                ..Default::default()
            }),
        );
    }
    let content = match std::fs::read_to_string(&resolved) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(FileResponse {
                    error: Some(e.to_string()),
                    ..Default::default()
                }),
            );
        }
    };
    let version = file_version(&resolved).unwrap_or_default();
    let name = std::path::Path::new(&q.path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let auto_generated = AUTO_GENERATED_NAMES.contains(&name);
    (
        StatusCode::OK,
        Json(FileResponse {
            success: true,
            path: Some(q.path.clone()),
            content: Some(content),
            version: Some(version),
            auto_generated: Some(auto_generated),
            ..Default::default()
        }),
    )
}

/// `PUT /api/customization/file`
pub async fn put_file(
    State(state): State<AppState>,
    Json(req): Json<SaveRequest>,
) -> (StatusCode, Json<FileResponse>) {
    if req.content.len() > MAX_CONTENT_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(FileResponse {
                error: Some(format!(
                    "content too large ({} bytes > {} limit)",
                    req.content.len(),
                    MAX_CONTENT_BYTES
                )),
                ..Default::default()
            }),
        );
    }
    let config_dir = state.paths.config_dir();
    match save_file_with_check(&config_dir, &req.path, &req.content, &req.expected_version) {
        SaveOutcome::Ok(version) => (
            StatusCode::OK,
            Json(FileResponse {
                success: true,
                path: Some(req.path.clone()),
                version: Some(version),
                ..Default::default()
            }),
        ),
        SaveOutcome::Conflict {
            current_content,
            current_version,
        } => (
            StatusCode::CONFLICT,
            Json(FileResponse {
                error: Some("version mismatch".to_string()),
                current_content: Some(current_content),
                current_version: Some(current_version),
                ..Default::default()
            }),
        ),
        SaveOutcome::NotFound => (
            StatusCode::NOT_FOUND,
            Json(FileResponse {
                error: Some("file not found".to_string()),
                ..Default::default()
            }),
        ),
        SaveOutcome::Io(e) => (
            StatusCode::BAD_REQUEST,
            Json(FileResponse {
                error: Some(e),
                ..Default::default()
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty() {
        assert_eq!(validate_customization_path(""), Err("invalid path"));
    }

    #[test]
    fn rejects_absolute_unix() {
        assert_eq!(
            validate_customization_path("/etc/passwd"),
            Err("invalid path")
        );
    }

    #[test]
    fn rejects_absolute_windows() {
        assert_eq!(
            validate_customization_path("\\Windows\\System32"),
            Err("invalid path")
        );
    }

    #[test]
    fn rejects_parent_traversal() {
        assert_eq!(
            validate_customization_path("../../etc/passwd"),
            Err("invalid path")
        );
        assert_eq!(
            validate_customization_path("agent-prompts/../../etc/passwd"),
            Err("invalid path")
        );
    }

    #[test]
    fn rejects_non_md() {
        assert_eq!(
            validate_customization_path("settings.json"),
            Err("only markdown files allowed")
        );
        assert_eq!(
            validate_customization_path("agent-prompts/foo.txt"),
            Err("only markdown files allowed")
        );
    }

    #[test]
    fn rejects_nested_subdirs() {
        assert_eq!(
            validate_customization_path("wards/foo/bar.md"),
            Err("invalid path")
        );
        assert_eq!(
            validate_customization_path("agent-prompts/foo/bar.md"),
            Err("invalid path")
        );
    }

    #[test]
    fn accepts_agent_contracts() {
        assert_eq!(
            validate_customization_path("agent/SOUL.md"),
            Ok(PathBuf::from("agent/SOUL.md"))
        );
        assert_eq!(
            validate_customization_path("agent/INSTRUCTIONS.md"),
            Ok(PathBuf::from("agent/INSTRUCTIONS.md"))
        );
    }

    #[test]
    fn accepts_canonical_prompt_md() {
        assert_eq!(
            validate_customization_path("agent-prompts/memory-learning.md"),
            Ok(PathBuf::from("agent-prompts/memory-learning.md"))
        );
    }

    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn list_files_finds_agent_contracts_and_prompts() {
        let tmp = tempdir().unwrap();
        let config = tmp.path();
        let agent = config.join("agent");
        fs::create_dir_all(&agent).unwrap();
        fs::write(agent.join("SOUL.md"), "soul").unwrap();
        fs::write(agent.join("INSTRUCTIONS.md"), "instr").unwrap();
        fs::write(agent.join("OS.md"), "os").unwrap();
        fs::write(config.join("settings.json"), "{}").unwrap();
        fs::create_dir_all(config.join("agent-prompts")).unwrap();
        fs::write(
            config.join("agent-prompts").join("first-turn-protocol.md"),
            "prompt",
        )
        .unwrap();
        fs::write(config.join("agent-prompts").join("ignored.txt"), "no").unwrap();

        let entries = enumerate_customization_files(config).expect("enumerate ok");
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"agent/SOUL.md"));
        assert!(paths.contains(&"agent/INSTRUCTIONS.md"));
        assert!(paths.contains(&"agent/OS.md"));
        assert!(paths.contains(&"agent-prompts/first-turn-protocol.md"));
        assert!(!paths.contains(&"settings.json"));
        assert!(!paths.contains(&"agent-prompts/ignored.txt"));

        let os_entry = entries.iter().find(|e| e.path == "agent/OS.md").unwrap();
        assert!(os_entry.auto_generated);

        let soul_entry = entries.iter().find(|e| e.path == "agent/SOUL.md").unwrap();
        assert!(!soul_entry.auto_generated);
    }

    use std::time::Duration;

    fn touch(path: &Path, content: &str) {
        fs::write(path, content).unwrap();
    }

    #[test]
    fn read_file_content_resolves_relative_to_config_dir() {
        let tmp = tempdir().unwrap();
        let config = tmp.path();
        fs::create_dir_all(config.join("agent")).unwrap();
        touch(&config.join("agent").join("SOUL.md"), "soul body");

        let resolved = resolve_path(config, "agent/SOUL.md").expect("ok");
        assert_eq!(resolved, config.join("agent").join("SOUL.md"));

        let body = fs::read_to_string(&resolved).unwrap();
        assert_eq!(body, "soul body");
    }

    #[test]
    fn read_file_rejects_invalid_path() {
        let tmp = tempdir().unwrap();
        assert!(resolve_path(tmp.path(), "../escape.md").is_err());
        assert!(resolve_path(tmp.path(), "/abs.md").is_err());
        assert!(resolve_path(tmp.path(), "wards/x.md").is_err());
    }

    #[test]
    fn save_file_succeeds_when_version_matches() {
        let tmp = tempdir().unwrap();
        let config = tmp.path();
        fs::create_dir_all(config.join("agent")).unwrap();
        let file = config.join("agent").join("SOUL.md");
        touch(&file, "v1");

        let initial_version = file_version(&file).unwrap();
        std::thread::sleep(Duration::from_millis(20));

        let result = save_file_with_check(config, "agent/SOUL.md", "v2", &initial_version);
        assert!(matches!(result, SaveOutcome::Ok(_)));
        assert_eq!(fs::read_to_string(&file).unwrap(), "v2");
    }

    #[test]
    fn save_file_returns_conflict_when_disk_changed() {
        let tmp = tempdir().unwrap();
        let config = tmp.path();
        fs::create_dir_all(config.join("agent")).unwrap();
        let file = config.join("agent").join("SOUL.md");
        touch(&file, "v1");

        let stale_version = file_version(&file).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        // Someone else updates the file:
        touch(&file, "v_external");

        let result = save_file_with_check(config, "agent/SOUL.md", "v_ours", &stale_version);
        match result {
            SaveOutcome::Conflict {
                current_content,
                current_version,
            } => {
                assert_eq!(current_content, "v_external");
                assert_ne!(current_version, stale_version);
            }
            other => panic!("expected Conflict, got {:?}", other),
        }
    }

    #[test]
    fn save_file_returns_not_found_for_missing_file() {
        let tmp = tempdir().unwrap();
        let result = save_file_with_check(tmp.path(), "agent-prompts/missing.md", "x", "v");
        assert!(matches!(result, SaveOutcome::NotFound));
    }

    #[test]
    fn save_file_returns_io_for_invalid_path() {
        let tmp = tempdir().unwrap();
        let result = save_file_with_check(tmp.path(), "../escape.md", "x", "v");
        match result {
            SaveOutcome::Io(msg) => assert!(msg.contains("invalid path")),
            other => panic!("expected Io, got {:?}", other),
        }
    }

    #[test]
    fn save_file_returns_io_for_non_md_extension() {
        let tmp = tempdir().unwrap();
        let result = save_file_with_check(tmp.path(), "foo.txt", "x", "v");
        match result {
            SaveOutcome::Io(msg) => assert!(msg.contains("invalid path")),
            other => panic!("expected Io, got {:?}", other),
        }
    }

    #[test]
    fn enumerate_returns_empty_when_no_md_files() {
        let tmp = tempdir().unwrap();
        let entries = enumerate_customization_files(tmp.path()).expect("ok");
        assert!(entries.is_empty());
    }

    #[test]
    fn enumerate_skips_when_prompt_dir_missing() {
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("agent")).unwrap();
        fs::write(tmp.path().join("agent").join("SOUL.md"), "x").unwrap();
        let entries = enumerate_customization_files(tmp.path()).expect("ok");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "agent/SOUL.md");
    }

    #[test]
    fn enumerate_sorts_agent_before_prompts_then_alphabetic() {
        let tmp = tempdir().unwrap();
        let config = tmp.path();
        fs::create_dir_all(config.join("agent")).unwrap();
        fs::write(config.join("agent").join("SOUL.md"), "z").unwrap();
        fs::write(config.join("agent").join("OS.md"), "a").unwrap();
        fs::create_dir_all(config.join("agent-prompts")).unwrap();
        fs::write(config.join("agent-prompts").join("z-prompt.md"), "z").unwrap();
        fs::write(config.join("agent-prompts").join("a-prompt.md"), "a").unwrap();

        let entries = enumerate_customization_files(config).expect("ok");
        let order: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            order,
            vec![
                "agent/OS.md",
                "agent/SOUL.md",
                "agent-prompts/a-prompt.md",
                "agent-prompts/z-prompt.md"
            ]
        );
    }

    #[test]
    fn file_kind_serialization_matches_lowercase() {
        let entry = FileEntry {
            path: "agent/SOUL.md".into(),
            kind: FileKind::Agent,
            size: 0,
            modified_at: "2026-04-01T00:00:00Z".into(),
            auto_generated: false,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"kind\":\"agent\""));
        let entry2 = FileEntry {
            kind: FileKind::Prompt,
            ..entry
        };
        let json = serde_json::to_string(&entry2).unwrap();
        assert!(json.contains("\"kind\":\"prompt\""));
    }

    #[test]
    fn save_outcome_ok_carries_new_version() {
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("agent")).unwrap();
        let file = tmp.path().join("agent").join("SOUL.md");
        touch(&file, "v1");
        let initial = file_version(&file).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let outcome = save_file_with_check(tmp.path(), "agent/SOUL.md", "v2", &initial);
        match outcome {
            SaveOutcome::Ok(new_version) => assert_ne!(new_version, initial),
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn resolve_path_joins_to_config_dir() {
        let tmp = tempdir().unwrap();
        let resolved = resolve_path(tmp.path(), "agent-prompts/foo.md").expect("ok");
        assert_eq!(resolved, tmp.path().join("agent-prompts/foo.md"));
    }
}

#[cfg(test)]
mod handler_tests {
    use super::*;
    use crate::AppState;
    use axum::extract::{Query, State};
    use std::fs;
    use tempfile::TempDir;

    fn make_state() -> (TempDir, AppState) {
        let dir = TempDir::new().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("agents")).unwrap();
        std::fs::create_dir_all(dir.path().join("skills")).unwrap();
        let state = AppState::minimal(dir.path().to_path_buf());
        (dir, state)
    }

    #[tokio::test]
    async fn list_files_returns_ok_with_existing_files() {
        let (_dir, state) = make_state();
        let cfg = state.paths.config_dir();
        std::fs::create_dir_all(cfg.join("agent")).unwrap();
        fs::write(cfg.join("agent").join("SOUL.md"), "soul body").unwrap();

        let (status, body) = list_files(State(state)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.0.success);
        let files = body.0.files.unwrap();
        assert!(files.iter().any(|f| f.path == "agent/SOUL.md"));
    }

    #[tokio::test]
    async fn get_file_returns_400_for_invalid_path() {
        let (_dir, state) = make_state();
        let q = Query(PathQuery {
            path: "../escape.md".to_string(),
        });
        let (status, body) = get_file(State(state), q).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.0.error.is_some());
    }

    #[tokio::test]
    async fn get_file_returns_404_when_file_missing() {
        let (_dir, state) = make_state();
        let q = Query(PathQuery {
            path: "agent-prompts/missing.md".to_string(),
        });
        let (status, body) = get_file(State(state), q).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body.0.error.as_deref(), Some("file not found"));
    }

    #[tokio::test]
    async fn get_file_returns_content_with_version_for_existing_file() {
        let (_dir, state) = make_state();
        let cfg = state.paths.config_dir();
        std::fs::create_dir_all(cfg.join("agent")).unwrap();
        fs::write(cfg.join("agent").join("SOUL.md"), "soul body").unwrap();

        let q = Query(PathQuery {
            path: "agent/SOUL.md".to_string(),
        });
        let (status, body) = get_file(State(state), q).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.0.success);
        assert_eq!(body.0.content.as_deref(), Some("soul body"));
        assert!(body.0.version.is_some());
        assert_eq!(body.0.auto_generated, Some(false));
    }

    #[tokio::test]
    async fn get_file_marks_os_md_as_auto_generated() {
        let (_dir, state) = make_state();
        let cfg = state.paths.config_dir();
        std::fs::create_dir_all(cfg.join("agent")).unwrap();
        fs::write(cfg.join("agent").join("OS.md"), "os body").unwrap();

        let q = Query(PathQuery {
            path: "agent/OS.md".to_string(),
        });
        let (_status, body) = get_file(State(state), q).await;
        assert_eq!(body.0.auto_generated, Some(true));
    }

    #[tokio::test]
    async fn put_file_returns_413_when_content_too_large() {
        let (_dir, state) = make_state();
        let body = SaveRequest {
            path: "agent/SOUL.md".to_string(),
            content: "x".repeat(MAX_CONTENT_BYTES + 1),
            expected_version: "v".to_string(),
        };
        let (status, response) = put_file(State(state), Json(body)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(response.0.error.unwrap().contains("too large"));
    }

    #[tokio::test]
    async fn put_file_returns_404_when_file_missing() {
        let (_dir, state) = make_state();
        let body = SaveRequest {
            path: "agent-prompts/missing.md".to_string(),
            content: "x".to_string(),
            expected_version: "v".to_string(),
        };
        let (status, response) = put_file(State(state), Json(body)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(response.0.error.as_deref(), Some("file not found"));
    }

    #[tokio::test]
    async fn put_file_returns_400_for_invalid_path() {
        let (_dir, state) = make_state();
        let body = SaveRequest {
            path: "../escape.md".to_string(),
            content: "x".to_string(),
            expected_version: "v".to_string(),
        };
        let (status, response) = put_file(State(state), Json(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(response.0.error.is_some());
    }

    #[tokio::test]
    async fn put_file_succeeds_when_version_matches() {
        let (_dir, state) = make_state();
        let cfg = state.paths.config_dir();
        std::fs::create_dir_all(cfg.join("agent")).unwrap();
        let file_path = cfg.join("agent").join("SOUL.md");
        fs::write(&file_path, "v1").unwrap();
        let initial = file_version(&file_path).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));

        let body = SaveRequest {
            path: "agent/SOUL.md".to_string(),
            content: "v2".to_string(),
            expected_version: initial,
        };
        let (status, response) = put_file(State(state), Json(body)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(response.0.success);
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "v2");
    }

    #[tokio::test]
    async fn put_file_returns_409_on_version_conflict() {
        let (_dir, state) = make_state();
        let cfg = state.paths.config_dir();
        std::fs::create_dir_all(cfg.join("agent")).unwrap();
        let file_path = cfg.join("agent").join("SOUL.md");
        fs::write(&file_path, "v1").unwrap();
        let stale = file_version(&file_path).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&file_path, "v_external").unwrap();

        let body = SaveRequest {
            path: "agent/SOUL.md".to_string(),
            content: "v_ours".to_string(),
            expected_version: stale,
        };
        let (status, response) = put_file(State(state), Json(body)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(response.0.current_content.as_deref(), Some("v_external"));
        assert!(response.0.current_version.is_some());
    }
}
