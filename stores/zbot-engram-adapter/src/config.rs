//! Adapter configuration that is independent of AgentZero settings storage.

use std::path::{Component, Path, PathBuf};

use engram_domain::types::ScopeMappingStrategy;
use engram_integration::{
    CapabilityPolicy, EmbeddingProviderConfig as EngramEmbeddingProviderConfig, EngramConfig,
    MigrationMode as EngramMigrationMode, SqliteStorageLayout as EngramSqliteStorageLayout,
};
use serde::{Deserialize, Serialize};

use crate::{
    error::{AdapterError, AdapterResult},
    scope::{ScopeMapper, ScopeTarget},
};

/// Which backing provider the AgentZero composition root selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderMode {
    /// Current AgentZero SQLite provider remains active.
    CurrentSqlite,
    /// Engram-backed adapter provider is active.
    Engram,
}

/// How AgentZero embedding bytes are handled by the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingMode {
    /// Preserve AgentZero little-endian f32 bytes in an adapter-owned sidecar.
    PreserveBytes,
    /// Store only Engram embedding references; ranking parity must be accepted.
    EngramRefs,
    /// Disable adapter-backed semantic embedding behavior.
    Disabled,
}

/// Migration mode for import tooling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationMode {
    /// Report what would happen without writing to Engram storage.
    DryRun,
    /// Write mapped records to Engram storage.
    Apply,
}

/// SQLite file layout requested from the Engram provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AdapterSqliteStorageLayout {
    /// Engram opens one SQLite file per store family.
    MultiFileDirectory,
    /// Engram opens every SQLite-backed store against one shared file.
    SingleFile { file_name: String },
}

impl Default for AdapterSqliteStorageLayout {
    fn default() -> Self {
        Self::SingleFile {
            file_name: default_single_file_name(),
        }
    }
}

impl AdapterSqliteStorageLayout {
    fn to_engram(&self) -> EngramSqliteStorageLayout {
        match self {
            Self::MultiFileDirectory => EngramSqliteStorageLayout::MultiFileDirectory,
            Self::SingleFile { file_name } => EngramSqliteStorageLayout::SingleFile {
                file_name: file_name.clone(),
            },
        }
    }
}

/// Embedding provider identity used by Engram vector indexes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterEmbeddingProviderConfig {
    /// Provider family, for example `fastembed`, `ollama`, or `openai`.
    pub provider_type: String,
    /// Provider-specific model identifier.
    pub model: String,
    /// Vector dimensions produced by the model.
    pub dimensions: u32,
    /// Prompt profile used for embedding generation.
    pub prompt_profile: String,
    /// Normalization applied to embeddings, if any.
    #[serde(default)]
    pub normalization: Option<String>,
}

impl Default for AdapterEmbeddingProviderConfig {
    fn default() -> Self {
        Self {
            provider_type: "fastembed".to_string(),
            model: "BAAI/bge-small-en-v1.5".to_string(),
            dimensions: 384,
            prompt_profile: "query".to_string(),
            normalization: None,
        }
    }
}

/// Configuration for the AgentZero-to-Engram adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterConfig {
    /// Selected provider mode.
    pub provider_mode: ProviderMode,
    /// Engram storage directory or logical provider location.
    pub engram_path: Option<PathBuf>,
    /// Trusted zbot data root injected by the composition layer.
    ///
    /// This is intentionally not serialized or deserialized from user settings:
    /// user-controlled config may choose a DB path, but it must not redefine the
    /// confinement boundary.
    #[serde(skip)]
    pub(crate) data_root: Option<PathBuf>,
    /// Tenant used for Engram scopes.
    pub tenant: String,
    /// Where AgentZero ward IDs map in Engram scope.
    pub ward_scope_target: ScopeTarget,
    /// Where AgentZero partition IDs map in Engram scope.
    pub partition_scope_target: ScopeTarget,
    /// Embedding compatibility mode.
    pub embedding_mode: EmbeddingMode,
    /// Engram embedding provider identity used for vector-space safety.
    #[serde(default)]
    pub embedding_provider: AdapterEmbeddingProviderConfig,
    /// Engram SQLite storage layout.
    #[serde(default)]
    pub sqlite_storage_layout: AdapterSqliteStorageLayout,
    /// Migration execution mode.
    pub migration_mode: MigrationMode,
}

