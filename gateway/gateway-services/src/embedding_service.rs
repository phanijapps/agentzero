//! # Embedding Service
//!
//! Phase 1 of the embedding backend selection feature.
//!
//! `EmbeddingService` owns the live `Arc<dyn EmbeddingClient>` that the rest of
//! the daemon uses. It supports:
//!
//! - Loading configuration from `config/settings.json` (optional `embeddings`
//!   section)
//! - Tracking the dimension that the sqlite-vec indexes are built at via an
//!   atomic marker file `data/.embedding-state`
//! - Atomically swapping between the internal (`fastembed`) backend and an
//!   Ollama-hosted backend via `arc_swap::ArcSwap`
//! - Reporting health for UI/observability
//!
//! The service does **not** itself own the knowledge database; reindex is
//! driven from outside via [`EmbeddingService::ensure_indexed`] (async) and
//! [`EmbeddingService::ensure_indexed_blocking`] (sync helper invoked at boot).

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use agent_runtime::llm::{
    EmbeddingClient, EmbeddingError, LocalEmbeddingClient, OpenAiEmbeddingClient,
};
use arc_swap::ArcSwap;
use async_trait::async_trait;
use chrono::Utc;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::ollama_client::OllamaClient;
use agent_primitives::vault_paths::SharedVaultPaths;

// ============================================================================
// Public config / health types
// ============================================================================

/// Backend selector.
///
/// `Unconfigured` is the boot-time default when `settings.json` has no
/// `embeddings` section. In this state `build_client` returns a no-op
/// client so the heavy internal BGE ONNX (~130MB) is never lazy-loaded
/// just because a user hasn't yet chosen a backend. The first
/// `reconfigure` from the UI flips the state to Internal or Ollama.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingBackend {
    #[default]
    Unconfigured,
    Internal,
    Ollama,
}

impl EmbeddingBackend {
    /// Stable wire-name for HTTP/UI surfaces. Always one of
    /// `"unconfigured"`, `"internal"`, or `"ollama"`. Distinct from the
    /// model identifier.
    pub fn as_str(&self) -> &'static str {
        match self {
            EmbeddingBackend::Unconfigured => "unconfigured",
            EmbeddingBackend::Internal => "internal",
            EmbeddingBackend::Ollama => "ollama",
        }
    }
}

/// Ollama connection config (only used when `backend == Ollama`).
///
/// `dimensions` mirrors the embedding width that `model` produces. It lives
/// on this struct (rather than only on [`EmbeddingConfig`]) because the new
/// on-disk shape places it inside the ollama block — the user picks a model
/// and its dimension together, so the two are logically bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaConfig {
    pub base_url: String,
    pub model: String,
    pub dimensions: usize,
}

fn default_ollama_url() -> String {
    "http://localhost:11434".to_string()
}

/// Persisted embedding configuration, mirroring the optional
/// `embeddings` section of `settings.json`.
///
/// ## Wire format (on disk)
///
/// ```json
/// "embeddings": {
///   "internal": true,
///   "ollama": { "url": "http://localhost:11434", "model": "nomic-embed-text", "dimensions": 768 }
/// }
/// ```
///
/// `internal: true` selects the built-in BGE-small-en-v1.5 (384-d). The
/// `ollama` block is preserved across toggles so switching back to Ollama
/// keeps the user's URL / model choice without having to retype it.
///
/// A custom Deserialize also accepts the **legacy** shape
/// `{"backend": "internal|ollama", "dimensions": N, "ollama": {"base_url", "model"}}`
/// so settings.json files written by earlier builds migrate transparently
/// on first load. Serialize always emits the new shape.
///
/// ## Internal representation
///
/// `backend`, top-level `dimensions`, and `ollama` remain as structured
/// fields because the rest of the service (client construction, reindex
/// trigger, health loop) already dispatches on them. `dimensions` is the
/// effective width: 384 for Internal, `ollama.dimensions` for Ollama,
/// 0 for Unconfigured. The wire format derives these during serialize and
/// rehydrates them during deserialize — keeping the in-process types
/// stable while presenting the shape the user actually wants to edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingConfig {
    pub backend: EmbeddingBackend,
    pub dimensions: usize,
    pub ollama: Option<OllamaConfig>,
}

/// Default dim for the internal BGE-small backend.
const INTERNAL_BGE_DIM: usize = 384;

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            backend: EmbeddingBackend::Unconfigured,
            dimensions: 0,
            ollama: None,
        }
    }
}

// ----------------------------------------------------------------------------
// Custom serde for the on-disk shape.
// See the docstring on `EmbeddingConfig` for the format rationale.
// ----------------------------------------------------------------------------

/// The on-wire form of the ollama block for the new schema.
#[derive(Debug, Serialize, Deserialize)]
struct WireOllama {
    /// New schema key for the base URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    /// Legacy schema key. Accepted on load; never emitted on save.
    #[serde(default, skip_serializing)]
    base_url: Option<String>,
    #[serde(default)]
    model: String,
    /// Present in new schema; absent in legacy (where it lived at the top level).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
}

/// The on-wire form of the full embeddings section. Accepts both the new
/// (`internal` + nested dims) and legacy (`backend` + top-level dims) shapes.
#[derive(Debug, Serialize, Deserialize)]
struct WireEmbeddings {
    /// New: boolean switch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    internal: Option<bool>,
    /// Legacy: string enum. Accepted on load; never emitted on save.
    #[serde(default, skip_serializing)]
    backend: Option<String>,
    /// Legacy: top-level dim. Accepted on load; never emitted on save.
    #[serde(default, skip_serializing)]
    dimensions: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ollama: Option<WireOllama>,
}

impl Serialize for EmbeddingConfig {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let internal_flag = matches!(
            self.backend,
            EmbeddingBackend::Internal | EmbeddingBackend::Unconfigured
        );
        let ollama_wire = self.ollama.as_ref().map(|o| WireOllama {
            url: Some(o.base_url.clone()),
            base_url: None,
            model: o.model.clone(),
            dimensions: Some(o.dimensions),
        });
        let wire = WireEmbeddings {
            internal: Some(internal_flag),
            backend: None,
            dimensions: None,
            ollama: ollama_wire,
        };
        wire.serialize(s)
    }
}

impl<'de> Deserialize<'de> for EmbeddingConfig {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wire = WireEmbeddings::deserialize(d)?;

        // Resolve the ollama block if it's present, combining new-schema
        // and legacy-schema fields.
        let ollama_resolved = wire.ollama.as_ref().map(|o| OllamaConfig {
            base_url: o
                .url
                .clone()
                .or_else(|| o.base_url.clone())
                .unwrap_or_else(default_ollama_url),
            model: o.model.clone(),
            // Prefer nested dim (new shape); fall back to top-level (legacy).
            dimensions: o.dimensions.or(wire.dimensions).unwrap_or(0),
        });

        // Determine backend.
        let backend = if let Some(flag) = wire.internal {
            if flag {
                EmbeddingBackend::Internal
            } else if ollama_resolved.is_some() {
                EmbeddingBackend::Ollama
            } else {
                EmbeddingBackend::Unconfigured
            }
        } else if let Some(legacy) = wire.backend.as_deref() {
            match legacy {
                "internal" => EmbeddingBackend::Internal,
                "ollama" => {
                    if ollama_resolved.is_some() {
                        EmbeddingBackend::Ollama
                    } else {
                        EmbeddingBackend::Unconfigured
                    }
                }
                _ => EmbeddingBackend::Unconfigured,
            }
        } else {
            EmbeddingBackend::Unconfigured
        };

        let dimensions = match backend {
            EmbeddingBackend::Internal => INTERNAL_BGE_DIM,
            EmbeddingBackend::Ollama => ollama_resolved.as_ref().map(|o| o.dimensions).unwrap_or(0),
            EmbeddingBackend::Unconfigured => 0,
        };

        Ok(Self {
            backend,
            dimensions,
            ollama: ollama_resolved,
        })
    }
}

