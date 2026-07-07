//! Dry-run and gated apply tooling for moving current zbot DBs to Engram.
//!
//! This module deliberately starts with diagnostics and manifest binding. It
//! does not import row contents yet; non-empty source tables produce a hard
//! blocker so apply cannot imply unsupported parity.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use chrono::Utc;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{
    capabilities::AdapterFeature,
    config::{AdapterConfig, MigrationMode, ProviderMode},
    error::{AdapterError, AdapterResult},
};

const MIGRATION_COMPONENT: &str = "migration";
const MIGRATION_MARKER_FILE: &str = "zbot-migration.sqlite";

const KNOWLEDGE_TABLES: &[&str] = &[
    "memory_facts",
    "memory_facts_archive",
    "kg_entities",
    "kg_relationships",
    "kg_aliases",
    "kg_episodes",
    "kg_episode_payloads",
    "kg_goals",
    "kg_compactions",
    "kg_causal_edges",
    "kg_beliefs",
    "kg_belief_contradictions",
    "ward_wiki_articles",
    "procedures",
    "session_episodes",
    "embedding_cache",
    "skill_index_state",
];

const CONVERSATION_TABLES: &[&str] = &[
    "sessions",
    "messages",
    "agent_executions",
    "execution_logs",
    "bridge_outbox",
    "distillation_runs",
    "recall_log",
    "artifacts",
];

/// Source DB family read by migration dry-run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationSourceKind {
    /// Current `knowledge.db` style source.
    Knowledge,
    /// Current `conversations.db` style source.
    Conversation,
}

impl MigrationSourceKind {
    fn as_key(self) -> &'static str {
        match self {
            Self::Knowledge => "knowledge",
            Self::Conversation => "conversation",
        }
    }

    fn allowlisted_tables(self) -> &'static [&'static str] {
        match self {
            Self::Knowledge => KNOWLEDGE_TABLES,
            Self::Conversation => CONVERSATION_TABLES,
        }
    }
}

/// Private input pointing at a local SQLite source DB.
///
/// This type intentionally does not serialize: path-bearing values must not be
/// copied into committed diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationSource {
    /// Source DB kind.
    pub kind: MigrationSourceKind,
    /// Local source DB path.
    pub path: PathBuf,
}

impl MigrationSource {
    /// Build a knowledge DB source.
    pub fn knowledge(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: MigrationSourceKind::Knowledge,
            path: path.into(),
        }
    }

    /// Build a conversation DB source.
    pub fn conversation(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: MigrationSourceKind::Conversation,
            path: path.into(),
        }
    }
}

/// Migration input supplied by composition or tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationInput {
    /// Adapter config used for provider/mapping fingerprints.
    pub config: AdapterConfig,
    /// Source DBs to inspect.
    pub sources: Vec<MigrationSource>,
    /// Adapter package/version evidence.
    pub adapter_version: String,
    /// Engram source revision/provenance evidence.
    pub engram_revision: String,
    /// Migration code version evidence.
    pub migration_code_version: String,
    /// Host-owned ontology policy name.
    pub ontology_policy: String,
    /// Host-owned taxonomy policy name.
    pub taxonomy_policy: String,
}

impl MigrationInput {
    /// Construct default input with crate-local version evidence.
    pub fn new(config: AdapterConfig, sources: Vec<MigrationSource>) -> Self {
        Self {
            config,
            sources,
            adapter_version: env!("CARGO_PKG_VERSION").to_string(),
            engram_revision: "local-engram-unpinned".to_string(),
            migration_code_version: "t7-manifest-gate-v1".to_string(),
            ontology_policy: "agentzero_dynamic_ontology".to_string(),
            taxonomy_policy: "agentzero_dynamic_taxonomy".to_string(),
        }
    }

    /// Override Engram revision/provenance evidence for an auditable run.
    pub fn with_engram_revision(mut self, revision: impl Into<String>) -> Self {
        self.engram_revision = revision.into();
        self
    }

    /// Override migration code version evidence for tests or release records.
    pub fn with_migration_code_version(mut self, version: impl Into<String>) -> Self {
        self.migration_code_version = version.into();
        self
    }
}

/// Redacted severity for migration diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationDiagnosticSeverity {
    /// Informational dry-run event.
    Info,
    /// Recoverable source-shape issue.
    Warning,
    /// Hard validation issue.
    Error,
}

/// Public dry-run diagnostic. Fields are positive-allowlisted and path-free.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationDiagnostic {
    /// Stable diagnostic code.
    pub code: String,
    /// Diagnostic severity.
    pub severity: MigrationDiagnosticSeverity,
    /// Source kind, when applicable.
    pub source: Option<MigrationSourceKind>,
    /// Allowlisted table name, when applicable.
    pub table: Option<String>,
    /// Redacted operator-facing message.
    pub message: String,
}

