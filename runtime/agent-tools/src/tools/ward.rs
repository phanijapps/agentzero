// ============================================================================
// WARD TOOL
// Agent-managed project containers (named directories)
// ============================================================================

use std::io::{Read, Seek, Write};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use agent_primitives::{
    AgentError, DelegateAction, FileSystemContext, Result, Tool, ToolContext, ToolPermissions,
    WardArchetypeId,
};
use zbot_stores_traits::MemoryFactStore;

use crate::tools::guards::{planning_gate_awaits_ward, start_planning_after_ward};

/// AGENTS.md file name - living readme for agent executions
const WARD_AGENTS_MD: &str = "AGENTS.md";

/// Observer for ward-tool creation events. Implemented gateway-side so the
/// ward curator's telemetry sidecar gets a `created_by = "agent"` mark
/// whenever an agent scaffolds a new ward through this tool.
///
/// The trait is async (mirrors [`IngestionAccess`] / [`GoalAccess`]) even
/// though current implementations only do a brief synchronous bump — keeps
/// the surface uniform if future observers wire IO.
#[async_trait]
pub trait WardUsageAccess: Send + Sync + 'static {
    /// Called immediately after the ward tool creates a new ward directory,
    /// before the tool returns. Implementations should not panic — telemetry
    /// is best-effort.
    async fn mark_created_agent(&self, ward: &str, archetype: WardArchetypeId);
}

/// Gateway-provided access to the user-owned ward layout template.
///
/// The tool intentionally knows nothing about layout roles or YAML fields. It
/// receives only a bounded context packet and lint report from the lower-level
/// generic interpreter.
pub trait WardLayoutAccess: Send + Sync + 'static {
    fn create(
        &self,
        ward: &str,
        archetype: Option<WardArchetypeId>,
    ) -> std::result::Result<WardLayoutState, String>;
    fn rollback_created(&self, ward: &str) -> std::result::Result<(), String>;
    fn load(&self, ward: &str) -> std::result::Result<WardLayoutState, String>;
    fn validate(&self, ward: &str, expected_digest: &str) -> std::result::Result<(), String>;
    fn lint(&self, ward: &str, expected_digest: &str) -> std::result::Result<Value, String>;
    fn concept(
        &self,
        ward: &str,
        components: &[String],
        expected_digest: &str,
        apply: bool,
    ) -> std::result::Result<Value, String>;
}

#[derive(Debug, Clone)]
pub struct WardLayoutState {
    pub context: Option<String>,
    pub packet: Value,
}

/// Which actions the model-facing surface (description + schema) advertises,
/// mirroring what `validate_template_context` actually permits per actor:
/// `lint` is the delegated planner's post-write check; `dry_run`/
/// `create_concept` are root-only (plan-composer drives them from root-owned
/// setup steps); ward agents and other subagents hold no template actions.
/// Declaration-only: `execute()` handles every action on every instance —
/// enforcement stays with `validate_template_context`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WardAudience {
    /// Every action (back-compat `new()`; direct tests).
    Full,
    /// Root: lifecycle + search + `dry_run`/`create_concept`, no `lint`
    /// (the planner owns conformance checking).
    Root,
    /// Delegated planner: lifecycle + search + `lint`.
    Planner,
    /// Ward agents and other subagents: lifecycle + search only.
    Subagent,
}

impl WardAudience {
    fn show_lint(self) -> bool {
        matches!(self, Self::Full | Self::Planner)
    }
    fn show_concept_actions(self) -> bool {
        matches!(self, Self::Full | Self::Root)
    }
}

/// Tool for managing wards (named project directories).
///
/// Wards are persistent, agent-named project directories under `vault/wards/`.
/// The agent autonomously creates and switches between wards.
pub struct WardTool {
    fs: Arc<dyn FileSystemContext>,
    fact_store: Option<Arc<dyn MemoryFactStore>>,
    /// Optional observer that gets a `created_by = "agent"` mark whenever the
    /// `use`/`create` action scaffolds a new ward directory. `None` is a
    /// valid no-op (tests, minimal configurations).
    ward_usage: Option<Arc<dyn WardUsageAccess>>,
    ward_layout: Arc<dyn WardLayoutAccess>,
    audience: WardAudience,
}

impl WardTool {
    fn validate_ward_name(name: &str) -> Result<()> {
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(AgentError::Tool(
                "Ward name must be 1-64 ASCII letters, numbers, hyphens, or underscores"
                    .to_string(),
            ));
        }
        Ok(())
    }

    /// Create a new WardTool with file system context, optional fact store,
    /// and optional ward-usage observer. Full action surface (back-compat;
    /// production registrations pick an audience explicitly).
    #[must_use]
    pub fn new(
        fs: Arc<dyn FileSystemContext>,
        fact_store: Option<Arc<dyn MemoryFactStore>>,
        ward_usage: Option<Arc<dyn WardUsageAccess>>,
        ward_layout: Arc<dyn WardLayoutAccess>,
    ) -> Self {
        Self::with_audience(fs, fact_store, ward_usage, ward_layout, WardAudience::Full)
    }

    /// Root audience: lifecycle + search + `dry_run`/`create_concept`
    /// (root is their only legal executor — plan-composer drives them from
    /// root-owned setup steps). `lint` stays hidden: the planner owns
    /// conformance checking.
    #[must_use]
    pub fn for_root(
        fs: Arc<dyn FileSystemContext>,
        fact_store: Option<Arc<dyn MemoryFactStore>>,
        ward_usage: Option<Arc<dyn WardUsageAccess>>,
        ward_layout: Arc<dyn WardLayoutAccess>,
    ) -> Self {
        Self::with_audience(fs, fact_store, ward_usage, ward_layout, WardAudience::Root)
    }

    /// Delegated-planner audience: lifecycle + search + `lint`.
    #[must_use]
    pub fn for_planner(
        fs: Arc<dyn FileSystemContext>,
        fact_store: Option<Arc<dyn MemoryFactStore>>,
        ward_usage: Option<Arc<dyn WardUsageAccess>>,
        ward_layout: Arc<dyn WardLayoutAccess>,
    ) -> Self {
        Self::with_audience(
            fs,
            fact_store,
            ward_usage,
            ward_layout,
            WardAudience::Planner,
        )
    }

    /// Ward-agent / ordinary-subagent audience: lifecycle + search only —
    /// `validate_template_context` rejects every template action for them.
    #[must_use]
    pub fn for_subagent(
        fs: Arc<dyn FileSystemContext>,
        fact_store: Option<Arc<dyn MemoryFactStore>>,
        ward_usage: Option<Arc<dyn WardUsageAccess>>,
        ward_layout: Arc<dyn WardLayoutAccess>,
    ) -> Self {
        Self::with_audience(
            fs,
            fact_store,
            ward_usage,
            ward_layout,
            WardAudience::Subagent,
        )
    }

    #[must_use]
    fn with_audience(
        fs: Arc<dyn FileSystemContext>,
        fact_store: Option<Arc<dyn MemoryFactStore>>,
        ward_usage: Option<Arc<dyn WardUsageAccess>>,
        ward_layout: Arc<dyn WardLayoutAccess>,
        audience: WardAudience,
    ) -> Self {
        Self {
            fs,
            fact_store,
            ward_usage,
            ward_layout,
            audience,
        }
    }

    /// List files in a ward directory (non-recursive, top-level only).
    fn list_ward_files(&self, ward_dir: &std::path::Path) -> Vec<String> {
        let mut files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(ward_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                // Skip hidden files
                if name.starts_with('.') {
                    continue;
                }
                if entry.path().is_dir() {
                    files.push(format!("{}/", name));
                } else {
                    files.push(name);
                }
            }
        }
        files.sort();
        files
    }

    /// Read the ward's AGENTS.md doctrine, bounded to `AGENTS_MD_CAP` bytes.
    /// Oversized doctrine truncates with a visible marker instead of loading
    /// unbounded bytes into the model context. Doctrine that is not valid
    /// UTF-8 within the cap is dropped (None), matching the previous
    /// `read_to_string` failure semantics — never a misleading marker.
    fn read_agents_md(&self, ward_dir: &std::path::Path) -> Option<String> {
        const AGENTS_MD_CAP: usize = 32 * 1024;
        use std::io::Read;
        let file = std::fs::File::open(ward_dir.join(WARD_AGENTS_MD)).ok()?;
        let mut bytes = Vec::with_capacity(AGENTS_MD_CAP.min(8 * 1024));
        file.take(AGENTS_MD_CAP as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > AGENTS_MD_CAP {
            bytes.truncate(AGENTS_MD_CAP);
            let mut doctrine = String::from_utf8(bytes).ok()?;
            doctrine.push_str(&format!(
                "\n\n[AGENTS.md truncated at {} KiB]",
                AGENTS_MD_CAP / 1024
            ));
            return Some(doctrine);
        }
        String::from_utf8(bytes).ok()
    }

    fn register_ward(wards_root: &std::path::Path, ward_name: &str) -> std::io::Result<()> {
        let index = wards_root.join("index.md");
        let before = std::fs::symlink_metadata(&index)?;
        validate_catalog_metadata(&before)?;
        let mut file = open_catalog_for_update(&index, &before)?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        let display_name = ward_display_name(ward_name);
        let entry = format!("- [[{ward_name}/{ward_name}|{display_name}]]");
        if !content.lines().any(|line| line.trim() == entry) {
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(&entry);
            content.push('\n');
            file.seek(std::io::SeekFrom::Start(0))?;
            file.set_len(0)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
        }
        Ok(())
    }

    fn ensure_ward_catalog(wards_root: &std::path::Path) -> std::io::Result<()> {
        ensure_ward_catalog(wards_root)
    }

    /// Recall facts relevant to the ward being entered.
    ///
    /// Best-effort: if no fact store is configured, or the recall fails,
    /// returns None and the ward switch still succeeds.
    async fn recall_ward_facts(
        &self,
        ward_name: &str,
        ctx: &Arc<dyn ToolContext>,
    ) -> Option<Value> {
        let store = self.fact_store.as_ref()?;

        let agent_id = ctx
            .get_state("app:agent_id")
            .and_then(|v| v.as_str().map(String::from))
            .or_else(|| {
                ctx.get_state("app:root_agent_id")
                    .and_then(|v| v.as_str().map(String::from))
            })?;

        let query = format!("ward {} context patterns corrections", ward_name);
        match store
            .recall_facts_prioritized(&agent_id, &query, 3, None)
            .await
        {
            Ok(result) => {
                let count = result.get("count").and_then(|c| c.as_u64()).unwrap_or(0);
                if count > 0 {
                    tracing::info!(
                        "Ward-entry recall for '{}': {} facts loaded",
                        ward_name,
                        count
                    );
                    Some(result)
                } else {
                    None
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Ward-entry recall failed for '{}': {} (non-fatal)",
                    ward_name,
                    e
                );
                None
            }
        }
    }

    /// Get a short description from AGENTS.md purpose section (if any).
    fn ward_description(&self, ward_dir: &std::path::Path) -> Option<String> {
        let content = self.read_agents_md(ward_dir)?;
        // Look for Purpose section and extract first non-empty, non-comment line
        let mut in_purpose = false;
        for line in content.lines() {
            if line.starts_with("## Purpose") {
                in_purpose = true;
                continue;
            }
            if in_purpose {
                if line.starts_with("## ") {
                    break; // Next section
                }
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with("<!--") {
                    return Some(trimmed.to_string());
                }
            }
        }
        None
    }
}