/// Health reported to the UI / HTTP layer.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Health {
    Ready,
    Reindexing {
        table: String,
        current: usize,
        total: usize,
    },
    Pulling {
        mb_done: u64,
        mb_total: u64,
    },
    OllamaUnreachable,
    ModelMissing,
    Misconfigured(String),
}

// ============================================================================
// Curated dropdown
// ============================================================================

/// Entry in the curated Ollama model list exposed via `/api/embeddings/models`.
#[derive(Debug, Clone, Serialize)]
pub struct CuratedModel {
    pub tag: &'static str,
    pub dim: usize,
    pub size_mb: u32,
    pub mteb: u32,
}

/// The six curated Ollama embedding models.
pub const CURATED_MODELS: &[CuratedModel] = &[
    CuratedModel {
        tag: "snowflake-arctic-embed:s",
        dim: 384,
        size_mb: 130,
        mteb: 57,
    },
    CuratedModel {
        tag: "nomic-embed-text",
        dim: 768,
        size_mb: 274,
        mteb: 62,
    },
    CuratedModel {
        tag: "mxbai-embed-large",
        dim: 1024,
        size_mb: 670,
        mteb: 65,
    },
    CuratedModel {
        tag: "bge-large",
        dim: 1024,
        size_mb: 670,
        mteb: 64,
    },
    CuratedModel {
        tag: "bge-m3",
        dim: 1024,
        size_mb: 1200,
        mteb: 63,
    },
    CuratedModel {
        tag: "snowflake-arctic-embed",
        dim: 1024,
        size_mb: 670,
        mteb: 63,
    },
];

/// Look up a curated model by Ollama tag.
#[must_use]
pub fn curated_lookup(tag: &str) -> Option<&'static CuratedModel> {
    CURATED_MODELS.iter().find(|m| m.tag == tag)
}

// ============================================================================
// Marker file (data/.embedding-state)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Marker {
    pub backend: EmbeddingBackend,
    pub model: String,
    pub dim: usize,
    pub indexed_at: String,
}

impl Marker {
    fn parse(text: &str) -> Option<Self> {
        let mut backend: Option<EmbeddingBackend> = None;
        let mut model: Option<String> = None;
        let mut dim: Option<usize> = None;
        let mut indexed_at: Option<String> = None;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                match k.trim() {
                    "backend" => {
                        backend = match v.trim() {
                            "internal" => Some(EmbeddingBackend::Internal),
                            "ollama" => Some(EmbeddingBackend::Ollama),
                            _ => None,
                        };
                    }
                    "model" => model = Some(v.trim().to_string()),
                    "dim" => dim = v.trim().parse().ok(),
                    "indexed_at" => indexed_at = Some(v.trim().to_string()),
                    _ => {}
                }
            }
        }
        Some(Self {
            backend: backend?,
            model: model?,
            dim: dim?,
            indexed_at: indexed_at.unwrap_or_default(),
        })
    }

    fn render(&self) -> String {
        let backend_str = match self.backend {
            EmbeddingBackend::Unconfigured => "unconfigured",
            EmbeddingBackend::Internal => "internal",
            EmbeddingBackend::Ollama => "ollama",
        };
        format!(
            "backend={}\nmodel={}\ndim={}\nindexed_at={}\n",
            backend_str, self.model, self.dim, self.indexed_at
        )
    }
}

fn marker_path(paths: &SharedVaultPaths) -> PathBuf {
    paths.data_dir().join(".embedding-state")
}

pub(crate) fn read_marker(paths: &SharedVaultPaths) -> Option<Marker> {
    let p = marker_path(paths);
    let text = fs::read_to_string(p).ok()?;
    Marker::parse(&text)
}

/// Atomic marker write via temp + rename.
pub(crate) fn write_marker(paths: &SharedVaultPaths, marker: &Marker) -> Result<(), String> {
    let final_path = marker_path(paths);
    let parent = final_path.parent().ok_or("marker has no parent")?;
    fs::create_dir_all(parent).map_err(|e| format!("create data dir: {e}"))?;
    let tmp_path = parent.join(".embedding-state.tmp");
    {
        let mut f = fs::File::create(&tmp_path).map_err(|e| format!("create temp marker: {e}"))?;
        f.write_all(marker.render().as_bytes())
            .map_err(|e| format!("write temp marker: {e}"))?;
        f.sync_all()
            .map_err(|e| format!("fsync temp marker: {e}"))?;
    }
    fs::rename(&tmp_path, &final_path).map_err(|e| format!("rename marker: {e}"))?;
    Ok(())
}

// ============================================================================
// Service
// ============================================================================

/// Settings snapshot (internal — decoupled from `SettingsService`).
fn read_config_from_settings_json(paths: &SharedVaultPaths) -> EmbeddingConfig {
    let Ok(text) = fs::read_to_string(paths.settings()) else {
        return EmbeddingConfig::default();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return EmbeddingConfig::default();
    };
    let Some(section) = json.get("embeddings") else {
        return EmbeddingConfig::default();
    };
    serde_json::from_value(section.clone()).unwrap_or_default()
}

/// Reconstruct an `EmbeddingConfig` from the on-disk `.embedding-state`
/// marker alone. Returns `None` when no marker exists or the recorded
/// backend is `Unconfigured` (nothing useful to recover).
///
/// The marker doesn't record the Ollama base URL (only backend/model/dim),
/// so Ollama recovery assumes the default URL. Users who used a non-default
/// URL get a reachable default; they can re-edit the URL in Settings if
/// needed. The alternative — leaving them fully Unconfigured — is worse.
fn recover_from_marker(paths: &SharedVaultPaths) -> Option<EmbeddingConfig> {
    let marker = read_marker(paths)?;
    match marker.backend {
        EmbeddingBackend::Unconfigured => None,
        EmbeddingBackend::Internal => Some(EmbeddingConfig {
            backend: EmbeddingBackend::Internal,
            dimensions: INTERNAL_BGE_DIM,
            ollama: None,
        }),
        EmbeddingBackend::Ollama => Some(EmbeddingConfig {
            backend: EmbeddingBackend::Ollama,
            dimensions: marker.dim,
            ollama: Some(OllamaConfig {
                base_url: default_ollama_url(),
                model: marker.model.clone(),
                dimensions: marker.dim,
            }),
        }),
    }
}

/// Write the `embeddings` section to `settings.json`, merging with any
/// existing top-level keys. Used by the boot marker-recovery path, which
/// runs before `EmbeddingService` is fully constructed (so it can't use
/// the instance method).
fn persist_settings_raw(paths: &SharedVaultPaths, cfg: &EmbeddingConfig) -> Result<(), String> {
    let path = paths.settings();
    let parent = path.parent().ok_or("settings has no parent")?;
    fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
    let current: serde_json::Value = if path.exists() {
        let text = fs::read_to_string(&path).map_err(|e| format!("read settings: {e}"))?;
        serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    let mut current = current;
    current["embeddings"] =
        serde_json::to_value(cfg).map_err(|e| format!("serialize embeddings: {e}"))?;
    let tmp = parent.join("settings.json.tmp");
    {
        let mut f = fs::File::create(&tmp).map_err(|e| format!("create temp: {e}"))?;
        let pretty = serde_json::to_string_pretty(&current).map_err(|e| format!("pretty: {e}"))?;
        f.write_all(pretty.as_bytes())
            .map_err(|e| format!("write temp: {e}"))?;
        f.sync_all().map_err(|e| format!("fsync: {e}"))?;
    }
    fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))?;
    Ok(())
}

/// True iff `settings.json` exists and carries an `embeddings` section.
/// Used by the fresh-install detector so we only self-heal when the user
/// has never touched embedding config — existing installs that
/// deliberately left the section off keep their Unconfigured state.
fn settings_json_has_embeddings(paths: &SharedVaultPaths) -> bool {
    let Ok(text) = fs::read_to_string(paths.settings()) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    json.get("embeddings").is_some()
}

/// Default self-healing config installed on fresh vaults. Internal BGE-small
/// is bundled into the daemon binary — no external dependency, no network,
/// safe to activate automatically.
fn fresh_install_default() -> EmbeddingConfig {
    EmbeddingConfig {
        backend: EmbeddingBackend::Internal,
        dimensions: INTERNAL_BGE_DIM,
        ollama: None,
    }
}