/// Hard blocker that prevents apply mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationBlocker {
    /// Stable blocker code.
    pub code: String,
    /// Adapter feature blocked by this issue.
    pub feature: AdapterFeature,
    /// Source kind, when applicable.
    pub source: Option<MigrationSourceKind>,
    /// Allowlisted table name, when applicable.
    pub table: Option<String>,
    /// Sanitized row count affected by this blocker.
    pub affected_rows: Option<u64>,
    /// Redacted reason.
    pub reason: String,
}

/// Per-source redacted count and identity evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationSourceReport {
    /// Source kind.
    pub source: MigrationSourceKind,
    /// Stable index within the supplied source list.
    pub source_index: usize,
    /// Hash of canonical local identity and metadata, never the path itself.
    pub source_identity_fingerprint: String,
    /// Source schema version when discoverable.
    pub schema_version: Option<i64>,
    /// Allowlisted table counts.
    pub counted_tables: BTreeMap<String, u64>,
}

/// Deterministic manifest that binds a dry-run to a later apply request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationManifest {
    /// Manifest schema version.
    pub manifest_version: u32,
    /// Hash over all manifest fields except this field.
    pub fingerprint: String,
    /// Hash over source identity fingerprints and sanitized counts.
    pub source_fingerprint: String,
    /// Source schema versions keyed by source kind/index.
    pub schema_versions: BTreeMap<String, i64>,
    /// Sanitized counts keyed by source kind/index/table.
    pub sanitized_counts: BTreeMap<String, u64>,
    /// Adapter version evidence.
    pub adapter_version: String,
    /// Engram revision/provenance evidence.
    pub engram_revision: String,
    /// Hash over path-free provider config evidence.
    pub provider_config_fingerprint: String,
    /// Hash over host-owned mapping policy evidence.
    pub mapping_config_fingerprint: String,
    /// Migration code version evidence.
    pub migration_code_version: String,
}

/// Dry-run result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationDryRunReport {
    /// Deterministic manifest for acceptance.
    pub manifest: MigrationManifest,
    /// Redacted per-source reports.
    pub sources: Vec<MigrationSourceReport>,
    /// Redacted diagnostics.
    pub diagnostics: Vec<MigrationDiagnostic>,
    /// Hard blockers that prevent apply.
    pub blockers: Vec<MigrationBlocker>,
    /// Always false for dry-run.
    pub would_write_engram_storage: bool,
}

/// Receipt returned after a gated apply marker is written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationApplyReceipt {
    /// Manifest fingerprint accepted and recorded.
    pub manifest_fingerprint: String,
    /// Component file name written under the confined Engram path.
    pub marker_component: String,
    /// RFC3339 timestamp.
    pub applied_at: String,
}

/// Inspect source DBs and build a deterministic, redacted manifest.
pub fn run_migration_dry_run(input: &MigrationInput) -> AdapterResult<MigrationDryRunReport> {
    validate_dry_run_input(input)?;
    let resolved = input.config.resolve_engram_path()?;

    let provider_config_fingerprint = provider_config_fingerprint(&input.config, &resolved)?;
    let mapping_config_fingerprint = mapping_config_fingerprint(input)?;

    let mut diagnostics = vec![diagnostic(
        "engram_path_validated",
        MigrationDiagnosticSeverity::Info,
        None,
        None,
        "configured Engram storage path is confined",
    )];
    let mut blockers = Vec::new();
    let mut source_reports = Vec::new();
    let mut schema_versions = BTreeMap::new();
    let mut sanitized_counts = BTreeMap::new();

    for (index, source) in input.sources.iter().enumerate() {
        let report = inspect_source(index, source, &mut diagnostics, &mut blockers);
        let source_key = source_report_key(source.kind, index);
        if let Some(version) = report.schema_version {
            schema_versions.insert(source_key, version);
        }
        for (table, count) in &report.counted_tables {
            sanitized_counts.insert(
                format!("{}.{}", source_report_key(source.kind, index), table),
                *count,
            );
        }
        source_reports.push(report);
    }

    let source_fingerprint = fingerprint_json(&source_reports)?;
    let mut manifest = MigrationManifest {
        manifest_version: 1,
        fingerprint: String::new(),
        source_fingerprint,
        schema_versions,
        sanitized_counts,
        adapter_version: input.adapter_version.clone(),
        engram_revision: input.engram_revision.clone(),
        provider_config_fingerprint,
        mapping_config_fingerprint,
        migration_code_version: input.migration_code_version.clone(),
    };
    manifest.fingerprint = manifest_fingerprint(&manifest)?;

    Ok(MigrationDryRunReport {
        manifest,
        sources: source_reports,
        diagnostics,
        blockers,
        would_write_engram_storage: false,
    })
}