pub fn ensure_ward_catalog(wards_root: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(wards_root)?;
    let root_metadata = std::fs::symlink_metadata(wards_root)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(std::io::Error::other("wards root must be a real directory"));
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        let root = open_directory_nofollow(wards_root)?;
        let opened = root.metadata()?;
        if root_metadata.dev() != opened.dev() || root_metadata.ino() != opened.ino() {
            return Err(std::io::Error::other(
                "wards root changed while it was opened",
            ));
        }
        ensure_ward_catalog_at(&root)
    }
    #[cfg(not(target_os = "linux"))]
    {
        ensure_ward_catalog_portable(wards_root)
    }
}

#[cfg(not(target_os = "linux"))]
fn ensure_ward_catalog_portable(wards_root: &std::path::Path) -> std::io::Result<()> {
    let index = wards_root.join("index.md");
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&index)
    {
        Ok(mut file) => {
            file.write_all(b"# Wards\n")?;
            file.sync_all()
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            validate_catalog_metadata(&std::fs::symlink_metadata(index)?)
        }
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "linux")]
fn ensure_ward_catalog_at(root: &std::fs::File) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};

    let name = CString::new("index.md").expect("static catalog filename");
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: `name` is NUL-terminated and `root` owns a live directory fd.
    let fd = unsafe { libc::openat(root.as_raw_fd(), name.as_ptr(), flags, 0o666) };
    if fd >= 0 {
        // SAFETY: `openat` returned a fresh descriptor now owned by `file`.
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        file.write_all(b"# Wards\n")?;
        return file.sync_all();
    }
    let error = std::io::Error::last_os_error();
    if error.kind() != std::io::ErrorKind::AlreadyExists {
        return Err(error);
    }

    let fd = unsafe {
        libc::openat(
            root.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `openat` returned a fresh descriptor now owned by `file`.
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    validate_catalog_metadata(&file.metadata()?)
}

fn validate_catalog_metadata(metadata: &std::fs::Metadata) -> std::io::Result<()> {
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(std::io::Error::other(
            "wards/index.md must be a real regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(std::io::Error::other(
                "wards/index.md must have exactly one link",
            ));
        }
    }
    Ok(())
}

fn ward_display_name(ward: &str) -> String {
    ward.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                format!("{}{}", first.to_ascii_uppercase(), chars.as_str())
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn validate_same_catalog_file(
    before: &std::fs::Metadata,
    opened: &std::fs::Metadata,
) -> std::io::Result<()> {
    validate_catalog_metadata(opened)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != opened.dev() || before.ino() != opened.ino() {
            return Err(std::io::Error::other(
                "wards/index.md changed while it was opened",
            ));
        }
    }
    Ok(())
}

fn open_catalog_for_update(
    index: &std::path::Path,
    before: &std::fs::Metadata,
) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options.open(index)?;
    let opened = file.metadata()?;
    validate_same_catalog_file(before, &opened)?;
    fs2::FileExt::lock_exclusive(&file)?;
    let after = std::fs::symlink_metadata(index)?;
    validate_same_catalog_file(&opened, &after)?;
    Ok(file)
}

fn required_name<'a>(args: &'a Value, action: &str) -> Result<&'a str> {
    args.get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| AgentError::Tool(format!("Missing 'name' parameter for {action}")))
}

fn reject_unknown_fields(action: &str, args: &Value) -> Result<()> {
    let allowed: &[&str] = match action {
        "list" => &["action"],
        "use" | "info" | "lint" => &["action", "name"],
        "create" => &["action", "name", "archetype"],
        "search" => &["action", "name", "query", "tags", "limit"],
        "dry_run" => &["action", "name", "operation", "components"],
        "create_concept" => &["action", "name", "components"],
        _ => return Ok(()),
    };
    if let Some(key) = args
        .as_object()
        .and_then(|object| object.keys().find(|key| !allowed.contains(&key.as_str())))
    {
        return Err(AgentError::Tool(format!(
            "ward: unknown field '{key}' for action '{action}'"
        )));
    }
    Ok(())
}

fn is_delegated_planner(ctx: &dyn ToolContext) -> bool {
    ctx.get_state("app:actor_kind")
        .and_then(|value| value.as_str().map(str::to_owned))
        .as_deref()
        == Some("delegated_executor")
        && ctx
            .get_state("app:agent_id")
            .and_then(|value| value.as_str().map(str::to_owned))
            .as_deref()
            == Some("planner-agent")
}

fn validate_template_context(
    ctx: &dyn ToolContext,
    ward: &str,
    allow_planner_lint: bool,
) -> std::result::Result<String, &'static str> {
    let actor_kind = ctx
        .get_state("app:actor_kind")
        .and_then(|value| value.as_str().map(str::to_owned));
    let is_root = actor_kind.as_deref() == Some("root");
    let is_bundled_planner = allow_planner_lint && is_delegated_planner(ctx);
    if !is_root && !is_bundled_planner {
        return Err("root_required");
    }
    let packet = ctx
        .get_state("ward_template")
        .ok_or("template_unavailable")?;
    if packet.get("status").and_then(Value::as_str) != Some("available") {
        return Err("template_unavailable");
    }
    let host_session_id = ctx
        .get_state("session_id")
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or("template_stale")?;
    if packet.get("ward_id").and_then(Value::as_str) != Some(ward)
        || packet.get("session_id").and_then(Value::as_str) != Some(host_session_id.as_str())
        || packet.get("root_context_id").and_then(Value::as_str)
            != ctx
                .get_state("ward_template_context_id")
                .and_then(|value| value.as_str().map(str::to_owned))
                .as_deref()
        || ctx
            .get_state("ward_id")
            .and_then(|v| v.as_str().map(str::to_owned))
            != Some(ward.to_string())
    {
        return Err("template_stale");
    }
    packet
        .get("digest")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or("template_stale")
}

fn parse_tags(args: &Value) -> Result<Vec<String>> {
    let values = args
        .get("tags")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if values.len() > 32 {
        return Err(AgentError::Tool(
            "ward search accepts at most 32 tags".into(),
        ));
    }
    values
        .into_iter()
        .map(|value| {
            let tag = value
                .as_str()
                .filter(|tag| !tag.is_empty() && tag.len() <= 64)
                .ok_or_else(|| AgentError::Tool("ward search tag is invalid".into()))?;
            Ok(tag.to_lowercase())
        })
        .collect()
}

fn parse_components(args: &Value) -> Result<Vec<String>> {
    let values = args
        .get("components")
        .and_then(Value::as_array)
        .ok_or_else(|| AgentError::Tool("Missing 'components' parameter".into()))?;
    if values.is_empty() || values.len() > 16 {
        return Err(AgentError::Tool("concept path is invalid".into()));
    }
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| AgentError::Tool("concept path is invalid".into()))
        })
        .collect()
}

fn ok_envelope(action: &str, ward: &str, digest: &str, data: Value) -> Value {
    json!({"ok":true,"action":action,"ward_id":ward,"template_digest":digest,"data":data})
}

fn error_envelope(action: &str, ward: &str, digest: &str, code: &str) -> Value {
    json!({
        "ok":false,"action":action,"ward_id":ward,"template_digest":digest,
        "error":{"code":code,"message":"The Ward operation could not be completed."}
    })
}

fn search_markdown(
    root: &std::path::Path,
    query: &str,
    required_tags: &[String],
    limit: usize,
) -> std::result::Result<Value, String> {
    #[cfg(target_os = "linux")]
    {
        search_markdown_linux(root, query, required_tags, limit)
    }
    #[cfg(not(target_os = "linux"))]
    {
        search_markdown_portable(root, query, required_tags, limit)
    }
}