/// Build an `EmbeddingClient` for the given config (no network IO performed
/// here — this is a pure construction step).
fn build_client(cfg: &EmbeddingConfig) -> Result<Arc<dyn EmbeddingClient>, String> {
    match cfg.backend {
        EmbeddingBackend::Unconfigured => {
            // No backend chosen yet — return a no-op client so the heavy
            // internal BGE ONNX is NOT lazy-loaded on first recall just
            // because the user hasn't picked a backend yet. Embedding
            // attempts error with a clear message; recall paths treat
            // `None` embeddings as "skip vector search".
            Ok(Arc::new(NoopEmbeddingClient))
        }
        EmbeddingBackend::Internal => {
            // Default internal model is BGE-small (384d, ~130MB) via
            // `LocalEmbeddingClient::default()`.
            let client = LocalEmbeddingClient::new();
            Ok(Arc::new(client))
        }
        EmbeddingBackend::Ollama => {
            let ollama = cfg.ollama.as_ref().ok_or_else(|| {
                "ollama backend selected but no ollama config present".to_string()
            })?;
            if ollama.dimensions == 0 {
                return Err(
                    "ollama config missing dimensions — must be a positive integer".to_string(),
                );
            }
            // Ollama exposes an OpenAI-compatible surface at /v1/embeddings.
            // The curated list is advisory (shown in the UI as suggestions);
            // any model the user can name on their Ollama instance is
            // accepted. Dim correctness is verified by `probe_dimensions`
            // after swap — a mismatch surfaces as Health::Misconfigured
            // rather than a silent corruption of the vec0 indexes.
            let base = format!("{}/v1", ollama.base_url.trim_end_matches('/'));
            let client = OpenAiEmbeddingClient::with_provider_type(
                base,
                String::new(),
                ollama.model.clone(),
                ollama.dimensions,
                "ollama".to_string(),
            );
            Ok(Arc::new(client))
        }
    }
}

/// Embed a single short probe string and assert the returned vector has the
/// declared dimension. Replaces the old hardcoded curated-list dim lookup:
/// any model the user can name is allowed, but a wrong dim would silently
/// corrupt the vec0 indexes (built at a fixed width), so we verify up front.
///
/// On a mismatch or upstream error, returns a human-readable reason the
/// caller surfaces as `Health::Misconfigured`.
async fn probe_dimensions(client: &dyn EmbeddingClient, declared: usize) -> Result<(), String> {
    let result = client
        .embed(&["agentzero dim probe"])
        .await
        .map_err(|e| format!("probe embed failed: {e}"))?;
    let actual = result
        .first()
        .map(|v| v.len())
        .ok_or_else(|| "probe returned no vectors".to_string())?;
    if actual != declared {
        return Err(format!(
            "model returned vectors of dimension {actual}, but config declared {declared}. \
             Update the dimension in Settings or choose a model whose width matches."
        ));
    }
    Ok(())
}

/// Mutable state tracked under the `RwLock`.
struct EmbeddingState {
    config: EmbeddingConfig,
    dimensions: usize,
    indexed_dim: usize,
    needs_reindex: bool,
    health: Health,
}

/// Sized wrapper so we can store the client in an `ArcSwap` (which requires
/// `RefCnt` / `Sized`).
#[derive(Clone)]
struct ClientHandle {
    inner: Arc<dyn EmbeddingClient>,
}

/// Central embedding service. Cheap to clone via `Arc`.
pub struct EmbeddingService {
    swap: ArcSwap<ClientHandle>,
    state: RwLock<EmbeddingState>,
    paths: SharedVaultPaths,
    // Serializes concurrent `reconfigure` calls.
    sem: tokio::sync::Semaphore,
}

impl EmbeddingService {
    /// Construct the service from `config/settings.json` + `.embedding-state`.
    ///
    /// # Errors
    ///
    /// Returns an error if the client cannot be built for the persisted
    /// configuration. Callers may treat this as non-fatal and fall back to
    /// defaults.
    pub fn from_config(paths: SharedVaultPaths) -> Result<Self, String> {
        // Two distinct self-heal paths share this entry point:
        //
        // 1. Fresh install — neither `settings.json` has an `embeddings`
        //    section nor does the `.embedding-state` marker exist. Brand-new
        //    vault: auto-activate internal BGE-small and persist the choice
        //    so the next boot takes the normal path. Existing installs that
        //    deliberately want Unconfigured will also have the marker, so
        //    they aren't false-positives here.
        //
        // 2. Marker recovery — `settings.json` parsed to Unconfigured but a
        //    marker from a previous run records a real backend. Promote the
        //    marker back into `settings.json` so the selection survives the
        //    class of bug where a writer wiped the section before the
        //    strip-on-save fix landed.
        //
        // Orphan reindex tables are dropped lazily when the first reindex
        // runs; nothing to do here since we don't own the DB handle.
        let has_section = settings_json_has_embeddings(&paths);
        let has_marker = marker_path(&paths).exists();
        let self_healed_fresh = !has_section && !has_marker;

        let mut cfg = if self_healed_fresh {
            fresh_install_default()
        } else {
            read_config_from_settings_json(&paths)
        };

        let mut recovered_from_marker = false;
        if !self_healed_fresh && matches!(cfg.backend, EmbeddingBackend::Unconfigured) {
            if let Some(recovered) = recover_from_marker(&paths) {
                tracing::warn!(
                    recovered_backend = %recovered.backend.as_str(),
                    "embeddings: settings.json missing, recovered from .embedding-state marker"
                );
                cfg = recovered;
                recovered_from_marker = true;
                if let Err(e) = persist_settings_raw(&paths, &cfg) {
                    tracing::warn!(error = %e, "embeddings: marker-recovery write-back failed");
                }
            }
        }

        let service = Self::with_config(paths, cfg.clone())?;

        if self_healed_fresh {
            match service.persist_settings(&cfg) {
                Ok(()) => tracing::info!(
                    "Fresh vault detected — auto-activated internal BGE-small \
                     (384d) embedding backend. Change via Settings > Advanced \
                     > Embeddings."
                ),
                Err(e) => tracing::warn!(
                    "Self-healed embedding config but failed to persist: {}. \
                     Next boot will re-apply the default.",
                    e
                ),
            }
        } else if recovered_from_marker {
            tracing::info!(
                backend = %cfg.backend.as_str(),
                "embeddings: marker-based recovery completed"
            );
        }

        Ok(service)
    }

    /// Construct with an explicit config (test helper / advanced use).
    ///
    /// # Errors
    ///
    /// Returns an error if the client for `cfg` cannot be built.
    pub fn with_config(paths: SharedVaultPaths, cfg: EmbeddingConfig) -> Result<Self, String> {
        let client = build_client(&cfg)?;
        let marker = read_marker(&paths);
        let indexed_dim = marker.as_ref().map(|m| m.dim).unwrap_or(cfg.dimensions);
        let needs_reindex = marker
            .as_ref()
            .is_none_or(|m| m.dim != cfg.dimensions || marker_model(m) != config_model(&cfg));

        let health = Health::Ready;
        let state = EmbeddingState {
            config: cfg.clone(),
            dimensions: cfg.dimensions,
            indexed_dim,
            needs_reindex,
            health,
        };

        Ok(Self {
            swap: ArcSwap::new(Arc::new(ClientHandle { inner: client })),
            state: RwLock::new(state),
            paths,
            sem: tokio::sync::Semaphore::new(1),
        })
    }

    /// Hot path — return the currently-live client.
    #[must_use]
    pub fn client(&self) -> Arc<dyn EmbeddingClient> {
        self.swap.load_full().inner.clone()
    }

    /// Dimension of the live client.
    #[must_use]
    pub fn dimensions(&self) -> usize {
        self.state.read().dimensions
    }

    /// True if the live dimension differs from the marker dimension — reindex
    /// required before embedding-based recall is trustworthy.
    #[must_use]
    pub fn needs_reindex(&self) -> bool {
        self.state.read().needs_reindex
    }