impl Default for AdapterConfig {
    fn default() -> Self {
        Self {
            provider_mode: ProviderMode::CurrentSqlite,
            engram_path: None,
            data_root: None,
            tenant: "agentzero".to_string(),
            ward_scope_target: ScopeTarget::Workspace,
            partition_scope_target: ScopeTarget::Workspace,
            embedding_mode: EmbeddingMode::PreserveBytes,
            embedding_provider: AdapterEmbeddingProviderConfig::default(),
            sqlite_storage_layout: AdapterSqliteStorageLayout::default(),
            migration_mode: MigrationMode::DryRun,
        }
    }
}

impl AdapterConfig {
    /// Build an Engram-mode config for a zbot data root and storage path.
    pub fn engram_for_data_root(
        data_root: impl Into<PathBuf>,
        engram_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            provider_mode: ProviderMode::Engram,
            engram_path: Some(engram_path.into()),
            ..Self::default()
        }
        .with_trusted_data_root(data_root)
    }

    /// Inject the trusted zbot data root supplied by the composition layer.
    pub fn with_trusted_data_root(mut self, data_root: impl Into<PathBuf>) -> Self {
        self.data_root = Some(data_root.into());
        self
    }

    /// Validate config before constructing stores or starting workers.
    pub fn validate(&self) -> AdapterResult<()> {
        if self.tenant.trim().is_empty() {
            return Err(AdapterError::MissingConfig { field: "tenant" });
        }
        if self.provider_mode == ProviderMode::Engram && self.engram_path.is_none() {
            return Err(AdapterError::MissingConfig {
                field: "engramPath",
            });
        }
        if self.provider_mode == ProviderMode::Engram && self.data_root.is_none() {
            return Err(AdapterError::MissingConfig { field: "dataRoot" });
        }
        if self.embedding_provider.provider_type.trim().is_empty() {
            return Err(AdapterError::MissingConfig {
                field: "embeddingProvider.providerType",
            });
        }
        if self.embedding_provider.model.trim().is_empty() {
            return Err(AdapterError::MissingConfig {
                field: "embeddingProvider.model",
            });
        }
        if self.embedding_provider.dimensions == 0 {
            return Err(AdapterError::MissingConfig {
                field: "embeddingProvider.dimensions",
            });
        }
        if self.embedding_provider.prompt_profile.trim().is_empty() {
            return Err(AdapterError::MissingConfig {
                field: "embeddingProvider.promptProfile",
            });
        }
        Ok(())
    }

    /// Build a scope mapper from the configured tenant and mapping targets.
    pub fn scope_mapper(&self) -> AdapterResult<ScopeMapper> {
        self.validate()?;
        ScopeMapper::new(
            self.tenant.clone(),
            self.ward_scope_target,
            self.partition_scope_target,
        )
    }

    /// Resolve and confine the configured Engram storage path.
    pub fn resolve_engram_path(&self) -> AdapterResult<ResolvedEngramPath> {
        self.validate()?;

        let data_root = self
            .data_root
            .as_deref()
            .ok_or(AdapterError::MissingConfig { field: "dataRoot" })?;
        let engram_path = self
            .engram_path
            .as_deref()
            .ok_or(AdapterError::MissingConfig {
                field: "engramPath",
            })?;

        resolve_confined_path(data_root, engram_path)
    }

    /// Build Engram's provider-facade config from the adapter config.
    pub fn to_engram_config(&self) -> AdapterResult<EngramConfig> {
        let resolved = self.resolve_engram_path()?;
        resolved.prepare_parent_for_provider()?;
        resolved.reject_existing_path_symlink(resolved.path())?;
        let config = EngramConfig::new(
            resolved.path().to_path_buf(),
            resolved.data_root().to_path_buf(),
            ScopeMappingStrategy::Strict,
            EngramEmbeddingProviderConfig {
                provider_type: self.embedding_provider.provider_type.clone(),
                model: self.embedding_provider.model.clone(),
                dimensions: self.embedding_provider.dimensions,
                prompt_profile: self.embedding_provider.prompt_profile.clone(),
                normalization: self.embedding_provider.normalization.clone(),
            },
            match self.migration_mode {
                MigrationMode::DryRun => EngramMigrationMode::DryRun,
                MigrationMode::Apply => EngramMigrationMode::Apply,
            },
            CapabilityPolicy::FailClosed,
        )
        .with_sqlite_storage_layout(self.sqlite_storage_layout.to_engram());
        config
            .validate()
            .map_err(|reason| AdapterError::Bootstrap {
                component: "provider_config",
                reason,
            })?;
        self.validate_provider_storage_targets(&resolved)?;
        Ok(config)
    }

    fn validate_provider_storage_targets(
        &self,
        resolved: &ResolvedEngramPath,
    ) -> AdapterResult<()> {
        match &self.sqlite_storage_layout {
            AdapterSqliteStorageLayout::MultiFileDirectory => {
                resolved.reject_existing_path_symlink(resolved.path())
            }
            AdapterSqliteStorageLayout::SingleFile { file_name } => {
                resolved.prepare_storage_dir()?;
                let path = resolved.path().join(file_name);
                resolved.reject_existing_path_symlink(&path)
            }
        }
    }

    /// Build a confined path for adapter-owned compatibility tables.
    ///
    /// In Engram's single-file SQLite layout, zbot compatibility tables live
    /// in the same database as the Engram core stores so the runtime has one
    /// database file to copy/delete/debug. In multi-file mode, compatibility
    /// tables keep their historic sidecar files.
    pub fn compatibility_store_path(&self, sidecar_file_name: &str) -> AdapterResult<PathBuf> {
        let resolved = self.resolve_engram_path()?;
        match &self.sqlite_storage_layout {
            AdapterSqliteStorageLayout::MultiFileDirectory => {
                let path = resolved.sidecar_path(sidecar_file_name)?;
                resolved.reject_existing_path_symlink(&path)?;
                Ok(path)
            }
            AdapterSqliteStorageLayout::SingleFile { file_name } => {
                // Reuse EngramConfig validation for the shared file name.
                let _ = self.to_engram_config()?;
                resolved.prepare_storage_dir()?;
                let path = resolved.path().join(file_name);
                resolved.reject_existing_path_symlink(&path)?;
                Ok(path)
            }
        }
    }
}