#[cfg(target_os = "linux")]
fn search_markdown_linux(
    root: &std::path::Path,
    query: &str,
    required_tags: &[String],
    limit: usize,
) -> std::result::Result<Value, String> {
    use std::os::fd::{AsRawFd, FromRawFd};

    if query.len() > 256 || !(1..=50).contains(&limit) {
        return Err("invalid_search".into());
    }
    let wards_path = root
        .parent()
        .ok_or_else(|| "ward_unavailable".to_string())?;
    let ward_name = root
        .file_name()
        .ok_or_else(|| "ward_unavailable".to_string())?;
    let before =
        std::fs::symlink_metadata(wards_path).map_err(|_| "ward_unavailable".to_string())?;
    if before.file_type().is_symlink() || !before.is_dir() {
        return Err("ward_unavailable".into());
    }
    let wards = open_directory_nofollow(wards_path).map_err(|_| "ward_unavailable".to_string())?;
    let opened = wards
        .metadata()
        .map_err(|_| "ward_unavailable".to_string())?;
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != opened.dev() || before.ino() != opened.ino() {
            return Err("search_race".into());
        }
    }
    let ward_fd =
        open_at_nofollow(&wards, ward_name, true).map_err(|_| "ward_unavailable".to_string())?;
    // SAFETY: open_at_nofollow returns a fresh owned descriptor.
    let ward = unsafe { std::fs::File::from_raw_fd(ward_fd) };
    let mut pending = vec![(ward, std::path::PathBuf::new())];
    let mut results = Vec::new();
    let mut files_visited = 0usize;
    let mut bytes_read = 0usize;
    let mut entries_visited = 0usize;
    let mut truncated = false;
    let query = query.to_lowercase();

    while let Some((directory, relative_dir)) = pending.pop() {
        let fd_path = std::path::PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
        let remaining_entries = 10_000usize.saturating_sub(entries_visited);
        let mut source = std::fs::read_dir(fd_path).map_err(|_| "search_failed".to_string())?;
        let mut entries: Vec<_> = source
            .by_ref()
            .filter_map(|entry| entry.ok())
            .take(remaining_entries.saturating_add(1))
            .collect();
        if entries.len() > remaining_entries {
            entries.pop();
            truncated = true;
        }
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries.into_iter().rev() {
            entries_visited += 1;
            if entries_visited > 10_000 {
                truncated = true;
                break;
            }
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            if let Ok(fd) = open_at_nofollow(&directory, &name, true) {
                // SAFETY: open_at_nofollow returns a fresh owned descriptor.
                let child = unsafe { std::fs::File::from_raw_fd(fd) };
                pending.push((child, relative_dir.join(&name)));
                continue;
            }
            if std::path::Path::new(&name)
                .extension()
                .and_then(|value| value.to_str())
                != Some("md")
            {
                continue;
            }
            if files_visited >= 2_000 || bytes_read >= 8 * 1024 * 1024 {
                truncated = true;
                break;
            }
            let fd = match open_at_nofollow(&directory, &name, false) {
                Ok(fd) => fd,
                Err(_) => continue,
            };
            // SAFETY: open_at_nofollow returns a fresh owned descriptor.
            let file = unsafe { std::fs::File::from_raw_fd(fd) };
            if !file
                .metadata()
                .map_err(|_| "search_failed".to_string())?
                .is_file()
            {
                continue;
            }
            files_visited += 1;
            let remaining = 8 * 1024 * 1024 - bytes_read;
            let Some(content) = read_open_file_bounded(file, remaining)? else {
                truncated = true;
                break;
            };
            bytes_read += content.len();
            let relative = relative_dir.join(&name);
            let (title, tags) = markdown_metadata(&content, &relative);
            let haystack = format!("{}\n{}\n{}", relative.display(), title, content).to_lowercase();
            if !query.is_empty() && !haystack.contains(&query) {
                continue;
            }
            let lowered: Vec<String> = tags.iter().map(|tag| tag.to_lowercase()).collect();
            if !required_tags.iter().all(|tag| lowered.contains(tag)) {
                continue;
            }
            results.push(json!({
                "path": relative.to_string_lossy().replace('\\', "/"),
                "title": title,
                "tags": tags,
            }));
        }
        if truncated {
            break;
        }
    }
    results.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    if results.len() > limit {
        results.truncate(limit);
        truncated = true;
    }
    Ok(json!({
        "results": results, "truncated": truncated,
        "files_visited": files_visited, "bytes_read": bytes_read
    }))
}

#[cfg(target_os = "linux")]
fn open_directory_nofollow(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

#[cfg(target_os = "linux")]
fn open_at_nofollow(
    parent: &std::fs::File,
    name: &std::ffi::OsStr,
    directory: bool,
) -> std::io::Result<i32> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        fn openat(dirfd: i32, pathname: *const std::ffi::c_char, flags: i32, ...) -> i32;
    }
    let name = CString::new(name.as_bytes()).map_err(|_| std::io::Error::other("invalid name"))?;
    let mut flags = 0o400000 | 0o2000000; // O_NOFOLLOW | O_CLOEXEC
    if directory {
        flags |= 0o200000; // O_DIRECTORY
    }
    // SAFETY: name is NUL-terminated and parent owns a live directory fd.
    let fd = unsafe { openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(fd)
    }
}

#[cfg(target_os = "linux")]
fn read_open_file_bounded(
    file: std::fs::File,
    limit: usize,
) -> std::result::Result<Option<String>, String> {
    use std::io::Read;
    let mut bytes = Vec::with_capacity(limit.min(8 * 1024));
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "search_failed".to_string())?;
    if bytes.len() > limit {
        return Ok(None);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "search_failed".to_string())
}

#[cfg(not(target_os = "linux"))]
fn search_markdown_portable(
    root: &std::path::Path,
    query: &str,
    required_tags: &[String],
    limit: usize,
) -> std::result::Result<Value, String> {
    if query.len() > 256 || !(1..=50).contains(&limit) {
        return Err("invalid_search".into());
    }
    let wards_root = root
        .parent()
        .ok_or_else(|| "ward_unavailable".to_string())?;
    let wards_metadata =
        std::fs::symlink_metadata(wards_root).map_err(|_| "ward_unavailable".to_string())?;
    let root_metadata =
        std::fs::symlink_metadata(root).map_err(|_| "ward_unavailable".to_string())?;
    if wards_metadata.file_type().is_symlink()
        || root_metadata.file_type().is_symlink()
        || !root_metadata.is_dir()
    {
        return Err("ward_unavailable".into());
    }
    let wards_root = wards_root
        .canonicalize()
        .map_err(|_| "ward_unavailable".to_string())?;
    let root = root
        .canonicalize()
        .map_err(|_| "ward_unavailable".to_string())?;
    if root.parent() != Some(wards_root.as_path()) {
        return Err("path_escape".into());
    }
    let mut pending = vec![root.clone()];
    let mut results = Vec::new();
    let mut files_visited = 0usize;
    let mut bytes_read = 0usize;
    let mut entries_visited = 0usize;
    let mut truncated = false;
    let query = query.to_lowercase();

    while let Some(directory) = pending.pop() {
        let canonical_directory = directory
            .canonicalize()
            .map_err(|_| "path_escape".to_string())?;
        if !canonical_directory.starts_with(&root) {
            return Err("path_escape".into());
        }
        let remaining_entries = 10_000usize.saturating_sub(entries_visited);
        let mut source = std::fs::read_dir(&directory).map_err(|_| "search_failed".to_string())?;
        let mut entries: Vec<_> = source
            .by_ref()
            .filter_map(|entry| entry.ok())
            .take(remaining_entries.saturating_add(1))
            .collect();
        if entries.len() > remaining_entries {
            entries.pop();
            truncated = true;
        }
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries.into_iter().rev() {
            entries_visited += 1;
            if entries_visited > 10_000 {
                truncated = true;
                break;
            }
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let file_type = entry.file_type().map_err(|_| "search_failed".to_string())?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if entry.path().extension().and_then(|value| value.to_str()) != Some("md") {
                continue;
            }
            if files_visited >= 2_000 || bytes_read >= 8 * 1024 * 1024 {
                truncated = true;
                break;
            }
            files_visited += 1;
            let remaining = 8 * 1024 * 1024 - bytes_read;
            let Some(content) = read_markdown_bounded(&entry.path(), remaining)? else {
                truncated = true;
                break;
            };
            bytes_read += content.len();
            let (title, tags) = markdown_metadata(&content, &entry.path());
            let entry_path = entry.path();
            let path = entry_path
                .strip_prefix(&root)
                .map_err(|_| "path_escape".to_string())?;
            let haystack = format!("{}\n{}\n{}", path.display(), title, content).to_lowercase();
            if !query.is_empty() && !haystack.contains(&query) {
                continue;
            }
            let lowered: Vec<String> = tags.iter().map(|tag| tag.to_lowercase()).collect();
            if !required_tags.iter().all(|tag| lowered.contains(tag)) {
                continue;
            }
            results.push(
                json!({"path":path.to_string_lossy().replace('\\', "/"),"title":title,"tags":tags}),
            );
        }
        if truncated {
            break;
        }
    }
    results.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    if results.len() > limit {
        results.truncate(limit);
        truncated = true;
    }
    Ok(
        json!({"results":results,"truncated":truncated,"files_visited":files_visited,"bytes_read":bytes_read}),
    )
}

#[cfg(not(target_os = "linux"))]
fn read_markdown_bounded(
    path: &std::path::Path,
    limit: usize,
) -> std::result::Result<Option<String>, String> {
    use std::io::Read;
    let before = std::fs::symlink_metadata(path).map_err(|_| "search_failed".to_string())?;
    if before.file_type().is_symlink() || !before.is_file() {
        return Ok(None);
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x20000); // O_NOFOLLOW
    }
    let file = options
        .open(path)
        .map_err(|_| "search_failed".to_string())?;
    let opened = file.metadata().map_err(|_| "search_failed".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != opened.dev() || before.ino() != opened.ino() {
            return Err("search_race".into());
        }
    }
    let mut bytes = Vec::with_capacity(limit.min(8 * 1024));
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "search_failed".to_string())?;
    if bytes.len() > limit {
        return Ok(None);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "search_failed".to_string())
}

fn markdown_metadata(content: &str, path: &std::path::Path) -> (String, Vec<String>) {
    let fallback = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_string();
    if !content.starts_with("---\n") {
        return (fallback, Vec::new());
    }
    let Some(end) = content[4..].find("\n---") else {
        return (fallback, Vec::new());
    };
    let frontmatter = &content[4..4 + end];
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(frontmatter) else {
        return (fallback, Vec::new());
    };
    let title = value
        .get("title")
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or(&fallback)
        .to_string();
    let tags = value
        .get("tags")
        .and_then(serde_yaml::Value::as_sequence)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_yaml::Value::as_str)
                .take(32)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    (title, tags)
}