    /// Current health snapshot.
    #[must_use]
    pub fn health(&self) -> Health {
        self.state.read().health.clone()
    }

    /// Apply a new config. Serialized via per-process semaphore.
    ///
    /// `on_progress` is called for each `Health` transition so that SSE
    /// consumers can stream progress. The caller is responsible for
    /// forwarding reindex to [`Self::ensure_indexed`] after this returns.
    ///
    /// # Errors
    ///
    /// Returns an error if the new client cannot be built or validated.
    pub async fn reconfigure<F>(&self, new: EmbeddingConfig, on_progress: F) -> Result<(), String>
    where
        F: Fn(Health) + Send + Sync,
    {
        let _permit = self
            .sem
            .acquire()
            .await
            .map_err(|e| format!("semaphore closed: {e}"))?;

        // Validation + Ollama reachability / pull.
        if let EmbeddingBackend::Ollama = new.backend {
            let ollama = new
                .ollama
                .as_ref()
                .ok_or_else(|| "ollama backend requires ollama config".to_string())?;
            if ollama.dimensions == 0 {
                let reason = "ollama config missing dimensions".to_string();
                let h = Health::Misconfigured(reason.clone());
                self.set_health(h.clone());
                on_progress(h);
                return Err(reason);
            }

            let client = OllamaClient::new(ollama.base_url.clone());
            // Probe reachability.
            if let Err(e) = client.ping().await {
                self.set_health(Health::OllamaUnreachable);
                on_progress(Health::OllamaUnreachable);
                return Err(format!("ollama unreachable: {e}"));
            }
            // Pull if missing.
            let tags = client.list_models().await.unwrap_or_default();
            let have_model = tags
                .iter()
                .any(|t| t == &ollama.model || t.starts_with(&format!("{}:", ollama.model)));
            if !have_model {
                // Emit an initial Pulling(0,0) so subscribers see the transition.
                self.set_health(Health::Pulling {
                    mb_done: 0,
                    mb_total: 0,
                });
                on_progress(Health::Pulling {
                    mb_done: 0,
                    mb_total: 0,
                });
                let on_pg = |done: u64, total: u64| {
                    on_progress(Health::Pulling {
                        mb_done: done / (1024 * 1024),
                        mb_total: total / (1024 * 1024),
                    });
                };
                if let Err(e) = client.pull_model(&ollama.model, on_pg).await {
                    let reason = format!("ollama pull failed: {e}");
                    let h = Health::Misconfigured(reason.clone());
                    self.set_health(h.clone());
                    on_progress(h);
                    return Err(reason);
                }
            }
        }

        let client = build_client(&new).inspect_err(|e| {
            let h = Health::Misconfigured(e.clone());
            self.set_health(h.clone());
            on_progress(h);
        })?;

        // Dim probe: for Ollama backends, verify the model actually returns
        // vectors at the declared dimension before we swap it in. A mismatch
        // would silently corrupt the vec0 indexes (which are built at a
        // fixed dim). This used to be guarded by a 6-entry curated dim
        // lookup; dropping that gate in favor of a truthful probe lets the
        // UI accept any embedding model the user has pulled.
        if let EmbeddingBackend::Ollama = new.backend {
            if let Err(reason) = probe_dimensions(client.as_ref(), new.dimensions).await {
                let h = Health::Misconfigured(reason.clone());
                self.set_health(h.clone());
                on_progress(h);
                return Err(reason);
            }
        }

        // Determine whether indexes need a rebuild.
        let marker = read_marker(&self.paths);
        let needs = marker
            .as_ref()
            .is_none_or(|m| m.dim != new.dimensions || marker_model(m) != config_model(&new));

        // Atomic swap.
        self.swap.store(Arc::new(ClientHandle { inner: client }));
        {
            let mut s = self.state.write();
            s.config = new.clone();
            s.dimensions = new.dimensions;
            s.needs_reindex = needs;
            s.health = Health::Ready;
        }

        on_progress(Health::Ready);
        Ok(())
    }

    /// Synchronous convenience used at boot. No-op — the actual reindex
    /// runs from [`crate::EmbeddingService::reconcile_at_boot_via`] (wired
    /// in `AppState::reconcile_embeddings_at_boot`), which has access to
    /// the `KnowledgeDatabase` this crate intentionally does not depend on.
    ///
    /// # Errors
    ///
    /// Always returns `Ok`.
    pub fn ensure_indexed_blocking(&self) -> Result<(), String> {
        Ok(())
    }

    /// Async companion to [`Self::ensure_indexed_blocking`]. Also a no-op —
    /// reindex is driven from the boot reconciler in `gateway::state` which
    /// owns the `KnowledgeDatabase` handle.
    ///
    /// # Errors
    ///
    /// Always returns `Ok`.
    pub async fn ensure_indexed(&self) -> Result<(), String> {
        Ok(())
    }

    /// Snapshot the current config (cheap — clone out of the RwLock).
    #[must_use]
    pub fn config_snapshot(&self) -> EmbeddingConfig {
        self.state.read().config.clone()
    }

    /// Public health setter so the boot reconciler / HTTP handler can
    /// stream `Reindexing { .. }` transitions while driving
    /// `reindex_all` themselves.
    pub fn publish_health(&self, h: Health) {
        self.set_health(h);
    }

    /// Pre-emptive reachability check performed at boot. If the backend
    /// is Ollama, ping it and set `Health::OllamaUnreachable` on failure so
    /// the UI (which polls `/api/embeddings/health`) shows the degraded
    /// state immediately instead of waiting for the periodic loop.
    pub async fn preflight(&self) {
        let cfg = self.config_snapshot();
        if let EmbeddingBackend::Ollama = cfg.backend {
            let Some(ollama) = cfg.ollama.as_ref() else {
                self.set_health(Health::Misconfigured(
                    "ollama backend without ollama config".to_string(),
                ));
                return;
            };
            let client = OllamaClient::new(ollama.base_url.clone());
            match client.ping().await {
                Ok(()) => self.set_health(Health::Ready),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        base_url = %ollama.base_url,
                        model = %ollama.model,
                        "embedding preflight: ollama unreachable — recall will \
                         degrade to FTS-only until resolved. Fix: start Ollama \
                         at this URL, OR switch to the internal (BGE-small) \
                         backend via Settings > Advanced > Embeddings."
                    );
                    self.set_health(Health::OllamaUnreachable);
                }
            }
        }
    }

    /// Spawn a periodic health-check loop. For `Internal` backends this is a
    /// cheap re-check (no network). For `Ollama`, each tick pings the
    /// configured URL and updates `Health::Ready` <-> `Health::OllamaUnreachable`
    /// accordingly. The loop inspects the live config on every iteration, so
    /// after `reconfigure` swaps backend the loop adapts automatically.
    pub fn start_health_loop(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        self.start_health_loop_with_interval(std::time::Duration::from_secs(60))
    }

    /// Test-visible variant of [`Self::start_health_loop`] with a custom
    /// interval. Not exposed in the public prelude but public so integration
    /// tests can exercise the loop without waiting 60 seconds.
    pub fn start_health_loop_with_interval(
        self: Arc<Self>,
        interval: std::time::Duration,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // Skip the immediate first tick — preflight already ran.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let cfg = self.config_snapshot();
                match cfg.backend {
                    EmbeddingBackend::Unconfigured => continue,
                    EmbeddingBackend::Internal => self.probe_internal_health(),
                    EmbeddingBackend::Ollama => self.probe_ollama_health(&cfg).await,
                }
            }
        })
    }

    /// `Internal` probe: nothing to ping, just ensure health reflects readiness
    /// without stomping on in-flight Reindexing/Pulling transitions.
    fn probe_internal_health(&self) {
        if matches!(self.health(), Health::OllamaUnreachable) {
            self.set_health(Health::Ready);
        }
    }

    /// `Ollama` probe: ping the configured endpoint and flip
    /// `Ready <-> OllamaUnreachable`. Honors in-flight Reindexing/Pulling.
    async fn probe_ollama_health(&self, cfg: &EmbeddingConfig) {
        let Some(ollama) = cfg.ollama.as_ref() else {
            return;
        };
        if matches!(
            self.health(),
            Health::Reindexing { .. } | Health::Pulling { .. }
        ) {
            return;
        }
        let client = OllamaClient::new(ollama.base_url.clone());
        match client.ping().await {
            Ok(()) => {
                if matches!(self.health(), Health::OllamaUnreachable) {
                    self.set_health(Health::Ready);
                }
            }
            Err(_) => self.set_health(Health::OllamaUnreachable),
        }
    }

    /// Internal: mark the current config as fully indexed at `dim` and
    /// persist the marker. Called by the reindex pipeline on success.
    ///
    /// # Errors
    ///
    /// Returns an error if the marker file cannot be written.
    pub fn mark_indexed(&self, dim: usize) -> Result<(), String> {
        let (backend, model) = {
            let s = self.state.read();
            (s.config.backend, config_model(&s.config))
        };
        let marker = Marker {
            backend,
            model,
            dim,
            indexed_at: Utc::now().to_rfc3339(),
        };
        write_marker(&self.paths, &marker)?;
        let mut s = self.state.write();
        s.indexed_dim = dim;
        s.needs_reindex = false;
        Ok(())
    }

    fn set_health(&self, h: Health) {
        self.state.write().health = h;
    }

    /// Overwrite `config/settings.json` `embeddings` section atomically.
    /// Other top-level keys are preserved.
    ///
    /// # Errors
    ///
    /// Returns an error on IO or serialization failure.
    pub fn persist_settings(&self, cfg: &EmbeddingConfig) -> Result<(), String> {
        persist_settings_raw(&self.paths, cfg)
    }
}