/// Apply only after the accepted manifest exactly matches a fresh dry-run and
/// no hard blockers remain.
pub fn apply_migration(
    input: &MigrationInput,
    accepted_manifest: &MigrationManifest,
) -> AdapterResult<MigrationApplyReceipt> {
    if input.config.migration_mode != MigrationMode::Apply {
        return Err(AdapterError::UnsupportedFeature {
            feature: MIGRATION_COMPONENT,
            reason: "apply requires migrationMode=apply".to_string(),
        });
    }

    let mut dry_run_input = input.clone();
    dry_run_input.config.migration_mode = MigrationMode::DryRun;
    let dry_run = run_migration_dry_run(&dry_run_input)?;

    if &dry_run.manifest != accepted_manifest {
        return Err(AdapterError::UnsupportedFeature {
            feature: MIGRATION_COMPONENT,
            reason: "accepted dry-run manifest does not match current source/config".to_string(),
        });
    }
    if !dry_run.blockers.is_empty() {
        return Err(AdapterError::UnsupportedFeature {
            feature: MIGRATION_COMPONENT,
            reason: "dry-run has unresolved migration blockers".to_string(),
        });
    }

    let marker_path = input
        .config
        .compatibility_store_path(MIGRATION_MARKER_FILE)?;
    let connection = Connection::open(marker_path).map_err(|_| AdapterError::Storage {
        component: MIGRATION_COMPONENT,
        reason: "migration marker store cannot be opened".to_string(),
    })?;
    connection
        .execute_batch(
            r#"
            PRAGMA foreign_keys = ON;
            PRAGMA journal_mode = WAL;
            PRAGMA busy_timeout = 5000;
            CREATE TABLE IF NOT EXISTS migration_apply_runs (
                fingerprint TEXT PRIMARY KEY,
                applied_at TEXT NOT NULL,
                manifest_json TEXT NOT NULL
            );
            "#,
        )
        .map_err(|_| AdapterError::Storage {
            component: MIGRATION_COMPONENT,
            reason: "migration marker schema cannot be created".to_string(),
        })?;

    let applied_at = Utc::now().to_rfc3339();
    let manifest_json =
        serde_json::to_string(accepted_manifest).map_err(|_| AdapterError::Storage {
            component: MIGRATION_COMPONENT,
            reason: "migration manifest cannot be encoded".to_string(),
        })?;
    connection
        .execute(
            "INSERT OR REPLACE INTO migration_apply_runs (fingerprint, applied_at, manifest_json) VALUES (?1, ?2, ?3)",
            params![accepted_manifest.fingerprint, applied_at, manifest_json],
        )
        .map_err(|_| AdapterError::Storage {
            component: MIGRATION_COMPONENT,
            reason: "migration marker cannot be recorded".to_string(),
        })?;

    Ok(MigrationApplyReceipt {
        manifest_fingerprint: accepted_manifest.fingerprint.clone(),
        marker_component: MIGRATION_MARKER_FILE.to_string(),
        applied_at,
    })
}

fn validate_dry_run_input(input: &MigrationInput) -> AdapterResult<()> {
    if input.config.provider_mode != ProviderMode::Engram {
        return Err(AdapterError::UnsupportedFeature {
            feature: MIGRATION_COMPONENT,
            reason: "migration diagnostics require providerMode=engram".to_string(),
        });
    }
    if input.config.migration_mode != MigrationMode::DryRun {
        return Err(AdapterError::UnsupportedFeature {
            feature: MIGRATION_COMPONENT,
            reason: "dry-run requires migrationMode=dry_run".to_string(),
        });
    }
    input.config.validate()?;
    Ok(())
}