/// Tool descriptions per audience — plain literals so `description()` keeps
/// its `&'str` signature. Bodies differ only in the action-list tail.
const WARD_DESC_FULL: &str = concat!(
    "Manage code wards (named project directories). Wards persist across sessions.\n\
             Arguments are action-specific; unknown fields are rejected.\n\
             Actions:\n\
             - use: Switch to a ward (creates if needed). Sets working directory for shell/write/edit.\n\
             - create: Alias for use. Creates and switches to a new ward.\n\
             - list: List all wards with descriptions.\n\
             - info: Detailed info about a specific ward.\n\
             - search: Search Markdown in the active ward by text and exact tags.",
    "\n\
             - lint: Check the active ward against its ward-conf.yaml snapshot.",
    "\n\
             - dry_run: Preview a template-directed create_concept operation.\n\
             - create_concept: Create the concept node annotated by the active template."
);
const WARD_DESC_ROOT: &str = concat!(
    "Manage code wards (named project directories). Wards persist across sessions.\n\
             Arguments are action-specific; unknown fields are rejected.\n\
             Actions:\n\
             - use: Switch to a ward (creates if needed). Sets working directory for shell/write/edit.\n\
             - create: Alias for use. Creates and switches to a new ward.\n\
             - list: List all wards with descriptions.\n\
             - info: Detailed info about a specific ward.\n\
             - search: Search Markdown in the active ward by text and exact tags.",
    "\n\
             - dry_run: Preview a template-directed create_concept operation.\n\
             - create_concept: Create the concept node annotated by the active template."
);
const WARD_DESC_PLANNER: &str = concat!(
    "Manage code wards (named project directories). Wards persist across sessions.\n\
             Arguments are action-specific; unknown fields are rejected.\n\
             Actions:\n\
             - use: Switch to a ward (creates if needed). Sets working directory for shell/write/edit.\n\
             - create: Alias for use. Creates and switches to a new ward.\n\
             - list: List all wards with descriptions.\n\
             - info: Detailed info about a specific ward.\n\
             - search: Search Markdown in the active ward by text and exact tags.",
    "\n\
             - lint: Check the active ward against its ward-conf.yaml snapshot."
);
const WARD_DESC_SUBAGENT: &str = "Manage code wards (named project directories). Wards persist across sessions.\n\
             Arguments are action-specific; unknown fields are rejected.\n\
             Actions:\n\
             - use: Switch to a ward (creates if needed). Sets working directory for shell/write/edit.\n\
             - create: Alias for use. Creates and switches to a new ward.\n\
             - list: List all wards with descriptions.\n\
             - info: Detailed info about a specific ward.\n\
             - search: Search Markdown in the active ward by text and exact tags.";

#[async_trait]
impl Tool for WardTool {
    fn name(&self) -> &str {
        "ward"
    }

    fn description(&self) -> &str {
        match self.audience {
            WardAudience::Full => WARD_DESC_FULL,
            WardAudience::Root => WARD_DESC_ROOT,
            WardAudience::Planner => WARD_DESC_PLANNER,
            WardAudience::Subagent => WARD_DESC_SUBAGENT,
        }
    }

    fn parameters_schema(&self) -> Option<Value> {
        let named = |actions: Value| {
            json!({
                "type": "object",
                "properties": {"action": actions, "name": {"type": "string"}},
                "required": ["action", "name"],
                "additionalProperties": false
            })
        };
        let mut branches = vec![
            named(json!({"enum": if self.audience.show_lint() {
                vec!["use", "info", "lint"]
            } else {
                vec!["use", "info"]
            }})),
            json!({"type":"object","properties":{
                "action":{"const":"create"},
                "name":{"type":"string"},
                "archetype":{"enum":["generic","coding","documentation","journal","ebook","research","news"]}
            },"required":["action","name"],"additionalProperties":false}),
            json!({"type":"object","properties":{"action":{"const":"list"}},"required":["action"],"additionalProperties":false}),
            json!({"type":"object","properties":{
                "action":{"const":"search"}, "name":{"type":"string"},
                "query":{"type":"string","maxLength":256},
                "tags":{"type":"array","maxItems":32,"items":{"type":"string","maxLength":64}},
                "limit":{"type":"integer","minimum":1,"maximum":50}
            },"required":["action","name"],"additionalProperties":false}),
        ];
        if self.audience.show_concept_actions() {
            branches.push(json!({"type":"object","properties":{
                "action":{"const":"dry_run"}, "name":{"type":"string"},
                "operation":{"const":"create_concept"},
                "components":{"type":"array","minItems":1,"maxItems":16,"items":{"type":"string","maxLength":64}}
            },"required":["action","name","operation","components"],"additionalProperties":false}));
            branches.push(json!({"type":"object","properties":{
                "action":{"const":"create_concept"}, "name":{"type":"string"},
                "components":{"type":"array","minItems":1,"maxItems":16,"items":{"type":"string","maxLength":64}}
            },"required":["action","name","components"],"additionalProperties":false}));
        }
        Some(json!({"oneOf": branches}))
    }

    fn permissions(&self) -> ToolPermissions {
        ToolPermissions::safe()
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        // Check for error markers from truncated/malformed tool calls
        if let Some(error_type) = args.get("__error__").and_then(|v| v.as_str()) {
            let message = args
                .get("__message__")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Err(AgentError::Tool(format!("{}: {}", error_type, message)));
        }

        let action = args.get("action").and_then(|v| v.as_str()).ok_or_else(|| {
            AgentError::Tool(
                "ward: missing 'action' parameter (one of: use, create, list, info)".to_string(),
            )
        })?;
        reject_unknown_fields(action, &args)?;

        let wards_root = self
            .fs
            .wards_root_dir()
            .ok_or_else(|| AgentError::Tool("Wards directory not configured".to_string()))?;

        match action {
            "use" | "create" => {
                let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                    AgentError::Tool("Missing 'name' parameter for use/create".to_string())
                })?;
                let archetype = if action == "create" {
                    args.get("archetype")
                        .and_then(Value::as_str)
                        .map(str::parse::<WardArchetypeId>)
                        .transpose()
                        .map_err(|_| {
                            AgentError::Tool(
                                "ward: invalid archetype; expected generic, coding, documentation, journal, ebook, research, or news"
                                    .to_string(),
                            )
                        })?
                } else {
                    None
                };

                Self::ensure_ward_catalog(&wards_root).map_err(|error| {
                    AgentError::Tool(format!("Failed to initialize Ward catalog: {error}"))
                })?;

                Self::validate_ward_name(name)?;

                // The delegated planner is host-bound to the selected ward.
                // It may re-enter that ward, but cannot mutate its file-tool
                // working directory by switching to a sibling ward.
                if is_delegated_planner(ctx.as_ref())
                    && validate_template_context(ctx.as_ref(), name, true).is_err()
                {
                    return Err(AgentError::Tool("planner_ward_locked".to_string()));
                }

                let ward_dir = wards_root.join(name);
                let created = !ward_dir.exists();

                // Subagents cannot create wards — only root can
                let is_delegated = ctx
                    .get_state("app:is_delegated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if created && is_delegated {
                    return Err(AgentError::Tool(format!(
                        "Subagents cannot create wards. Use the ward specified in your task: '{}' does not exist. \
                         Ask the root agent to create it first.",
                        name
                    )));
                }

                let mut layout_state = if created {
                    let state = self.ward_layout.create(name, archetype).map_err(|error| {
                        AgentError::Tool(format!("Ward template nudge: {error}"))
                    })?;
                    if let Err(error) = Self::register_ward(&wards_root, name) {
                        self.ward_layout.rollback_created(name).map_err(|rollback| {
                            AgentError::Tool(format!(
                                "Failed to update Ward catalog and roll back the new Ward: {error}; {rollback}"
                            ))
                        })?;
                        return Err(AgentError::Tool(format!(
                            "Failed to update Ward catalog; the new Ward was rolled back: {error}"
                        )));
                    }

                    // Tell the ward-curator telemetry sidecar that this ward
                    // is agent-authored. Curator-eligible wards must have
                    // `created_by = "agent"`; without this hook every
                    // scaffolded ward lazy-inserts as `user` and is skipped.
                    if let Some(observer) = self.ward_usage.as_ref() {
                        observer
                            .mark_created_agent(name, archetype.unwrap_or_default())
                            .await;
                    }
                    state
                } else {
                    self.ward_layout
                        .load(name)
                        .unwrap_or_else(|_| WardLayoutState {
                            context: None,
                            packet: json!({
                                "status":"unavailable",
                                "ward_id":name,
                                "diagnostic":{"code":"template_unavailable"}
                            }),
                        })
                };

                layout_state.packet["session_id"] = json!(ctx.session_id());
                layout_state.packet["ward_id"] = json!(name);
                ctx.set_state("ward_id".to_string(), json!(name));

                let planner_gate_pending = planning_gate_awaits_ward(ctx.as_ref());
                let planner_template_ready = layout_state.context.is_some()
                    && layout_state.packet.get("status").and_then(Value::as_str)
                        == Some("available");
                let planner_task = planner_template_ready
                    .then(|| start_planning_after_ward(ctx.as_ref(), name))
                    .flatten();
                if let Some(task) = planner_task.as_ref() {
                    let mut actions = ctx.actions();
                    actions.delegate = Some(DelegateAction {
                        agent_id: "planner-agent".to_string(),
                        task: task.clone(),
                        context: None,
                        wait_for_result: true,
                        max_iterations: None,
                        output_schema: None,
                        skills: Vec::new(),
                        capability_assignment: None,
                        planning_capability_catalog: ctx
                            .get_state("app:planning_capability_catalog"),
                        complexity: None,
                        mode: None,
                        parallel: false,
                        child_execution_id: None,
                    });
                    ctx.set_actions(actions);
                }
                // The bootstrap packet is immutable for this executor because
                // it is also embedded in the system instruction. A ward
                // switch takes effect for ordinary file tools immediately,
                // but template-dependent actions wait for the next root turn.

                // List files in the ward
                let files = self.list_ward_files(&ward_dir);

                // Read AGENTS.md if it exists
                let agents_md = self.read_agents_md(&ward_dir);

                tracing::info!("Ward switched to '{}' (created: {})", name, created);

                // Best-effort recall of ward-scoped knowledge
                let ward_knowledge = self.recall_ward_facts(name, &ctx).await;

                // Return result with __ward_changed__ marker for the executor.
                // Slim by contract (spec ward-slim AC1): only model-actionable
                // fields. The full template packet reaches the model through
                // executor state → next-turn system instruction; digests and
                // the layout projection are lint machinery, not model input.
                let mut result = json!({
                    "__ward_changed__": true,
                    "ward_id": name,
                    "action": if created { "created" } else { "switched" },
                    "ward_status": {
                        "status": layout_state.packet.get("status").cloned().unwrap_or(json!("unknown")),
                        // null when the packet declares no archetype
                        // (user-created/legacy wards, unavailable template) —
                        // never a fabricated concrete archetype.
                        "archetype": layout_state.packet.get("archetype").cloned().unwrap_or(Value::Null),
                    },
                    "files": files,
                    "file_count": files.len(),
                    "agents_md": agents_md,
                });

                if let Some(knowledge) = ward_knowledge {
                    result["ward_knowledge"] = knowledge;
                }

                if planner_task.is_some() {
                    result["planner"] = json!("started");
                } else if planner_gate_pending && !planner_template_ready {
                    // Template-unavailable stays fail-closed (#249): the gate
                    // keeps awaiting the ward and the next ward(use) retries.
                    result["planner"] = json!("pending-template");
                }

                Ok(result)
            }