fn config_model(cfg: &EmbeddingConfig) -> String {
    match cfg.backend {
        EmbeddingBackend::Unconfigured => "unconfigured".to_string(),
        EmbeddingBackend::Internal => "bge-small-en-v1.5".to_string(),
        EmbeddingBackend::Ollama => cfg
            .ollama
            .as_ref()
            .map(|o| o.model.clone())
            .unwrap_or_default(),
    }
}

// ============================================================================
// NoopEmbeddingClient
// Returned by `build_client` when no backend has been configured yet.
// Keeps the heavy internal BGE ONNX out of memory until the user picks a
// backend via Settings → Advanced → Embeddings.
// ============================================================================

struct NoopEmbeddingClient;

#[async_trait]
impl EmbeddingClient for NoopEmbeddingClient {
    async fn embed(&self, _texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        Err(EmbeddingError::ConfigError(
            "embedding backend not configured — choose internal or ollama in Settings → Advanced → Embeddings".to_string(),
        ))
    }

    fn dimensions(&self) -> usize {
        0
    }

    fn model_name(&self) -> String {
        "unconfigured".to_string()
    }

    fn provider_type(&self) -> String {
        "unconfigured".to_string()
    }
}

// ============================================================================
// LiveEmbeddingClient
// A thin wrapper that re-reads the current `EmbeddingService` client on every
// call, so downstream consumers can hold an `Arc<dyn EmbeddingClient>` that
// transparently follows backend swaps (ArcSwap) performed by the UI's
// "Save & Switch" flow.
//
// Without this, distillation/recall/etc. cache the initial (Unconfigured /
// Internal / Ollama) client at boot and never see later reconfigures —
// producing confusing "embedding backend not configured" errors even after
// the user has configured Ollama in Settings.
// ============================================================================

pub struct LiveEmbeddingClient {
    service: Arc<EmbeddingService>,
}