fn inspect_source(
    index: usize,
    source: &MigrationSource,
    diagnostics: &mut Vec<MigrationDiagnostic>,
    blockers: &mut Vec<MigrationBlocker>,
) -> MigrationSourceReport {
    let metadata = match fs::metadata(&source.path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => {
            blockers.push(blocker(
                "source_not_a_file",
                Some(source.kind),
                None,
                None,
                "source is not a readable SQLite file",
            ));
            return unavailable_source_report(index, source.kind, "not_a_file");
        }
        Err(_) => {
            blockers.push(blocker(
                "source_unavailable",
                Some(source.kind),
                None,
                None,
                "source is not a readable SQLite file",
            ));
            return unavailable_source_report(index, source.kind, "unavailable");
        }
    };

    let path_fingerprint = path_fingerprint(&source.path);
    let file_len = metadata.len();
    let modified_unix_seconds = metadata_modified_seconds(&metadata);
    let connection =
        match Connection::open_with_flags(&source.path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(connection) => connection,
            Err(_) => {
                blockers.push(blocker(
                    "source_unreadable",
                    Some(source.kind),
                    None,
                    None,
                    "source could not be opened as SQLite",
                ));
                return MigrationSourceReport {
                    source: source.kind,
                    source_index: index,
                    source_identity_fingerprint: fingerprint_lossless(&format!(
                        "{}:{}:{}:{}:unreadable",
                        source.kind.as_key(),
                        path_fingerprint,
                        file_len,
                        modified_unix_seconds
                    )),
                    schema_version: None,
                    counted_tables: BTreeMap::new(),
                };
            }
        };

    diagnostics.push(diagnostic(
        "source_opened",
        MigrationDiagnosticSeverity::Info,
        Some(source.kind),
        None,
        "source opened for read-only diagnostics",
    ));

    let schema_version = read_schema_version(&connection).ok().flatten();
    let mut counted_tables = BTreeMap::new();

    for table in source.kind.allowlisted_tables() {
        match table_exists(&connection, table) {
            Ok(true) => match count_table(&connection, table) {
                Ok(count) => {
                    diagnostics.push(diagnostic(
                        "source_table_counted",
                        MigrationDiagnosticSeverity::Info,
                        Some(source.kind),
                        Some(*table),
                        "allowlisted table count recorded",
                    ));
                    counted_tables.insert((*table).to_string(), count);
                    if count > 0 {
                        blockers.push(blocker(
                            "row_import_not_implemented",
                            Some(source.kind),
                            Some(*table),
                            Some(count),
                            "non-empty source table requires row import mapping before apply",
                        ));
                    }
                }
                Err(_) => diagnostics.push(diagnostic(
                    "source_table_count_failed",
                    MigrationDiagnosticSeverity::Warning,
                    Some(source.kind),
                    Some(*table),
                    "allowlisted table count could not be recorded",
                )),
            },
            Ok(false) => diagnostics.push(diagnostic(
                "source_table_missing",
                MigrationDiagnosticSeverity::Warning,
                Some(source.kind),
                Some(*table),
                "allowlisted table is absent in source",
            )),
            Err(_) => diagnostics.push(diagnostic(
                "source_table_probe_failed",
                MigrationDiagnosticSeverity::Warning,
                Some(source.kind),
                Some(*table),
                "allowlisted table presence could not be checked",
            )),
        }
    }

    let identity = SourceIdentityPayload {
        source: source.kind,
        source_index: index,
        path_fingerprint,
        file_len,
        modified_unix_seconds,
        schema_version,
        counted_tables: counted_tables.clone(),
    };

    MigrationSourceReport {
        source: source.kind,
        source_index: index,
        source_identity_fingerprint: fingerprint_json_lossy(&identity),
        schema_version,
        counted_tables,
    }
}

fn unavailable_source_report(
    index: usize,
    source: MigrationSourceKind,
    reason_code: &str,
) -> MigrationSourceReport {
    MigrationSourceReport {
        source,
        source_index: index,
        source_identity_fingerprint: fingerprint_lossless(&format!(
            "{}:{}:{}",
            source.as_key(),
            index,
            reason_code
        )),
        schema_version: None,
        counted_tables: BTreeMap::new(),
    }
}

fn read_schema_version(connection: &Connection) -> rusqlite::Result<Option<i64>> {
    if table_exists(connection, "schema_version")? {
        connection
            .query_row(
                "SELECT version FROM schema_version ORDER BY version DESC LIMIT 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
    } else {
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .map(Some)
    }
}

fn table_exists(connection: &Connection, table: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![table],
        |row| {
            let count: i64 = row.get(0)?;
            Ok(count > 0)
        },
    )
}

fn count_table(connection: &Connection, table: &str) -> rusqlite::Result<u64> {
    let sql = format!("SELECT COUNT(*) FROM \"{}\"", table.replace('"', "\"\""));
    connection.query_row(&sql, [], |row| {
        let count: i64 = row.get(0)?;
        Ok(count.max(0) as u64)
    })
}