            "list" => {
                let mut wards = Vec::new();

                if wards_root.exists()
                    && let Ok(entries) = std::fs::read_dir(&wards_root)
                {
                    for entry in entries.flatten() {
                        if entry.path().is_dir() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            // Skip hidden directories (.venv, .node_env)
                            if name.starts_with('.') {
                                continue;
                            }
                            let files = self.list_ward_files(&entry.path());
                            let description = self.ward_description(&entry.path());
                            wards.push(json!({
                                "name": name,
                                "files": files.len(),
                                "description": description,
                            }));
                        }
                    }
                }

                wards.sort_by(|a, b| {
                    a.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .cmp(b.get("name").and_then(|v| v.as_str()).unwrap_or(""))
                });

                Ok(json!({
                    "wards": wards,
                    "total": wards.len(),
                }))
            }

            "info" => {
                let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                    AgentError::Tool("Missing 'name' parameter for info".to_string())
                })?;
                Self::validate_ward_name(name)?;

                let ward_dir = wards_root.join(name);
                if !ward_dir.exists() {
                    return Ok(json!({
                        "found": false,
                        "name": name,
                        "message": "Ward not found",
                    }));
                }

                let files = self.list_ward_files(&ward_dir);
                let agents_md = self.read_agents_md(&ward_dir);

                Ok(json!({
                    "found": true,
                    "name": name,
                    "files": files,
                    "file_count": files.len(),
                    "agents_md": agents_md,
                }))
            }

            "lint" => {
                let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                    AgentError::Tool("Missing 'name' parameter for lint".to_string())
                })?;
                Self::validate_ward_name(name)?;
                let digest = match validate_template_context(ctx.as_ref(), name, true) {
                    Ok(digest) => digest,
                    Err(code) => return Ok(error_envelope(action, name, "", code)),
                };
                match self.ward_layout.lint(name, &digest) {
                    Ok(report) => Ok(ok_envelope(action, name, &digest, report)),
                    Err(code) => Ok(error_envelope(action, name, &digest, &code)),
                }
            }

            "search" => {
                let name = required_name(&args, action)?;
                Self::validate_ward_name(name)?;
                let digest = match validate_template_context(ctx.as_ref(), name, false) {
                    Ok(digest) => digest,
                    Err(code) => return Ok(error_envelope(action, name, "", code)),
                };
                let query = args.get("query").and_then(Value::as_str).unwrap_or("");
                let tags = parse_tags(&args)?;
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
                if let Err(code) = self.ward_layout.validate(name, &digest) {
                    return Ok(error_envelope(action, name, &digest, &code));
                }
                match search_markdown(&wards_root.join(name), query, &tags, limit) {
                    Ok(data) => Ok(ok_envelope(action, name, &digest, data)),
                    Err(code) => Ok(error_envelope(action, name, &digest, &code)),
                }
            }

            "dry_run" | "create_concept" => {
                let name = required_name(&args, action)?;
                Self::validate_ward_name(name)?;
                let digest = match validate_template_context(ctx.as_ref(), name, false) {
                    Ok(digest) => digest,
                    Err(code) => return Ok(error_envelope(action, name, "", code)),
                };
                if action == "dry_run"
                    && args.get("operation").and_then(Value::as_str) != Some("create_concept")
                {
                    return Ok(error_envelope(
                        action,
                        name,
                        &digest,
                        "unsupported_operation",
                    ));
                }
                let components = parse_components(&args)?;
                let apply = action == "create_concept";
                match self.ward_layout.concept(name, &components, &digest, apply) {
                    Ok(data) => Ok(ok_envelope(action, name, &digest, data)),
                    Err(code) => Ok(error_envelope(action, name, &digest, &code)),
                }
            }

            _ => Err(AgentError::Tool(format!("Unknown ward action: {}", action))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_primitives::event::EventActions;
    use agent_primitives::types::Content;
    use agent_primitives::{CallbackContext, ReadonlyContext};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tempfile::TempDir;

    // STUB: AC4
    #[test]
    fn ward_catalog_registers_one_canonical_wikilink() {
        let root = tempfile::tempdir().unwrap();
        WardTool::ensure_ward_catalog(root.path()).unwrap();
        WardTool::register_ward(root.path(), "financial-analysis").unwrap();
        WardTool::register_ward(root.path(), "financial-analysis").unwrap();

        let content = std::fs::read_to_string(root.path().join("index.md")).unwrap();
        let expected = "- [[financial-analysis/financial-analysis|Financial Analysis]]";
        assert_eq!(content.lines().filter(|line| *line == expected).count(), 1);
    }

    #[test]
    fn concurrent_catalog_registrations_preserve_every_ward_link() {
        let root = tempfile::tempdir().unwrap();
        WardTool::ensure_ward_catalog(root.path()).unwrap();
        let wards_root = std::sync::Arc::new(root.path().to_path_buf());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
        let threads = (0..16)
            .map(|index| {
                let wards_root = wards_root.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    WardTool::register_ward(&wards_root, &format!("ward-{index}")).unwrap();
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap();
        }

        let content = std::fs::read_to_string(root.path().join("index.md")).unwrap();
        for index in 0..16 {
            let expected = format!("- [[ward-{index}/ward-{index}|Ward {index}]]");
            assert_eq!(
                content.lines().filter(|line| *line == expected).count(),
                1,
                "{expected}"
            );
        }
    }

    struct TestFs {
        base: PathBuf,
    }

    struct TestLayout {
        base: PathBuf,
    }

    impl WardLayoutAccess for TestLayout {
        fn create(
            &self,
            ward: &str,
            archetype: Option<WardArchetypeId>,
        ) -> std::result::Result<WardLayoutState, String> {
            let path = self.base.join("wards").join(ward);
            std::fs::create_dir(&path).map_err(|error| error.to_string())?;
            std::fs::write(path.join("ward-conf.yaml"), "test")
                .map_err(|error| error.to_string())?;
            let mut state = self.load(ward)?;
            state.packet["archetype"] = json!(archetype.unwrap_or_default());
            Ok(state)
        }

        fn rollback_created(&self, ward: &str) -> std::result::Result<(), String> {
            std::fs::remove_dir_all(self.base.join("wards").join(ward))
                .map_err(|error| error.to_string())
        }

        fn load(&self, ward: &str) -> std::result::Result<WardLayoutState, String> {
            if !self
                .base
                .join("wards")
                .join(ward)
                .join("ward-conf.yaml")
                .is_file()
            {
                return Err("rebuild_required: ward-conf.yaml is missing".into());
            }
            Ok(WardLayoutState {
                context: Some("<ward-layout digest=\"test\">{}</ward-layout>".into()),
                packet: json!({"status": "available", "digest": "test"}),
            })
        }

        fn validate(&self, ward: &str, expected_digest: &str) -> std::result::Result<(), String> {
            self.load(ward)?;
            (expected_digest == "test")
                .then_some(())
                .ok_or_else(|| "template_stale".to_string())
        }

        fn lint(&self, ward: &str, expected_digest: &str) -> std::result::Result<Value, String> {
            self.validate(ward, expected_digest)?;
            Ok(json!({"valid": true, "findings": []}))
        }

        fn concept(
            &self,
            ward: &str,
            components: &[String],
            _expected_digest: &str,
            apply: bool,
        ) -> std::result::Result<Value, String> {
            self.load(ward)?;
            let path = components.join("/");
            if apply {
                std::fs::create_dir(self.base.join("wards").join(ward).join(&path))
                    .map_err(|error| error.to_string())?;
            }
            Ok(json!({"changes":[{"operation":"create","path":path,"size":0,"digest":"test"}]}))
        }
    }

    fn test_layout(base: &std::path::Path) -> Arc<dyn WardLayoutAccess> {
        Arc::new(TestLayout {
            base: base.to_path_buf(),
        })
    }

    struct CatalogBreakingLayout {
        inner: TestLayout,
    }

    impl WardLayoutAccess for CatalogBreakingLayout {
        fn create(
            &self,
            ward: &str,
            archetype: Option<WardArchetypeId>,
        ) -> std::result::Result<WardLayoutState, String> {
            let state = self.inner.create(ward, archetype)?;
            let catalog = self.inner.base.join("wards/index.md");
            std::fs::remove_file(&catalog).map_err(|error| error.to_string())?;
            std::fs::create_dir(&catalog).map_err(|error| error.to_string())?;
            Ok(state)
        }

        fn rollback_created(&self, ward: &str) -> std::result::Result<(), String> {
            self.inner.rollback_created(ward)
        }

        fn load(&self, ward: &str) -> std::result::Result<WardLayoutState, String> {
            self.inner.load(ward)
        }

        fn validate(&self, ward: &str, expected_digest: &str) -> std::result::Result<(), String> {
            self.inner.validate(ward, expected_digest)
        }

        fn lint(&self, ward: &str, expected_digest: &str) -> std::result::Result<Value, String> {
            self.inner.lint(ward, expected_digest)
        }

        fn concept(
            &self,
            ward: &str,
            components: &[String],
            expected_digest: &str,
            apply: bool,
        ) -> std::result::Result<Value, String> {
            self.inner.concept(ward, components, expected_digest, apply)
        }
    }

    struct GateContext {
        state: Mutex<HashMap<String, Value>>,
        actions: Mutex<EventActions>,
        content: Content,
    }

    impl GateContext {
        fn cold_graph() -> Self {
            Self {
                state: Mutex::new(HashMap::new()),
                actions: Mutex::new(EventActions::default()),
                content: Content::user(""),
            }
        }

        fn gated_cold_graph() -> Self {
            let context = Self::cold_graph();
            let mut state = context.state.lock().unwrap();
            state.insert(
                crate::tools::guards::PLANNING_GATE_STATE.to_string(),
                serde_json::to_value(crate::tools::guards::PlanningGate::awaiting_ward(
                    "Plan this request",
                ))
                .unwrap(),
            );
            state.insert(
                "app:planning_capability_catalog".to_string(),
                json!({"skills": [], "mcps": []}),
            );
            drop(state);
            context
        }

        fn delegated_cold_graph() -> Self {
            let context = Self::gated_cold_graph();
            context
                .state
                .lock()
                .unwrap()
                .insert("app:is_delegated".to_string(), json!(true));
            context
        }

        fn delegated_planner(ward: &str) -> Self {
            let context = Self::delegated_cold_graph();
            let mut state = context.state.lock().unwrap();
            state.insert("app:actor_kind".into(), json!("delegated_executor"));
            state.insert("app:agent_id".into(), json!("planner-agent"));
            state.insert("session_id".into(), json!("host-session"));
            state.insert("ward_id".into(), json!(ward));
            state.insert(
                "ward_template".into(),
                json!({
                    "status":"available", "session_id":"host-session", "ward_id":ward,
                    "root_context_id":"planner-test", "digest":"test", "projection":{}
                }),
            );
            state.insert("ward_template_context_id".into(), json!("planner-test"));
            drop(state);
            context
        }

        fn active_root(ward: &str) -> Self {
            let context = Self::cold_graph();
            let mut state = context.state.lock().unwrap();
            state.insert("app:actor_kind".into(), json!("root"));
            state.insert("session_id".into(), json!("test"));
            state.insert("ward_id".into(), json!(ward));
            state.insert(
                "ward_template".into(),
                json!({
                    "status":"available", "session_id":"test", "ward_id":ward,
                    "root_context_id":"root-test", "digest":"test", "projection":{}
                }),
            );
            state.insert("ward_template_context_id".into(), json!("root-test"));
            drop(state);
            context
        }
    }

    impl ReadonlyContext for GateContext {
        fn invocation_id(&self) -> &str {
            "test"
        }
        fn agent_name(&self) -> &str {
            "root"
        }
        fn user_id(&self) -> &str {
            "test"
        }
        fn app_name(&self) -> &str {
            "test"
        }
        fn session_id(&self) -> &str {
            "test"
        }
        fn branch(&self) -> &str {
            "test"
        }
        fn user_content(&self) -> &Content {
            &self.content
        }
    }

    impl CallbackContext for GateContext {
        fn get_state(&self, key: &str) -> Option<Value> {
            self.state.lock().ok()?.get(key).cloned()
        }

        fn set_state(&self, key: String, value: Value) {
            if let Ok(mut state) = self.state.lock() {
                state.insert(key, value);
            }
        }
    }

    impl ToolContext for GateContext {
        fn function_call_id(&self) -> String {
            "test".to_string()
        }
        fn actions(&self) -> EventActions {
            self.actions
                .lock()
                .map(|actions| actions.clone())
                .unwrap_or_default()
        }
        fn set_actions(&self, actions: EventActions) {
            if let Ok(mut current) = self.actions.lock() {
                *current = actions;
            }
        }
    }

    impl FileSystemContext for TestFs {
        fn conversation_dir(&self, _id: &str) -> Option<PathBuf> {
            None
        }
        fn outputs_dir(&self) -> Option<PathBuf> {
            None
        }
        fn skills_dir(&self) -> Option<PathBuf> {
            None
        }
        fn agents_dir(&self) -> Option<PathBuf> {
            None
        }
        fn agent_data_dir(&self, _id: &str) -> Option<PathBuf> {
            None
        }
        fn python_executable(&self) -> Option<PathBuf> {
            None
        }
        fn vault_path(&self) -> Option<PathBuf> {
            Some(self.base.clone())
        }
    }

    #[test]
    fn test_list_ward_files_empty() {
        let dir = TempDir::new().unwrap();
        let fs = Arc::new(TestFs {
            base: dir.path().to_path_buf(),
        });
        let tool = WardTool::new(fs, None, None, test_layout(dir.path()));
        let ward_dir = dir.path().join("wards").join("test");
        std::fs::create_dir_all(&ward_dir).unwrap();

        let files = tool.list_ward_files(&ward_dir);
        assert!(files.is_empty());
    }

    #[test]
    fn test_list_ward_files_with_content() {
        let dir = TempDir::new().unwrap();
        let fs = Arc::new(TestFs {
            base: dir.path().to_path_buf(),
        });
        let tool = WardTool::new(fs, None, None, test_layout(dir.path()));
        let ward_dir = dir.path().join("wards").join("test");
        std::fs::create_dir_all(&ward_dir).unwrap();

        // Create some files
        std::fs::write(ward_dir.join("app.js"), "console.log('hi')").unwrap();
        std::fs::write(ward_dir.join("readme.md"), "# Test").unwrap();
        std::fs::create_dir(ward_dir.join("src")).unwrap();

        // Create hidden file (should be excluded)
        std::fs::write(ward_dir.join(".hidden_file"), "{}").unwrap();

        let files = tool.list_ward_files(&ward_dir);
        assert_eq!(files.len(), 3);
        assert!(files.contains(&"app.js".to_string()));
        assert!(files.contains(&"readme.md".to_string()));
        assert!(files.contains(&"src/".to_string()));
    }

    #[test]
    fn test_ward_description_from_agents_md() {
        let dir = TempDir::new().unwrap();
        let fs = Arc::new(TestFs {
            base: dir.path().to_path_buf(),
        });
        let tool = WardTool::new(fs, None, None, test_layout(dir.path()));

        std::fs::write(
            dir.path().join("AGENTS.md"),
            "# My Project\n\n## Purpose\nStock tracker using yfinance\n\n## Structure\n",
        )
        .unwrap();

        let desc = tool.ward_description(dir.path());
        assert_eq!(desc, Some("Stock tracker using yfinance".to_string()));
    }

    #[test]
    fn test_ward_description_missing() {
        let dir = TempDir::new().unwrap();
        let fs = Arc::new(TestFs {
            base: dir.path().to_path_buf(),
        });
        let tool = WardTool::new(fs, None, None, test_layout(dir.path()));

        let desc = tool.ward_description(dir.path());
        assert!(desc.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn ward_catalog_rejects_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let wards = dir.path().join("wards");
        std::fs::create_dir(&wards).unwrap();
        let outside = dir.path().join("outside.md");
        std::fs::write(&outside, "keep").unwrap();
        symlink(&outside, wards.join("index.md")).unwrap();
        assert!(WardTool::ensure_ward_catalog(&wards).is_err());
        assert!(WardTool::register_ward(&wards, "safe").is_err());
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "keep");
    }

    #[cfg(unix)]
    #[test]
    fn catalog_open_rejects_a_raced_symlink_to_the_previously_checked_inode() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let index = dir.path().join("index.md");
        let moved = dir.path().join("moved.md");
        std::fs::write(&index, "# Wards\n").unwrap();
        let before = std::fs::symlink_metadata(&index).unwrap();
        std::fs::rename(&index, &moved).unwrap();
        symlink(&moved, &index).unwrap();

        assert!(open_catalog_for_update(&index, &before).is_err());
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), "# Wards\n");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn catalog_seed_stays_anchored_when_the_wards_path_is_replaced() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let wards = dir.path().join("wards");
        let moved = dir.path().join("moved-wards");
        let outside = dir.path().join("outside");
        std::fs::create_dir(&wards).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let opened = open_directory_nofollow(&wards).unwrap();
        std::fs::rename(&wards, &moved).unwrap();
        symlink(&outside, &wards).unwrap();

        ensure_ward_catalog_at(&opened).unwrap();
        assert!(moved.join("index.md").is_file());
        assert!(!outside.join("index.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn ward_catalog_rejects_hardlink_aliases() {
        let dir = TempDir::new().unwrap();
        let wards = dir.path().join("wards");
        std::fs::create_dir(&wards).unwrap();
        let outside = dir.path().join("outside.md");
        std::fs::write(&outside, "keep").unwrap();
        std::fs::hard_link(&outside, wards.join("index.md")).unwrap();

        assert!(WardTool::ensure_ward_catalog(&wards).is_err());
        assert!(WardTool::register_ward(&wards, "safe").is_err());
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "keep");
    }

    #[test]
    fn test_read_agents_md() {
        let dir = TempDir::new().unwrap();
        let fs = Arc::new(TestFs {
            base: dir.path().to_path_buf(),
        });
        let tool = WardTool::new(fs, None, None, test_layout(dir.path()));

        std::fs::write(dir.path().join("AGENTS.md"), "# My Project\n\nTest content").unwrap();

        let content = tool.read_agents_md(dir.path());
        assert!(content.is_some());
        assert!(content.unwrap().contains("# My Project"));
    }

    #[test]
    fn root_surface_keeps_concept_actions_drops_lint() {
        let dir = TempDir::new().unwrap();
        let tool = WardTool::for_root(
            Arc::new(TestFs {
                base: dir.path().to_path_buf(),
            }),
            None,
            None,
            test_layout(dir.path()),
        );

        let description = tool.description();
        for present in [
            "use:",
            "create:",
            "list:",
            "info:",
            "search:",
            "dry_run:",
            "create_concept:",
        ] {
            assert!(
                description.contains(present),
                "root surface missing {present}"
            );
        }
        assert!(
            !description.contains("lint"),
            "root surface must not advertise lint — the planner owns conformance"
        );

        let schema = tool.parameters_schema().expect("schema").to_string();
        assert!(
            schema.contains("create_concept"),
            "root keeps concept actions"
        );
        assert!(!schema.contains("lint"));
        let branches = tool.parameters_schema().unwrap()["oneOf"]
            .as_array()
            .unwrap()
            .len();
        assert_eq!(
            branches, 6,
            "root: named(use/info), create, list, search, dry_run, create_concept"
        );
    }

    #[test]
    fn planner_surface_keeps_lint_drops_concept_actions() {
        let dir = TempDir::new().unwrap();
        let tool = WardTool::for_planner(
            Arc::new(TestFs {
                base: dir.path().to_path_buf(),
            }),
            None,
            None,
            test_layout(dir.path()),
        );

        let description = tool.description();
        for present in ["use:", "create:", "list:", "info:", "search:", "lint:"] {
            assert!(
                description.contains(present),
                "planner surface missing {present}"
            );
        }
        for hidden in ["dry_run", "create_concept"] {
            assert!(
                !description.contains(hidden),
                "planner surface must not advertise {hidden} — guard rejects it (root_required)"
            );
        }
        let schema = tool.parameters_schema().expect("schema").to_string();
        assert!(schema.contains("lint"));
        assert!(!schema.contains("create_concept"));
    }

    #[test]
    fn subagent_surface_is_lifecycle_only() {
        let dir = TempDir::new().unwrap();
        let tool = WardTool::for_subagent(
            Arc::new(TestFs {
                base: dir.path().to_path_buf(),
            }),
            None,
            None,
            test_layout(dir.path()),
        );

        let description = tool.description();
        for lifecycle in ["use:", "create:", "list:", "info:", "search:"] {
            assert!(description.contains(lifecycle));
        }
        for template in ["lint", "dry_run", "create_concept"] {
            assert!(
                !description.contains(template),
                "subagent surface must not advertise {template}"
            );
        }
        let branches = tool.parameters_schema().unwrap()["oneOf"]
            .as_array()
            .unwrap()
            .len();
        assert_eq!(
            branches, 4,
            "subagent: named(use/info), create, list, search"
        );
    }

    #[test]
    fn full_surface_lists_all_actions() {
        let dir = TempDir::new().unwrap();
        let tool = WardTool::new(
            Arc::new(TestFs {
                base: dir.path().to_path_buf(),
            }),
            None,
            None,
            test_layout(dir.path()),
        );

        let description = tool.description();
        for action in [
            "use:",
            "create:",
            "list:",
            "info:",
            "search:",
            "lint:",
            "dry_run:",
            "create_concept:",
        ] {
            assert!(
                description.contains(action),
                "full surface missing {action}"
            );
        }
        let branches = tool.parameters_schema().unwrap()["oneOf"]
            .as_array()
            .unwrap()
            .len();
        assert_eq!(branches, 6, "full schema keeps all six branches");
        assert!(
            tool.parameters_schema()
                .unwrap()
                .to_string()
                .contains("lint")
        );
    }

    #[test]
    fn ward_schema_exposes_template_directed_actions_and_rejects_extra_fields() {
        let dir = TempDir::new().unwrap();
        let tool = WardTool::new(
            Arc::new(TestFs {
                base: dir.path().to_path_buf(),
            }),
            None,
            None,
            test_layout(dir.path()),
        );
        let schema = tool.parameters_schema().unwrap().to_string();
        for action in [
            "search",
            "lint",
            "dry_run",
            "create_concept",
            "coding",
            "generic",
        ] {
            assert!(schema.contains(action));
        }
        assert!(schema.contains("additionalProperties"));
    }

    #[tokio::test]
    async fn create_accepts_closed_archetype_and_invalid_value_has_no_effects() {
        let dir = TempDir::new().unwrap();
        let tool = WardTool::new(
            Arc::new(TestFs {
                base: dir.path().to_path_buf(),
            }),
            None,
            None,
            test_layout(dir.path()),
        );
        let ctx: Arc<dyn ToolContext> = Arc::new(GateContext::active_root("scratch"));

        let invalid = tool
            .execute(
                ctx.clone(),
                json!({"action":"create","name":"unsafe","archetype":"../coding"}),
            )
            .await
            .unwrap_err();
        assert!(invalid.to_string().contains("invalid archetype"));
        assert!(!dir.path().join("wards").exists());

        let created = tool
            .execute(
                ctx,
                json!({"action":"create","name":"compiler","archetype":"coding"}),
            )
            .await
            .unwrap();
        assert_eq!(created["ward_status"]["archetype"], "coding");
    }

    #[tokio::test]
    async fn cold_graph_ward_entry_starts_planner_once_with_actual_ward() {
        let dir = TempDir::new().unwrap();
        let tool = WardTool::new(
            Arc::new(TestFs {
                base: dir.path().to_path_buf(),
            }),
            None,
            None,
            test_layout(dir.path()),
        );
        let ctx: Arc<dyn ToolContext> = Arc::new(GateContext::gated_cold_graph());

        let result = tool
            .execute(
                ctx.clone(),
                json!({"action":"create","name":"creative-design"}),
            )
            .await
            .expect("ward creation succeeds");

        assert_eq!(result["planner"], "started");
        let action = ctx.actions().delegate.expect("planner action is emitted");
        assert_eq!(action.agent_id, "planner-agent");
        assert!(action.wait_for_result);
        assert!(!action.parallel);
        assert!(action.task.contains("Active ward:"));
        assert!(action.task.contains("creative-design"));
        assert_eq!(
            action.planning_capability_catalog,
            Some(json!({"skills": [], "mcps": []}))
        );

        let second = tool
            .execute(
                ctx.clone(),
                json!({"action":"use","name":"creative-design"}),
            )
            .await
            .expect("re-entering the ward succeeds");
        assert!(second.get("planner").is_none());
    }

    #[tokio::test]
    async fn ward_entry_result_is_slim() {
        let dir = TempDir::new().unwrap();
        let tool = WardTool::new(
            Arc::new(TestFs {
                base: dir.path().to_path_buf(),
            }),
            None,
            None,
            test_layout(dir.path()),
        );
        let ctx: Arc<dyn ToolContext> = Arc::new(GateContext::gated_cold_graph());

        let result = tool
            .execute(ctx, json!({"action":"create","name":"slim-check"}))
            .await
            .expect("ward creation succeeds");

        // Exact key set — nothing the model cannot act on
        let mut keys: Vec<&str> = result
            .as_object()
            .expect("result is an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        let mut expected = vec![
            "__ward_changed__",
            "ward_id",
            "action",
            "ward_status",
            "files",
            "file_count",
            "agents_md",
            "planner",
        ];
        expected.sort_unstable();
        assert_eq!(keys, expected);
        assert_eq!(result["ward_status"]["status"], "available");
        let rendered = result.to_string();
        assert!(!rendered.contains("projection"), "no layout projection");
        assert!(!rendered.contains("digest"), "no digests");
        assert!(!rendered.contains("recall_nudge"));
        assert!(!rendered.contains("planner_started"));
        assert!(!rendered.contains("planner_error"));
    }

    /// Records the requested recall limit and returns a canned envelope.
    struct RecordingFactStore {
        requested_limit: std::sync::Mutex<Vec<usize>>,
    }

    impl RecordingFactStore {
        fn envelope(limit: usize) -> Value {
            let results: Vec<Value> = (0..limit)
                .map(|i| json!({"content": format!("fact {i}"), "category": "correction"}))
                .collect();
            json!({
                "query": "ward",
                "results": results,
                "count": limit,
                "source": "memory_db",
                "prioritized": true,
            })
        }
    }

    #[async_trait::async_trait]
    impl MemoryFactStore for RecordingFactStore {
        async fn save_fact(
            &self,
            _agent_id: &str,
            _category: &str,
            _key: &str,
            _content: &str,
            _confidence: f64,
            _session_id: Option<&str>,
            _valid_from: Option<chrono::DateTime<chrono::Utc>>,
        ) -> std::result::Result<Value, String> {
            Ok(json!({"saved": true}))
        }

        async fn recall_facts(
            &self,
            _agent_id: &str,
            _query: &str,
            limit: usize,
        ) -> std::result::Result<Value, String> {
            Ok(Self::envelope(limit))
        }

        async fn recall_facts_prioritized(
            &self,
            _agent_id: &str,
            _query: &str,
            limit: usize,
            _as_of: Option<chrono::DateTime<chrono::Utc>>,
        ) -> std::result::Result<Value, String> {
            self.requested_limit.lock().unwrap().push(limit);
            Ok(Self::envelope(limit))
        }
    }

    #[tokio::test]
    async fn ward_entry_recall_is_trimmed_and_present() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(RecordingFactStore {
            requested_limit: std::sync::Mutex::new(Vec::new()),
        });
        let tool = WardTool::new(
            Arc::new(TestFs {
                base: dir.path().to_path_buf(),
            }),
            Some(store.clone()),
            None,
            test_layout(dir.path()),
        );
        let ctx: Arc<dyn ToolContext> = Arc::new(GateContext::gated_cold_graph());
        ctx.set_state("app:agent_id".to_string(), json!("root"));

        let result = tool
            .execute(ctx, json!({"action":"create","name":"knowing"}))
            .await
            .expect("ward creation succeeds");

        // The tool must request exactly 3 facts (spec AC3)
        assert_eq!(*store.requested_limit.lock().unwrap(), vec![3]);

        let knowledge = result["ward_knowledge"]
            .as_object()
            .expect("knowledge present");
        assert_eq!(
            knowledge["count"], 3,
            "count reports the trimmed result count"
        );
        assert_eq!(
            knowledge["results"].as_array().map(Vec::len),
            Some(3),
            "at most 3 facts ride the entry payload"
        );
        // Full key set = slim schema + ward_knowledge
        let mut keys: Vec<&str> = result
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        let mut expected = vec![
            "__ward_changed__",
            "ward_id",
            "action",
            "ward_status",
            "files",
            "file_count",
            "agents_md",
            "planner",
            "ward_knowledge",
        ];
        expected.sort_unstable();
        assert_eq!(keys, expected);
    }

    #[tokio::test]
    async fn agents_md_is_bounded() {
        let dir = TempDir::new().unwrap();
        let ward = dir.path().join("wards").join("verbose");
        std::fs::create_dir_all(&ward).unwrap();
        std::fs::write(ward.join("ward-conf.yaml"), "test").unwrap();
        // ~64 KiB of doctrine — double the cap
        let huge = "# Verbose Ward\n\n".to_string() + &"doctrine line\n".repeat(4_500);
        std::fs::write(ward.join(WARD_AGENTS_MD), &huge).unwrap();
        let tool = WardTool::new(
            Arc::new(TestFs {
                base: dir.path().to_path_buf(),
            }),
            None,
            None,
            test_layout(dir.path()),
        );
        let ctx: Arc<dyn ToolContext> = Arc::new(GateContext::active_root("verbose"));

        let result = tool
            .execute(ctx, json!({"action":"use","name":"verbose"}))
            .await
            .expect("ward entry succeeds");

        let agents_md = result["agents_md"].as_str().expect("agents_md present");
        assert!(
            agents_md.len() <= 32 * 1024 + 64,
            "agents_md must be bounded, got {} bytes",
            agents_md.len()
        );
        assert!(
            agents_md.contains("truncated at 32 KiB"),
            "oversized doctrine must carry a truncation marker"
        );
        assert!(agents_md.starts_with("# Verbose Ward"));
    }

    #[tokio::test]
    async fn unavailable_template_keeps_cold_graph_gated_until_retry_succeeds() {
        let dir = TempDir::new().unwrap();
        let ward = dir.path().join("wards").join("research");
        std::fs::create_dir_all(&ward).unwrap();
        let tool = WardTool::new(
            Arc::new(TestFs {
                base: dir.path().to_path_buf(),
            }),
            None,
            None,
            test_layout(dir.path()),
        );
        let ctx: Arc<dyn ToolContext> = Arc::new(GateContext::gated_cold_graph());

        let unavailable = tool
            .execute(ctx.clone(), json!({"action":"use","name":"research"}))
            .await
            .unwrap();
        assert_eq!(unavailable["planner"], "pending-template");
        assert!(planning_gate_awaits_ward(ctx.as_ref()));
        assert!(ctx.actions().delegate.is_none());

        std::fs::write(ward.join("ward-conf.yaml"), "test").unwrap();
        let retry = tool
            .execute(ctx.clone(), json!({"action":"use","name":"research"}))
            .await
            .unwrap();
        assert_eq!(retry["planner"], "started");
        assert!(!planning_gate_awaits_ward(ctx.as_ref()));
        assert_eq!(
            ctx.actions()
                .delegate
                .as_ref()
                .map(|action| action.agent_id.as_str()),
            Some("planner-agent")
        );
    }

    #[test]
    fn cold_graph_gate_allows_only_ward_establishment_and_safe_reads() {
        let ctx = GateContext::gated_cold_graph();

        for action in ["create", "use", "list", "info"] {
            assert!(
                !crate::tools::guards::planning_gate_blocks_tool(
                    &ctx,
                    "ward",
                    &json!({"action": action})
                ),
                "{action} should remain available while establishing a ward"
            );
        }
        for action in ["lint", "search", "dry_run", "create_concept"] {
            assert!(
                crate::tools::guards::planning_gate_blocks_tool(
                    &ctx,
                    "ward",
                    &json!({"action": action})
                ),
                "{action} must not bypass planning"
            );
        }
        assert!(crate::tools::guards::planning_gate_blocks_tool(
            &ctx,
            "blender_mcp__execute_code",
            &json!({"code": "mutate_scene()"})
        ));
        let delegated = GateContext::delegated_cold_graph();
        assert!(!crate::tools::guards::planning_gate_blocks_tool(
            &delegated,
            "shell",
            &json!({})
        ));
    }

    #[tokio::test]
    async fn catalog_failure_rolls_back_only_the_new_ward() {
        let dir = TempDir::new().unwrap();
        let wards = dir.path().join("wards");
        std::fs::create_dir_all(wards.join("existing")).unwrap();
        std::fs::write(wards.join("existing/keep.md"), "keep").unwrap();
        let tool = WardTool::new(
            Arc::new(TestFs {
                base: dir.path().to_path_buf(),
            }),
            None,
            None,
            Arc::new(CatalogBreakingLayout {
                inner: TestLayout {
                    base: dir.path().to_path_buf(),
                },
            }),
        );
        let ctx: Arc<dyn ToolContext> = Arc::new(GateContext::active_root("scratch"));

        let error = tool
            .execute(ctx, json!({"action":"create","name":"new-ward"}))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("rolled back"));
        assert!(!wards.join("new-ward").exists());
        assert_eq!(
            std::fs::read_to_string(wards.join("existing/keep.md")).unwrap(),
            "keep"
        );
    }

    #[tokio::test]
    async fn search_is_root_scoped_bounded_and_filters_exact_tags() {
        let dir = TempDir::new().unwrap();
        let ward = dir.path().join("wards").join("research");
        std::fs::create_dir_all(&ward).unwrap();
        std::fs::write(ward.join("ward-conf.yaml"), "test").unwrap();
        std::fs::write(
            ward.join("apple.md"),
            "---\ntitle: Apple valuation\ntags:\n  - equity\n  - apple\n---\n\nIntrinsic value research\n",
        ).unwrap();
        std::fs::write(ward.join("book.md"), "# Apple book\n").unwrap();
        let tool = WardTool::new(
            Arc::new(TestFs {
                base: dir.path().to_path_buf(),
            }),
            None,
            None,
            test_layout(dir.path()),
        );
        let ctx: Arc<dyn ToolContext> = Arc::new(GateContext::active_root("research"));

        let result = tool.execute(ctx, json!({
            "action":"search", "name":"research", "query":"value", "tags":["EQUITY"], "limit":10
        })).await.unwrap();

        assert_eq!(result["ok"], true);
        assert_eq!(result["data"]["results"].as_array().unwrap().len(), 1);
        assert_eq!(result["data"]["results"][0]["path"], "apple.md");
    }

    #[tokio::test]
    async fn template_actions_reject_delegated_context() {
        let dir = TempDir::new().unwrap();
        let ward = dir.path().join("wards").join("research");
        std::fs::create_dir_all(&ward).unwrap();
        std::fs::write(ward.join("ward-conf.yaml"), "test").unwrap();
        let tool = WardTool::new(
            Arc::new(TestFs {
                base: dir.path().to_path_buf(),
            }),
            None,
            None,
            test_layout(dir.path()),
        );
        let ctx: Arc<dyn ToolContext> = Arc::new(GateContext::delegated_cold_graph());
        let result = tool
            .execute(ctx, json!({"action":"lint","name":"research"}))
            .await
            .unwrap();
        assert_eq!(result["ok"], false);
        assert_eq!(result["error"]["code"], "root_required");
    }

    #[tokio::test]
    async fn planner_with_matching_host_packet_can_lint_selected_ward() {
        let dir = TempDir::new().unwrap();
        let ward = dir.path().join("wards").join("research");
        std::fs::create_dir_all(&ward).unwrap();
        std::fs::write(ward.join("ward-conf.yaml"), "test").unwrap();
        let tool = WardTool::new(
            Arc::new(TestFs {
                base: dir.path().to_path_buf(),
            }),
            None,
            None,
            test_layout(dir.path()),
        );
        let ctx: Arc<dyn ToolContext> = Arc::new(GateContext::delegated_planner("research"));

        let lint = tool
            .execute(ctx.clone(), json!({"action":"lint","name":"research"}))
            .await
            .unwrap();
        assert_eq!(lint["ok"], true);
        assert_eq!(lint["template_digest"], "test");

        for args in [
            json!({"action":"search","name":"research","query":"secret"}),
            json!({"action":"dry_run","name":"research"}),
            json!({"action":"create_concept","name":"research"}),
        ] {
            let result = tool.execute(ctx.clone(), args).await.unwrap();
            assert_eq!(result["ok"], false);
            assert_eq!(result["error"]["code"], "root_required");
        }

        let wrong_ward = tool
            .execute(ctx, json!({"action":"lint","name":"other"}))
            .await
            .unwrap();
        assert_eq!(wrong_ward["ok"], false);
        assert_eq!(wrong_ward["error"]["code"], "template_stale");
    }

    #[tokio::test]
    async fn planner_lint_rejects_a_packet_for_another_host_session() {
        let dir = TempDir::new().unwrap();
        let ward = dir.path().join("wards").join("research");
        std::fs::create_dir_all(&ward).unwrap();
        std::fs::write(ward.join("ward-conf.yaml"), "test").unwrap();
        let tool = WardTool::new(
            Arc::new(TestFs {
                base: dir.path().to_path_buf(),
            }),
            None,
            None,
            test_layout(dir.path()),
        );
        let context = GateContext::delegated_planner("research");
        context
            .state
            .lock()
            .unwrap()
            .insert("session_id".into(), json!("other-host-session"));
        let ctx: Arc<dyn ToolContext> = Arc::new(context);

        let lint = tool
            .execute(ctx, json!({"action":"lint","name":"research"}))
            .await
            .unwrap();
        assert_eq!(lint["ok"], false);
        assert_eq!(lint["error"]["code"], "template_stale");
    }

    #[tokio::test]
    async fn planner_cannot_switch_ward_before_writing() {
        let dir = TempDir::new().unwrap();
        for ward in ["research", "sibling"] {
            let path = dir.path().join("wards").join(ward);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("ward-conf.yaml"), "test").unwrap();
        }
        let fs: Arc<dyn FileSystemContext> = Arc::new(TestFs {
            base: dir.path().to_path_buf(),
        });
        let tool = WardTool::new(fs.clone(), None, None, test_layout(dir.path()));
        let ctx: Arc<dyn ToolContext> = Arc::new(GateContext::delegated_planner("research"));

        let error = tool
            .execute(ctx.clone(), json!({"action":"use","name":"sibling"}))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("planner_ward_locked"));
        assert_eq!(ctx.get_state("ward_id"), Some(json!("research")));

        let writer = crate::tools::WriteFileTool::new(fs);
        writer
            .execute(
                ctx,
                json!({
                    "path":".zbot/specs/locked/spec.md",
                    "content":"# Locked to selected ward\n"
                }),
            )
            .await
            .unwrap();
        assert!(
            dir.path()
                .join("wards/research/.zbot/specs/locked/spec.md")
                .is_file()
        );
        assert!(
            !dir.path()
                .join("wards/sibling/.zbot/specs/locked/spec.md")
                .exists()
        );
    }

    #[tokio::test]
    async fn ward_switch_does_not_replace_the_bootstrap_template_packet() {
        let dir = TempDir::new().unwrap();
        for ward in ["research", "other"] {
            let path = dir.path().join("wards").join(ward);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("ward-conf.yaml"), "test").unwrap();
        }
        std::fs::write(dir.path().join("wards/index.md"), "# Wards\n").unwrap();
        let tool = WardTool::new(
            Arc::new(TestFs {
                base: dir.path().to_path_buf(),
            }),
            None,
            None,
            test_layout(dir.path()),
        );
        let ctx: Arc<dyn ToolContext> = Arc::new(GateContext::active_root("research"));

        tool.execute(ctx.clone(), json!({"action":"use","name":"other"}))
            .await
            .unwrap();

        assert_eq!(
            ctx.get_state("ward_template").unwrap()["ward_id"],
            "research"
        );
        let stale = tool
            .execute(ctx, json!({"action":"lint","name":"other"}))
            .await
            .unwrap();
        assert_eq!(stale["error"]["code"], "template_stale");
    }

    #[cfg(unix)]
    #[test]
    fn search_rejects_a_symlinked_ward_root() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("wards")).unwrap();
        std::fs::write(outside.path().join("secret.md"), "secret").unwrap();
        symlink(outside.path(), dir.path().join("wards/research")).unwrap();

        assert_eq!(
            search_markdown(&dir.path().join("wards/research"), "secret", &[], 10).unwrap_err(),
            "ward_unavailable"
        );
    }
}