fn default_single_file_name() -> String {
    "engram_data.db".to_string()
}

/// Confined file path ready for Engram SQLite construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEngramPath {
    data_root: PathBuf,
    path: PathBuf,
}

impl ResolvedEngramPath {
    /// Confined DB path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Canonical zbot data root used for confinement.
    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    /// Build an adapter sidecar path inside the confined Engram storage path.
    pub fn sidecar_path(&self, file_name: &str) -> AdapterResult<PathBuf> {
        self.prepare_storage_dir()?;
        Ok(self.path.join(file_name))
    }

    fn prepare_parent_for_provider(&self) -> AdapterResult<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| AdapterError::PathNotConfined {
                field: "engramPath",
                reason: "storage path has no parent".to_string(),
            })?;
        std::fs::create_dir_all(parent).map_err(|_| AdapterError::PathNotConfined {
            field: "engramPath",
            reason: "storage parent cannot be created".to_string(),
        })?;

        let canonical_parent =
            parent
                .canonicalize()
                .map_err(|_| AdapterError::PathNotConfined {
                    field: "engramPath",
                    reason: "storage parent cannot be resolved".to_string(),
                })?;
        if !canonical_parent.starts_with(&self.data_root) {
            return Err(AdapterError::PathNotConfined {
                field: "engramPath",
                reason: "path resolves outside the zbot data root".to_string(),
            });
        }
        Ok(())
    }

    fn prepare_storage_dir(&self) -> AdapterResult<()> {
        self.prepare_parent_for_provider()?;
        self.reject_existing_path_symlink(&self.path)?;
        std::fs::create_dir_all(&self.path).map_err(|_| AdapterError::PathNotConfined {
            field: "engramPath",
            reason: "storage path cannot be created".to_string(),
        })?;
        let canonical_storage =
            self.path
                .canonicalize()
                .map_err(|_| AdapterError::PathNotConfined {
                    field: "engramPath",
                    reason: "storage path cannot be resolved".to_string(),
                })?;
        if !canonical_storage.starts_with(&self.data_root) {
            return Err(AdapterError::PathNotConfined {
                field: "engramPath",
                reason: "path resolves outside the zbot data root".to_string(),
            });
        }
        Ok(())
    }

    fn reject_existing_path_symlink(&self, path: &Path) -> AdapterResult<()> {
        let meta = match std::fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => {
                return Err(AdapterError::PathNotConfined {
                    field: "engramPath",
                    reason: "storage path cannot be inspected".to_string(),
                });
            }
        };
        if meta.file_type().is_symlink() {
            return Err(AdapterError::PathNotConfined {
                field: "engramPath",
                reason: "storage path symlink is not allowed".to_string(),
            });
        }
        let canonical = path
            .canonicalize()
            .map_err(|_| AdapterError::PathNotConfined {
                field: "engramPath",
                reason: "storage path cannot be resolved".to_string(),
            })?;
        if !canonical.starts_with(&self.data_root) {
            return Err(AdapterError::PathNotConfined {
                field: "engramPath",
                reason: "path resolves outside the zbot data root".to_string(),
            });
        }
        Ok(())
    }
}