fn provider_config_fingerprint(
    config: &AdapterConfig,
    resolved: &crate::config::ResolvedEngramPath,
) -> AdapterResult<String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ProviderConfigPayload<'a> {
        provider_mode: ProviderMode,
        tenant: &'a str,
        ward_scope_target: crate::scope::ScopeTarget,
        partition_scope_target: crate::scope::ScopeTarget,
        embedding_mode: crate::config::EmbeddingMode,
        embedding_provider: &'a crate::config::AdapterEmbeddingProviderConfig,
        sqlite_storage_layout: &'a crate::config::AdapterSqliteStorageLayout,
        migration_mode: MigrationMode,
        engram_path_fingerprint: String,
    }

    fingerprint_json(&ProviderConfigPayload {
        provider_mode: config.provider_mode,
        tenant: &config.tenant,
        ward_scope_target: config.ward_scope_target,
        partition_scope_target: config.partition_scope_target,
        embedding_mode: config.embedding_mode,
        embedding_provider: &config.embedding_provider,
        sqlite_storage_layout: &config.sqlite_storage_layout,
        migration_mode: config.migration_mode,
        engram_path_fingerprint: fingerprint_lossless(&resolved.path().to_string_lossy()),
    })
}

fn mapping_config_fingerprint(input: &MigrationInput) -> AdapterResult<String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct MappingConfigPayload<'a> {
        tenant: &'a str,
        ward_scope_target: crate::scope::ScopeTarget,
        partition_scope_target: crate::scope::ScopeTarget,
        ontology_policy: &'a str,
        taxonomy_policy: &'a str,
    }

    fingerprint_json(&MappingConfigPayload {
        tenant: &input.config.tenant,
        ward_scope_target: input.config.ward_scope_target,
        partition_scope_target: input.config.partition_scope_target,
        ontology_policy: &input.ontology_policy,
        taxonomy_policy: &input.taxonomy_policy,
    })
}

fn manifest_fingerprint(manifest: &MigrationManifest) -> AdapterResult<String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ManifestFingerprintPayload<'a> {
        manifest_version: u32,
        source_fingerprint: &'a str,
        schema_versions: &'a BTreeMap<String, i64>,
        sanitized_counts: &'a BTreeMap<String, u64>,
        adapter_version: &'a str,
        engram_revision: &'a str,
        provider_config_fingerprint: &'a str,
        mapping_config_fingerprint: &'a str,
        migration_code_version: &'a str,
    }

    fingerprint_json(&ManifestFingerprintPayload {
        manifest_version: manifest.manifest_version,
        source_fingerprint: &manifest.source_fingerprint,
        schema_versions: &manifest.schema_versions,
        sanitized_counts: &manifest.sanitized_counts,
        adapter_version: &manifest.adapter_version,
        engram_revision: &manifest.engram_revision,
        provider_config_fingerprint: &manifest.provider_config_fingerprint,
        mapping_config_fingerprint: &manifest.mapping_config_fingerprint,
        migration_code_version: &manifest.migration_code_version,
    })
}

fn source_report_key(source: MigrationSourceKind, index: usize) -> String {
    format!("{}_{}", source.as_key(), index)
}

fn metadata_modified_seconds(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn path_fingerprint(path: &Path) -> String {
    let identity = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned();
    fingerprint_lossless(&identity)
}

fn fingerprint_json<T: Serialize>(value: &T) -> AdapterResult<String> {
    serde_json::to_string(value)
        .map(|json| fingerprint_lossless(&json))
        .map_err(|_| AdapterError::Mapping {
            field: MIGRATION_COMPONENT,
            reason: "fingerprint payload cannot be encoded".to_string(),
        })
}

fn fingerprint_json_lossy<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .map(|json| fingerprint_lossless(&json))
        .unwrap_or_else(|_| fingerprint_lossless("fingerprint_encode_error"))
}

fn fingerprint_lossless(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn diagnostic(
    code: &str,
    severity: MigrationDiagnosticSeverity,
    source: Option<MigrationSourceKind>,
    table: Option<&str>,
    message: &str,
) -> MigrationDiagnostic {
    MigrationDiagnostic {
        code: code.to_string(),
        severity,
        source,
        table: table.map(str::to_string),
        message: message.to_string(),
    }
}

fn blocker(
    code: &str,
    source: Option<MigrationSourceKind>,
    table: Option<&str>,
    affected_rows: Option<u64>,
    reason: &str,
) -> MigrationBlocker {
    MigrationBlocker {
        code: code.to_string(),
        feature: AdapterFeature::Migration,
        source,
        table: table.map(str::to_string),
        affected_rows,
        reason: reason.to_string(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceIdentityPayload {
    source: MigrationSourceKind,
    source_index: usize,
    path_fingerprint: String,
    file_len: u64,
    modified_unix_seconds: u64,
    schema_version: Option<i64>,
    counted_tables: BTreeMap<String, u64>,
}
