//! # Persistence factory
//!
//! Centralized construction of persistence-layer trait objects.
//!
//! AppState consumes `Arc<dyn KnowledgeGraphStore>` and `Arc<dyn
//! MemoryFactStore>` rather than the concrete SQLite repos so HTTP
//! handlers and sleep jobs don't need to know which backend they got.
//! Runtime semantic memory/knowledge uses Engram; SQLite stays with
//! conversations, execution state, outbox, tests, and migration readers.

use std::{path::PathBuf, sync::Arc};

use agent_runtime::llm::embedding::EmbeddingClient;
use gateway_services::VaultPaths;
use zbot_engram_adapter::{
    AdapterConfig, AdapterEmbeddingProviderConfig, AdapterSqliteStorageLayout, EmbeddingMode,
    EngramBeliefStore, EngramKnowledgeGraphStore, EngramMemoryFactStore, EngramProvider,
    EngramSidecarStores, EngramWikiStore, MigrationMode, ScopeTarget,
};
use zbot_stores::{KnowledgeGraphStore, MemoryFactStore};

/// Engram trait-object bundle used by `AppState` when configured.
#[derive(Clone)]
pub struct EngramStoreBundle {
    pub memory_store: Arc<dyn MemoryFactStore>,
    pub kg_store: Arc<dyn KnowledgeGraphStore>,
    pub wiki_store: Arc<dyn zbot_stores_traits::WikiStore>,
    pub procedure_store: Arc<dyn zbot_stores_traits::ProcedureStore>,
    pub episode_store: Arc<dyn zbot_stores_traits::EpisodeStore>,
    pub kg_episode_store: Arc<dyn zbot_stores_traits::KgEpisodeStore>,
    pub compaction_store: Arc<dyn zbot_stores_traits::CompactionStore>,
    pub goal_store: Arc<dyn zbot_stores_traits::GoalStore>,
    pub belief_store: Arc<dyn zbot_stores_traits::BeliefStore>,
    pub belief_contradiction_store: Arc<dyn zbot_stores_traits::BeliefContradictionStore>,
}

/// Build Engram trait-object stores for runtime semantic memory/knowledge.
pub fn build_engram_store_bundle(
    paths: &VaultPaths,
    settings: &gateway_memory::MemoryProviderSettings,
    embedding_client: Option<Arc<dyn EmbeddingClient>>,
) -> Result<EngramStoreBundle, String> {
    let config = adapter_config_from_memory_provider_settings(paths, settings)?;
    let _ = config
        .compatibility_store_path("zbot-memory-facts.sqlite")
        .map_err(|error| error.to_string())?;
    let provider = EngramProvider::open(config.clone()).map_err(|error| error.to_string())?;

    let memory_store: Arc<dyn MemoryFactStore> = Arc::new(
        EngramMemoryFactStore::from_provider_with_embedding_client(
            config.clone(),
            &provider,
            embedding_client,
        )
        .map_err(|error| error.to_string())?,
    );
    let kg_store: Arc<dyn KnowledgeGraphStore> = Arc::new(
        EngramKnowledgeGraphStore::from_provider(config.clone(), &provider)
            .map_err(|error| error.to_string())?,
    );
    let wiki_store: Arc<dyn zbot_stores_traits::WikiStore> = Arc::new(
        EngramWikiStore::from_provider(config.clone(), &provider)
            .map_err(|error| error.to_string())?,
    );
    let beliefs = Arc::new(
        EngramBeliefStore::from_provider(config.clone(), &provider)
            .map_err(|error| error.to_string())?,
    );
    let sidecars =
        Arc::new(EngramSidecarStores::open(config.clone()).map_err(|error| error.to_string())?);

    Ok(EngramStoreBundle {
        memory_store,
        kg_store,
        wiki_store,
        procedure_store: sidecars.clone(),
        episode_store: sidecars.clone(),
        kg_episode_store: sidecars.clone(),
        compaction_store: sidecars.clone(),
        goal_store: sidecars.clone(),
        // Distillation run status stays on conversations.db so existing
        // gateway/UI status contracts remain in one place.
        belief_store: beliefs.clone(),
        belief_contradiction_store: beliefs,
    })
}