fn resolve_confined_path(data_root: &Path, requested: &Path) -> AdapterResult<ResolvedEngramPath> {
    let data_root = data_root
        .canonicalize()
        .map_err(|_| AdapterError::PathNotConfined {
            field: "dataRoot",
            reason: "root does not exist or cannot be resolved".to_string(),
        })?;

    reject_unsafe_components("engramPath", requested)?;

    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        data_root.join(requested)
    };

    let parent = candidate
        .parent()
        .ok_or_else(|| AdapterError::PathNotConfined {
            field: "engramPath",
            reason: "database path has no parent".to_string(),
        })?;

    let resolved_parent = resolve_existing_parent(parent)?;
    if !resolved_parent.starts_with(&data_root) {
        return Err(AdapterError::PathNotConfined {
            field: "engramPath",
            reason: "path resolves outside the zbot data root".to_string(),
        });
    }

    let file_name = candidate
        .file_name()
        .ok_or_else(|| AdapterError::PathNotConfined {
            field: "engramPath",
            reason: "database path has no file name".to_string(),
        })?;

    Ok(ResolvedEngramPath {
        data_root,
        path: resolved_parent.join(file_name),
    })
}

fn reject_unsafe_components(field: &'static str, path: &Path) -> AdapterResult<()> {
    for component in path.components() {
        match component {
            Component::ParentDir => {
                return Err(AdapterError::PathNotConfined {
                    field,
                    reason: "parent directory traversal is not allowed".to_string(),
                });
            }
            Component::Prefix(_) => {
                return Err(AdapterError::PathNotConfined {
                    field,
                    reason: "platform path prefixes are not allowed".to_string(),
                });
            }
            _ => {}
        }
    }
    Ok(())
}

fn resolve_existing_parent(parent: &Path) -> AdapterResult<PathBuf> {
    let mut cursor = parent;
    let mut missing = Vec::new();

    while !cursor.exists() {
        let name = cursor
            .file_name()
            .ok_or_else(|| AdapterError::PathNotConfined {
                field: "engramPath",
                reason: "database parent cannot be resolved".to_string(),
            })?
            .to_os_string();
        missing.push(name);
        cursor = cursor
            .parent()
            .ok_or_else(|| AdapterError::PathNotConfined {
                field: "engramPath",
                reason: "database parent cannot be resolved".to_string(),
            })?;
    }

    let mut resolved = cursor
        .canonicalize()
        .map_err(|_| AdapterError::PathNotConfined {
            field: "engramPath",
            reason: "database parent cannot be resolved".to_string(),
        })?;

    for component in missing.into_iter().rev() {
        resolved.push(component);
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_safe_current_provider() {
        let config = AdapterConfig::default();

        assert_eq!(config.provider_mode, ProviderMode::CurrentSqlite);
        assert_eq!(config.embedding_mode, EmbeddingMode::PreserveBytes);
        assert_eq!(config.migration_mode, MigrationMode::DryRun);
        assert_eq!(config.ward_scope_target, ScopeTarget::Workspace);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn engram_mode_requires_path() {
        let config = AdapterConfig {
            provider_mode: ProviderMode::Engram,
            data_root: Some(PathBuf::from("data")),
            ..AdapterConfig::default()
        };

        assert_eq!(
            config.validate(),
            Err(AdapterError::MissingConfig {
                field: "engramPath"
            })
        );
    }

    #[test]
    fn engram_mode_requires_data_root() {
        let config = AdapterConfig {
            provider_mode: ProviderMode::Engram,
            engram_path: Some(PathBuf::from("engram.db")),
            ..AdapterConfig::default()
        };

        assert_eq!(
            config.validate(),
            Err(AdapterError::MissingConfig { field: "dataRoot" })
        );
    }

    #[test]
    fn config_builds_scope_mapper() {
        let config = AdapterConfig {
            tenant: "tenant-a".to_string(),
            ..AdapterConfig::default()
        };

        let mapper = config.scope_mapper().expect("mapper");
        let scope = mapper.ward_scope("ward-a").expect("scope");

        assert_eq!(scope.tenant, "tenant-a");
        assert_eq!(scope.workspace.as_deref(), Some("ward-a"));
    }
}