impl LiveEmbeddingClient {
    pub fn new(service: Arc<EmbeddingService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl EmbeddingClient for LiveEmbeddingClient {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        self.service.client().embed(texts).await
    }

    fn dimensions(&self) -> usize {
        self.service.client().dimensions()
    }

    fn model_name(&self) -> String {
        self.service.client().model_name()
    }

    fn provider_type(&self) -> String {
        self.service.client().provider_type()
    }

    fn prompt_profile(&self) -> String {
        self.service.client().prompt_profile()
    }

    fn normalization(&self) -> Option<String> {
        self.service.client().normalization()
    }
}

fn marker_model(m: &Marker) -> String {
    m.model.clone()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_paths() -> (tempfile::TempDir, SharedVaultPaths) {
        let dir = tempdir().unwrap();
        let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
            dir.path().to_path_buf(),
        ));
        paths.ensure_dirs_exist().unwrap();
        (dir, paths)
    }

    /// Poll `check` until it returns true or `deadline` elapses. Keeps health-loop
    /// tests stable on slow CI runners without coupling them to a specific tick count.
    async fn wait_until<F: FnMut() -> bool>(deadline: std::time::Duration, mut check: F) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < deadline {
            if check() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        check()
    }

    #[tokio::test]
    async fn from_config_on_fresh_vault_self_heals_to_internal() {
        let (_tmp, paths) = test_paths();
        let svc = EmbeddingService::from_config(paths.clone()).unwrap();

        // Fresh vault + no `embeddings` section in settings.json + no
        // marker → auto-activate internal BGE-small so recall works out
        // of the box.
        assert_eq!(svc.dimensions(), 384);
        assert_eq!(svc.client().model_name(), "bge-small-en-v1.5");

        // Choice is persisted so subsequent boots read the explicit
        // config instead of re-triggering the self-heal path.
        let settings_text = std::fs::read_to_string(paths.settings())
            .expect("settings.json should exist after self-heal");
        // The on-disk wire shape is the `{"internal": true}` form; see the
        // `EmbeddingConfig` doc comment for why the in-memory enum gets
        // flattened on serialize.
        assert!(
            settings_text.contains("\"embeddings\""),
            "persisted settings must carry an embeddings section ({settings_text:?})"
        );
        assert!(
            settings_text.contains("\"internal\""),
            "self-heal default is internal, not unconfigured or ollama"
        );
        assert!(
            settings_text.contains("true"),
            "internal flag must be set to true ({settings_text:?})"
        );
    }

    #[tokio::test]
    async fn from_config_with_existing_marker_does_not_self_heal() {
        let (_tmp, paths) = test_paths();
        // Pretend a prior boot already indexed at 1024 dims. The marker
        // alone (without an `embeddings` section) signals an existing
        // install, not a fresh vault — fresh-install self-heal must NOT
        // overwrite it. Main's marker-recovery path will still promote
        // the recorded backend (Ollama, dim 1024) back into the config
        // — that's the desired behaviour from the prior strip-on-save
        // fix and is what we want preserved here.
        let marker = Marker {
            backend: EmbeddingBackend::Ollama,
            model: "mxbai-embed-large".into(),
            dim: 1024,
            indexed_at: "".into(),
        };
        write_marker(&paths, &marker).unwrap();

        let svc = EmbeddingService::from_config(paths.clone()).unwrap();

        // The marker recovery should pick up the recorded Ollama
        // backend rather than the fresh-install default; dim stays at
        // 1024 so the existing index isn't invalidated.
        assert_eq!(svc.dimensions(), 1024);
        assert_eq!(svc.client().model_name(), "mxbai-embed-large");

        // Settings.json was written by the marker-recovery path (so the
        // selection survives a future strip), but it carries the Ollama
        // backend — NOT the fresh-install Internal default.
        let settings_text = std::fs::read_to_string(paths.settings())
            .expect("marker recovery writes settings.json");
        assert!(
            settings_text.contains("\"ollama\""),
            "marker recovery must persist the recovered Ollama config, not fresh-install default ({settings_text:?})"
        );
    }

    #[tokio::test]
    async fn from_config_with_internal_backend_loads_local_client() {
        let (_tmp, paths) = test_paths();
        let cfg = EmbeddingConfig {
            backend: EmbeddingBackend::Internal,
            dimensions: 384,
            ollama: None,
        };
        let svc = EmbeddingService::with_config(paths, cfg).unwrap();
        assert_eq!(svc.client().model_name(), "bge-small-en-v1.5");
    }

    #[tokio::test]
    async fn from_config_with_marker_dim_mismatch_sets_needs_reindex() {
        let (_tmp, paths) = test_paths();
        // Explicit internal-backend settings.json — so marker-recovery does
        // not kick in and overwrite with the marker's Ollama config.
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(
            paths.settings(),
            r#"{ "embeddings": { "internal": true } }"#,
        )
        .unwrap();
        // Marker records a previous indexing at a different dim.
        let marker = Marker {
            backend: EmbeddingBackend::Ollama,
            model: "mxbai-embed-large".into(),
            dim: 1024,
            indexed_at: "".into(),
        };
        write_marker(&paths, &marker).unwrap();

        let svc = EmbeddingService::from_config(paths).unwrap();
        assert_eq!(svc.config_snapshot().backend, EmbeddingBackend::Internal);
        assert!(svc.needs_reindex());
    }

    #[test]
    fn marker_write_is_atomic() {
        let (_tmp, paths) = test_paths();
        let m = Marker {
            backend: EmbeddingBackend::Ollama,
            model: "bge-m3".into(),
            dim: 1024,
            indexed_at: "2026-04-14T00:00:00Z".into(),
        };
        write_marker(&paths, &m).unwrap();
        let round = read_marker(&paths).unwrap();
        assert_eq!(round, m);
        // No leftover temp file.
        let tmp = paths.data_dir().join(".embedding-state.tmp");
        assert!(!tmp.exists());
    }

    #[test]
    fn marker_parse_rejects_incomplete() {
        assert!(Marker::parse("backend=internal\n").is_none());
        assert!(Marker::parse("").is_none());
    }

    /// Spin up a MockServer that responds to `/api/tags` with the given models
    /// (so `pull` is not triggered) and `/api/pull` with an immediate success
    /// line for safety. Returns the server and its base_url.
    async fn mock_ollama_with_models(models: &[&str]) -> httpmock::MockServer {
        use httpmock::prelude::*;
        let server = MockServer::start_async().await;
        let entries: Vec<_> = models
            .iter()
            .map(|m| serde_json::json!({ "name": m }))
            .collect();
        let body = serde_json::json!({ "models": entries });
        server
            .mock_async(move |when, then| {
                when.method(GET).path("/api/tags");
                then.status(200).json_body(body);
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/api/pull");
                then.status(200).body("{\"status\":\"success\"}\n");
            })
            .await;
        server
    }

    /// Install a `/v1/embeddings` mock on `server` that returns a single
    /// vector of the given dimension. Reconfigure's post-swap probe hits
    /// this endpoint to verify dim correctness.
    async fn stub_embed_dim(server: &httpmock::MockServer, dim: usize) {
        use httpmock::prelude::*;
        let fake_vec: Vec<f32> = vec![0.1_f32; dim];
        let body = serde_json::json!({
            "data": [{ "embedding": fake_vec, "index": 0, "object": "embedding" }],
            "model": "mock",
            "object": "list",
            "usage": { "prompt_tokens": 1, "total_tokens": 1 }
        });
        server
            .mock_async(move |when, then| {
                when.method(POST).path("/v1/embeddings");
                then.status(200).json_body(body);
            })
            .await;
    }

    #[tokio::test]
    async fn reconfigure_internal_to_same_dim_ollama_does_not_need_reindex_if_marker_matches() {
        let server = mock_ollama_with_models(&["snowflake-arctic-embed:s"]).await;
        stub_embed_dim(&server, 384).await;
        let (_tmp, paths) = test_paths();
        // Mark as already indexed at 384 under the snowflake:s model.
        let m = Marker {
            backend: EmbeddingBackend::Ollama,
            model: "snowflake-arctic-embed:s".into(),
            dim: 384,
            indexed_at: "x".into(),
        };
        write_marker(&paths, &m).unwrap();
        let svc = EmbeddingService::from_config(paths).unwrap();
        // Switch to that Ollama model at same dim 384.
        let new = EmbeddingConfig {
            backend: EmbeddingBackend::Ollama,
            dimensions: 384,
            ollama: Some(OllamaConfig {
                base_url: server.base_url(),
                model: "snowflake-arctic-embed:s".into(),
                dimensions: 384,
            }),
        };
        svc.reconfigure(new, |_| {}).await.unwrap();
        assert!(!svc.needs_reindex());
    }

    #[tokio::test]
    async fn reconfigure_with_dim_change_triggers_needs_reindex() {
        let server = mock_ollama_with_models(&["mxbai-embed-large"]).await;
        stub_embed_dim(&server, 1024).await;
        let (_tmp, paths) = test_paths();
        let svc = EmbeddingService::from_config(paths).unwrap();
        let new = EmbeddingConfig {
            backend: EmbeddingBackend::Ollama,
            dimensions: 1024,
            ollama: Some(OllamaConfig {
                base_url: server.base_url(),
                model: "mxbai-embed-large".into(),
                dimensions: 1024,
            }),
        };
        svc.reconfigure(new, |_| {}).await.unwrap();
        assert!(svc.needs_reindex());
        assert_eq!(svc.dimensions(), 1024);
        assert_eq!(svc.client().dimensions(), 1024);
    }

    /// Replaces the old `reconfigure_rejects_uncurated_model` test. Curated
    /// lookup is no longer a gate — dim correctness is verified by the
    /// probe. Here the mock returns 768-d vectors but the config declares
    /// 1024, so reconfigure must reject and mark Misconfigured.
    #[tokio::test]
    async fn reconfigure_rejects_dimension_mismatch_via_probe() {
        let server = mock_ollama_with_models(&["custom-embed"]).await;
        stub_embed_dim(&server, 768).await;
        let (_tmp, paths) = test_paths();
        let svc = EmbeddingService::from_config(paths).unwrap();
        let new = EmbeddingConfig {
            backend: EmbeddingBackend::Ollama,
            dimensions: 1024,
            ollama: Some(OllamaConfig {
                base_url: server.base_url(),
                model: "custom-embed".into(),
                dimensions: 1024,
            }),
        };
        let err = svc.reconfigure(new, |_| {}).await;
        assert!(err.is_err());
        let reason = err.err().unwrap();
        assert!(
            reason.contains("768") && reason.contains("1024"),
            "error should mention actual and declared dims: {reason}"
        );
        match svc.health() {
            Health::Misconfigured(_) => {}
            other => panic!("expected Misconfigured, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn concurrent_reconfigure_serializes_via_semaphore() {
        let (_tmp, paths) = test_paths();
        let svc = Arc::new(EmbeddingService::from_config(paths).unwrap());
        let mut handles = Vec::new();
        for i in 0..4 {
            let svc = svc.clone();
            handles.push(tokio::spawn(async move {
                let cfg = EmbeddingConfig {
                    backend: EmbeddingBackend::Internal,
                    dimensions: 384 + (i % 2),
                    ollama: None,
                };
                let _ = svc.reconfigure(cfg, |_| {}).await;
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        // State is defined afterwards.
        assert!(svc.dimensions() >= 384);
    }

    #[tokio::test]
    async fn health_reflects_ready_on_construction() {
        let (_tmp, paths) = test_paths();
        let svc = EmbeddingService::from_config(paths).unwrap();
        matches!(svc.health(), Health::Ready);
    }

    #[tokio::test]
    async fn mark_indexed_writes_marker_and_clears_flag() {
        let (_tmp, paths) = test_paths();
        // Use an explicit Internal config — `from_config` with no settings
        // now returns Unconfigured, which wouldn't produce a usable marker.
        let cfg = EmbeddingConfig {
            backend: EmbeddingBackend::Internal,
            dimensions: 384,
            ollama: None,
        };
        let svc = EmbeddingService::with_config(paths.clone(), cfg).unwrap();
        // Force needs_reindex first.
        svc.state.write().needs_reindex = true;
        svc.mark_indexed(384).unwrap();
        assert!(!svc.needs_reindex());
        let m = read_marker(&paths).unwrap();
        assert_eq!(m.dim, 384);
        assert_eq!(m.backend, EmbeddingBackend::Internal);
    }

    #[test]
    fn curated_list_has_six_entries() {
        assert_eq!(CURATED_MODELS.len(), 6);
        assert!(curated_lookup("mxbai-embed-large").is_some());
        assert!(curated_lookup("snowflake-arctic-embed:s").is_some());
        assert!(curated_lookup("not-a-model").is_none());
    }

    #[tokio::test]
    async fn persist_settings_round_trip() {
        let (_tmp, paths) = test_paths();
        let svc = EmbeddingService::from_config(paths.clone()).unwrap();
        let cfg = EmbeddingConfig {
            backend: EmbeddingBackend::Ollama,
            dimensions: 1024,
            ollama: Some(OllamaConfig {
                base_url: "http://localhost:11434".into(),
                model: "mxbai-embed-large".into(),
                dimensions: 1024,
            }),
        };
        svc.persist_settings(&cfg).unwrap();
        let reread = read_config_from_settings_json(&paths);
        assert_eq!(reread, cfg);
    }

    #[tokio::test]
    async fn reconfigure_swaps_client_arc() {
        let server = mock_ollama_with_models(&["bge-large"]).await;
        stub_embed_dim(&server, 1024).await;
        let (_tmp, paths) = test_paths();
        let svc = EmbeddingService::from_config(paths).unwrap();
        let before = svc.client();
        let new = EmbeddingConfig {
            backend: EmbeddingBackend::Ollama,
            dimensions: 1024,
            ollama: Some(OllamaConfig {
                base_url: server.base_url(),
                model: "bge-large".into(),
                dimensions: 1024,
            }),
        };
        svc.reconfigure(new, |_| {}).await.unwrap();
        let after = svc.client();
        assert!(!Arc::ptr_eq(&before, &after));
        assert_eq!(after.dimensions(), 1024);
    }

    #[test]
    fn marker_parse_round_trip() {
        let m = Marker {
            backend: EmbeddingBackend::Internal,
            model: "bge-small-en-v1.5".into(),
            dim: 384,
            indexed_at: "2026-04-14T00:00:00Z".into(),
        };
        let rendered = m.render();
        let back = Marker::parse(&rendered).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn curated_lookup_matches_dim() {
        let m = curated_lookup("nomic-embed-text").unwrap();
        assert_eq!(m.dim, 768);
        assert_eq!(m.size_mb, 274);
    }

    #[tokio::test]
    async fn reconfigure_full_round_trip_with_mock_ollama() {
        use httpmock::prelude::*;
        let server = MockServer::start_async().await;
        // First /api/tags returns empty (model needs pull).
        server
            .mock_async(|when, then| {
                when.method(GET).path("/api/tags");
                then.status(200)
                    .json_body(serde_json::json!({ "models": [] }));
            })
            .await;
        // /api/pull streams a tiny progress + success line.
        server
            .mock_async(|when, then| {
                when.method(POST).path("/api/pull");
                then.status(200).body(
                    "{\"status\":\"downloading\",\"completed\":100,\"total\":670}\n\
                     {\"status\":\"success\"}\n",
                );
            })
            .await;
        // Dim probe endpoint.
        stub_embed_dim(&server, 1024).await;

        let (_tmp, paths) = test_paths();
        let svc = EmbeddingService::from_config(paths.clone()).unwrap();

        let new = EmbeddingConfig {
            backend: EmbeddingBackend::Ollama,
            dimensions: 1024,
            ollama: Some(OllamaConfig {
                base_url: server.base_url(),
                model: "mxbai-embed-large".into(),
                dimensions: 1024,
            }),
        };

        let events = std::sync::Mutex::new(Vec::<Health>::new());
        svc.reconfigure(new.clone(), |h| events.lock().unwrap().push(h))
            .await
            .unwrap();

        let captured = events.into_inner().unwrap();
        assert!(
            captured.iter().any(|h| matches!(h, Health::Pulling { .. })),
            "expected Pulling event in: {captured:?}"
        );
        assert!(matches!(svc.health(), Health::Ready));
        assert_eq!(svc.dimensions(), 1024);
        assert_eq!(svc.client().dimensions(), 1024);
    }

    #[tokio::test]
    async fn reconfigure_ollama_unreachable_returns_error() {
        let (_tmp, paths) = test_paths();
        let svc = EmbeddingService::from_config(paths).unwrap();
        let new = EmbeddingConfig {
            backend: EmbeddingBackend::Ollama,
            dimensions: 1024,
            ollama: Some(OllamaConfig {
                base_url: "http://127.0.0.1:1".into(),
                model: "mxbai-embed-large".into(),
                dimensions: 1024,
            }),
        };
        let err = svc.reconfigure(new, |_| {}).await;
        assert!(err.is_err());
        assert!(matches!(svc.health(), Health::OllamaUnreachable));
    }

    #[tokio::test]
    async fn ensure_indexed_placeholders_return_ok() {
        let (_tmp, paths) = test_paths();
        let svc = EmbeddingService::from_config(paths).unwrap();
        svc.ensure_indexed_blocking().unwrap();
        svc.ensure_indexed().await.unwrap();
    }

    #[tokio::test]
    async fn preflight_with_internal_backend_is_noop() {
        let (_tmp, paths) = test_paths();
        let svc = EmbeddingService::from_config(paths).unwrap();
        svc.preflight().await;
        assert!(matches!(svc.health(), Health::Ready));
    }

    #[tokio::test]
    async fn preflight_with_unreachable_ollama_sets_health_immediately() {
        let (_tmp, paths) = test_paths();
        let cfg = EmbeddingConfig {
            backend: EmbeddingBackend::Ollama,
            dimensions: 1024,
            ollama: Some(OllamaConfig {
                base_url: "http://127.0.0.1:1".into(),
                model: "mxbai-embed-large".into(),
                dimensions: 1024,
            }),
        };
        // with_config does not network — builds Ollama client, then preflight pings.
        let svc = EmbeddingService::with_config(paths, cfg).unwrap();
        svc.preflight().await;
        assert!(matches!(svc.health(), Health::OllamaUnreachable));
    }

    #[tokio::test]
    async fn health_loop_does_nothing_when_internal_backend() {
        let (_tmp, paths) = test_paths();
        let svc = Arc::new(EmbeddingService::from_config(paths).unwrap());
        let handle = svc
            .clone()
            .start_health_loop_with_interval(std::time::Duration::from_millis(50));
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        // Health never left Ready on internal.
        assert!(matches!(svc.health(), Health::Ready));
        handle.abort();
    }

    #[tokio::test]
    async fn health_loop_pings_ollama_periodically_when_active() {
        let server = mock_ollama_with_models(&["mxbai-embed-large"]).await;
        let (_tmp, paths) = test_paths();
        let cfg = EmbeddingConfig {
            backend: EmbeddingBackend::Ollama,
            dimensions: 1024,
            ollama: Some(OllamaConfig {
                base_url: server.base_url(),
                model: "mxbai-embed-large".into(),
                dimensions: 1024,
            }),
        };
        let svc = Arc::new(EmbeddingService::with_config(paths, cfg).unwrap());
        // Seed as unreachable — loop should flip to Ready on successful ping.
        svc.publish_health(Health::OllamaUnreachable);
        let handle = svc
            .clone()
            .start_health_loop_with_interval(std::time::Duration::from_millis(50));
        let flipped = wait_until(std::time::Duration::from_secs(3), || {
            matches!(svc.health(), Health::Ready)
        })
        .await;
        assert!(
            flipped,
            "expected Ready after healthy ping, got {:?}",
            svc.health()
        );
        handle.abort();
    }

    #[tokio::test]
    async fn health_loop_marks_ollama_unreachable_when_down() {
        let (_tmp, paths) = test_paths();
        let cfg = EmbeddingConfig {
            backend: EmbeddingBackend::Ollama,
            dimensions: 1024,
            ollama: Some(OllamaConfig {
                base_url: "http://127.0.0.1:1".into(),
                model: "mxbai-embed-large".into(),
                dimensions: 1024,
            }),
        };
        let svc = Arc::new(EmbeddingService::with_config(paths, cfg).unwrap());
        // Start Ready; loop should discover it's actually unreachable.
        let handle = svc
            .clone()
            .start_health_loop_with_interval(std::time::Duration::from_millis(50));
        let flipped = wait_until(std::time::Duration::from_secs(3), || {
            matches!(svc.health(), Health::OllamaUnreachable)
        })
        .await;
        assert!(
            flipped,
            "expected OllamaUnreachable, got {:?}",
            svc.health()
        );
        handle.abort();
    }

    #[tokio::test]
    async fn config_snapshot_returns_current_config() {
        let (_tmp, paths) = test_paths();
        let cfg = EmbeddingConfig {
            backend: EmbeddingBackend::Internal,
            dimensions: 384,
            ollama: None,
        };
        let svc = EmbeddingService::with_config(paths, cfg.clone()).unwrap();
        assert_eq!(svc.config_snapshot(), cfg);
    }

    // ------------------------------------------------------------------------
    // Wire-format: new shape + legacy-shape migration
    // ------------------------------------------------------------------------

    #[test]
    fn deserialize_new_shape_internal_true() {
        let json = r#"{ "internal": true }"#;
        let cfg: EmbeddingConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.backend, EmbeddingBackend::Internal);
        assert_eq!(cfg.dimensions, 384);
        assert!(cfg.ollama.is_none());
    }

    #[test]
    fn deserialize_new_shape_internal_false_with_ollama() {
        let json = r#"{
            "internal": false,
            "ollama": {
                "url": "http://localhost:11434",
                "model": "nomic-embed-text",
                "dimensions": 768
            }
        }"#;
        let cfg: EmbeddingConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.backend, EmbeddingBackend::Ollama);
        assert_eq!(cfg.dimensions, 768);
        let o = cfg.ollama.unwrap();
        assert_eq!(o.base_url, "http://localhost:11434");
        assert_eq!(o.model, "nomic-embed-text");
        assert_eq!(o.dimensions, 768);
    }

    #[test]
    fn deserialize_new_shape_preserves_ollama_when_internal_true() {
        // Users toggling internal=true keep their last Ollama URL/model so
        // flipping back doesn't force them to retype.
        let json = r#"{
            "internal": true,
            "ollama": {
                "url": "http://box.local:11434",
                "model": "bge-large",
                "dimensions": 1024
            }
        }"#;
        let cfg: EmbeddingConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.backend, EmbeddingBackend::Internal);
        // Effective dim is the internal BGE dim, not the preserved Ollama dim.
        assert_eq!(cfg.dimensions, 384);
        let o = cfg.ollama.unwrap();
        assert_eq!(o.base_url, "http://box.local:11434");
        assert_eq!(o.model, "bge-large");
        assert_eq!(o.dimensions, 1024);
    }

    #[test]
    fn deserialize_legacy_shape_ollama_migrates() {
        // Shape written by older builds: `backend` string + top-level
        // `dimensions` + ollama.{base_url, model}. Must migrate cleanly.
        let json = r#"{
            "backend": "ollama",
            "dimensions": 1024,
            "ollama": {
                "base_url": "http://localhost:11434",
                "model": "snowflake-arctic-embed"
            }
        }"#;
        let cfg: EmbeddingConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.backend, EmbeddingBackend::Ollama);
        assert_eq!(cfg.dimensions, 1024);
        let o = cfg.ollama.unwrap();
        assert_eq!(o.base_url, "http://localhost:11434");
        assert_eq!(o.model, "snowflake-arctic-embed");
        assert_eq!(o.dimensions, 1024);
    }

    #[test]
    fn deserialize_legacy_shape_internal_migrates() {
        let json = r#"{ "backend": "internal", "dimensions": 384 }"#;
        let cfg: EmbeddingConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.backend, EmbeddingBackend::Internal);
        assert_eq!(cfg.dimensions, 384);
        assert!(cfg.ollama.is_none());
    }

    #[test]
    fn deserialize_empty_object_is_unconfigured() {
        let cfg: EmbeddingConfig = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(cfg.backend, EmbeddingBackend::Unconfigured);
        assert_eq!(cfg.dimensions, 0);
        assert!(cfg.ollama.is_none());
    }

    #[test]
    fn serialize_always_emits_new_shape() {
        let cfg = EmbeddingConfig {
            backend: EmbeddingBackend::Ollama,
            dimensions: 1024,
            ollama: Some(OllamaConfig {
                base_url: "http://h:11434".into(),
                model: "mxbai-embed-large".into(),
                dimensions: 1024,
            }),
        };
        let v = serde_json::to_value(&cfg).unwrap();
        assert_eq!(v["internal"].as_bool(), Some(false));
        assert_eq!(v["ollama"]["url"].as_str(), Some("http://h:11434"));
        assert_eq!(v["ollama"]["model"].as_str(), Some("mxbai-embed-large"));
        assert_eq!(v["ollama"]["dimensions"].as_u64(), Some(1024));
        // Legacy keys must NOT appear in output.
        assert!(v.get("backend").is_none());
        assert!(v.get("dimensions").is_none());
        assert!(v["ollama"].get("base_url").is_none());
    }

    // ------------------------------------------------------------------------
    // Boot marker-recovery
    // ------------------------------------------------------------------------

    #[test]
    fn from_config_recovers_ollama_selection_from_marker() {
        // Regression: previously the strip-on-save bug in SettingsService
        // wiped the embeddings section each time any other setting was
        // updated, leaving the service Unconfigured on boot while the
        // marker still recorded a valid selection. This test simulates
        // the stale-marker / empty-settings state and asserts the boot
        // path recovers and writes the section back.
        let (_tmp, paths) = test_paths();

        // No settings.json embeddings section (fresh install / wiped).
        // Marker says we previously indexed at 1024-d ollama/snowflake.
        let m = Marker {
            backend: EmbeddingBackend::Ollama,
            model: "snowflake-arctic-embed".into(),
            dim: 1024,
            indexed_at: "2026-04-23T18:42:28+00:00".into(),
        };
        write_marker(&paths, &m).unwrap();

        let svc = EmbeddingService::from_config(paths.clone()).unwrap();
        let cfg = svc.config_snapshot();
        assert_eq!(cfg.backend, EmbeddingBackend::Ollama);
        assert_eq!(cfg.dimensions, 1024);
        let o = cfg.ollama.as_ref().expect("ollama config recovered");
        assert_eq!(o.model, "snowflake-arctic-embed");
        assert_eq!(o.dimensions, 1024);

        // The recovered config should be written back to settings.json so
        // subsequent boots read from there without needing recovery.
        let written = read_config_from_settings_json(&paths);
        assert_eq!(written.backend, EmbeddingBackend::Ollama);
        assert_eq!(written.dimensions, 1024);
    }

    #[test]
    fn from_config_no_marker_no_settings_self_heals_to_internal() {
        // Superseded the original "stays_unconfigured" test: fresh vaults
        // (no settings + no marker) now auto-activate the internal
        // backend so recall works out of the box. The deliberate
        // Unconfigured state is still reachable — just not as a default.
        let (_tmp, paths) = test_paths();
        let svc = EmbeddingService::from_config(paths).unwrap();
        let cfg = svc.config_snapshot();
        assert_eq!(cfg.backend, EmbeddingBackend::Internal);
        assert_eq!(cfg.dimensions, INTERNAL_BGE_DIM);
    }

    #[test]
    fn legacy_shape_round_trips_to_new_shape_via_load_save() {
        // Simulates: boot reads legacy from disk, save() writes new shape back.
        let legacy = r#"{
            "backend": "ollama",
            "dimensions": 768,
            "ollama": {
                "base_url": "http://localhost:11434",
                "model": "nomic-embed-text"
            }
        }"#;
        let cfg: EmbeddingConfig = serde_json::from_str(legacy).unwrap();
        let reemitted = serde_json::to_string(&cfg).unwrap();
        // Reparse should produce the same struct.
        let cfg2: EmbeddingConfig = serde_json::from_str(&reemitted).unwrap();
        assert_eq!(cfg, cfg2);
        assert!(!reemitted.contains("\"backend\""));
        assert!(!reemitted.contains("\"base_url\""));
        assert!(reemitted.contains("\"internal\""));
    }
}