/// Translate additive gateway settings into adapter config. The trusted data
/// root always comes from `VaultPaths`; user settings cannot redefine it.
pub fn adapter_config_from_memory_provider_settings(
    paths: &VaultPaths,
    settings: &gateway_memory::MemoryProviderSettings,
) -> Result<AdapterConfig, String> {
    let mut config = AdapterConfig::engram_for_data_root(
        paths.data_dir(),
        PathBuf::from(settings.engram_path.clone()),
    );
    config.tenant = settings.tenant.clone();
    config.ward_scope_target = map_scope_target(settings.ward_scope_target);
    config.partition_scope_target = map_scope_target(settings.partition_scope_target);
    config.embedding_mode = match settings.embedding_mode {
        gateway_memory::MemoryEmbeddingMode::PreserveBytes => EmbeddingMode::PreserveBytes,
        gateway_memory::MemoryEmbeddingMode::EngramRefs => EmbeddingMode::EngramRefs,
        gateway_memory::MemoryEmbeddingMode::Disabled => EmbeddingMode::Disabled,
    };
    config.embedding_provider = AdapterEmbeddingProviderConfig {
        provider_type: settings.embedding_provider.provider_type.clone(),
        model: settings.embedding_provider.model.clone(),
        dimensions: settings.embedding_provider.dimensions,
        prompt_profile: settings.embedding_provider.prompt_profile.clone(),
        normalization: settings.embedding_provider.normalization.clone(),
    };
    config.sqlite_storage_layout = map_sqlite_storage_layout(&settings.sqlite_storage_layout);
    config.migration_mode = match settings.migration_mode {
        gateway_memory::MemoryMigrationMode::DryRun => MigrationMode::DryRun,
        gateway_memory::MemoryMigrationMode::Apply => MigrationMode::Apply,
    };
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

fn map_sqlite_storage_layout(
    layout: &gateway_memory::MemorySqliteStorageLayout,
) -> AdapterSqliteStorageLayout {
    match layout {
        gateway_memory::MemorySqliteStorageLayout::MultiFileDirectory => {
            AdapterSqliteStorageLayout::MultiFileDirectory
        }
        gateway_memory::MemorySqliteStorageLayout::SingleFile { file_name } => {
            AdapterSqliteStorageLayout::SingleFile {
                file_name: file_name.clone(),
            }
        }
    }
}

fn map_scope_target(target: gateway_memory::MemoryScopeTarget) -> ScopeTarget {
    match target {
        gateway_memory::MemoryScopeTarget::Workspace => ScopeTarget::Workspace,
        gateway_memory::MemoryScopeTarget::Environment => ScopeTarget::Environment,
        gateway_memory::MemoryScopeTarget::Subject => ScopeTarget::Subject,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::llm::embedding::EmbeddingError;
    use async_trait::async_trait;
    use gateway_services::VaultPaths;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;
    use zbot_engram_adapter::ProviderMode;

    struct RecordingEmbedder {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl EmbeddingClient for RecordingEmbedder {
        async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            self.calls.fetch_add(texts.len(), Ordering::SeqCst);
            Ok(texts.iter().map(|_| vec![1.0_f32, 0.0]).collect())
        }

        fn dimensions(&self) -> usize {
            2
        }

        fn model_name(&self) -> String {
            "recording".to_string()
        }

        fn provider_type(&self) -> String {
            "fastembed".to_string()
        }
    }

    #[test]
    fn adapter_config_uses_vault_data_dir_as_trusted_root() {
        let dir = TempDir::new().unwrap();
        let paths = VaultPaths::new(dir.path().to_path_buf());
        std::fs::create_dir_all(paths.data_dir()).unwrap();
        let settings = gateway_memory::MemoryProviderSettings {
            mode: gateway_memory::MemoryProviderMode::Engram,
            engram_path: "engram".to_string(),
            tenant: "tenant-a".to_string(),
            ward_scope_target: gateway_memory::MemoryScopeTarget::Environment,
            partition_scope_target: gateway_memory::MemoryScopeTarget::Subject,
            embedding_mode: gateway_memory::MemoryEmbeddingMode::EngramRefs,
            migration_mode: gateway_memory::MemoryMigrationMode::DryRun,
            ..gateway_memory::MemoryProviderSettings::default()
        };

        let config = adapter_config_from_memory_provider_settings(&paths, &settings)
            .expect("adapter config");
        let resolved = config.resolve_engram_path().expect("resolved");

        assert!(resolved.path().starts_with(paths.data_dir()));
        assert_eq!(config.provider_mode, ProviderMode::Engram);
        assert_eq!(config.tenant, "tenant-a");
        assert_eq!(config.ward_scope_target, ScopeTarget::Environment);
        assert_eq!(config.partition_scope_target, ScopeTarget::Subject);
        assert_eq!(config.embedding_mode, EmbeddingMode::EngramRefs);
        assert_eq!(
            config.sqlite_storage_layout,
            AdapterSqliteStorageLayout::SingleFile {
                file_name: "engram_data.db".to_string()
            }
        );
    }

    #[test]
    fn default_provider_selection_builds_engram_bundle() {
        let dir = TempDir::new().unwrap();
        let paths = VaultPaths::new(dir.path().to_path_buf());
        std::fs::create_dir_all(paths.data_dir()).unwrap();

        let bundle = build_engram_store_bundle(
            &paths,
            &gateway_memory::MemoryProviderSettings::default(),
            None,
        )
        .expect("selection");

        assert!(Arc::strong_count(&bundle.memory_store) >= 1);
        assert!(paths.data_dir().join("engram").exists());
    }

    #[test]
    fn engram_provider_selection_builds_trait_bundle() {
        let dir = TempDir::new().unwrap();
        let paths = VaultPaths::new(dir.path().to_path_buf());
        std::fs::create_dir_all(paths.data_dir()).unwrap();
        let settings = gateway_memory::MemoryProviderSettings {
            mode: gateway_memory::MemoryProviderMode::Engram,
            ..gateway_memory::MemoryProviderSettings::default()
        };

        let bundle = build_engram_store_bundle(&paths, &settings, None).expect("selection");

        assert!(Arc::strong_count(&bundle.memory_store) >= 1);
        assert!(Arc::strong_count(&bundle.kg_store) >= 1);
        assert!(Arc::strong_count(&bundle.wiki_store) >= 1);
        assert!(Arc::strong_count(&bundle.procedure_store) >= 1);
        assert!(paths.data_dir().join("engram").exists());
        assert!(paths
            .data_dir()
            .join("engram")
            .join("engram_data.db")
            .exists());
        let engram_dir = paths.data_dir().join("engram");
        for file_name in [
            "memory.db",
            "knowledge.db",
            "belief.db",
            "hierarchy.db",
            "vectors.db",
            "zbot-memory-facts.sqlite",
            "zbot-knowledge-graph.sqlite",
            "zbot-wiki.sqlite",
            "zbot-beliefs.sqlite",
            "zbot-sidecars.sqlite",
        ] {
            assert!(
                !engram_dir.join(file_name).exists(),
                "{file_name} should be folded into engram_data.db in single-file mode"
            );
        }
    }

    #[tokio::test]
    async fn engram_bundle_passes_live_embedding_client_to_memory_store() {
        let dir = TempDir::new().unwrap();
        let paths = VaultPaths::new(dir.path().to_path_buf());
        std::fs::create_dir_all(paths.data_dir()).unwrap();
        let embedder = Arc::new(RecordingEmbedder {
            calls: AtomicUsize::new(0),
        });
        let embedding_client: Arc<dyn EmbeddingClient> = embedder.clone();
        let settings = gateway_memory::MemoryProviderSettings {
            embedding_provider: gateway_memory::MemoryEmbeddingProviderSettings {
                model: "recording".to_string(),
                dimensions: 2,
                ..gateway_memory::MemoryEmbeddingProviderSettings::default()
            },
            ..gateway_memory::MemoryProviderSettings::default()
        };
        let bundle = build_engram_store_bundle(&paths, &settings, Some(embedding_client))
            .expect("selection");

        bundle
            .memory_store
            .save_fact(
                "agent-a",
                "domain",
                "finance.amd.valuation_methodology",
                "AMD valuation analysis uses relative valuation methodology",
                0.9,
                None,
                None,
            )
            .await
            .expect("save");
        let recalled = bundle
            .memory_store
            .recall_facts_prioritized(
                "agent-a",
                "academic paper review methodology research analysis critical evaluation",
                5,
                None,
            )
            .await
            .expect("recall");
        let results = recalled
            .get("results")
            .and_then(serde_json::Value::as_array)
            .expect("recall envelope results");
        assert!(
            results.is_empty(),
            "composition-root Engram memory store must not broad-fallback on generic lexical content: {recalled:?}"
        );
        assert_eq!(recalled["degraded"], false);
        assert_eq!(
            embedder.calls.load(Ordering::SeqCst),
            1,
            "composition root must pass the live embedding client into Engram memory recall"
        );
    }
}
