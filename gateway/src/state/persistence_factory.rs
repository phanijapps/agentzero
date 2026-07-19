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
    AdapterConfig, AdapterEmbeddingProviderConfig, AdapterSqliteStorageLayout,
    AllowUnclassifiedPolicy, EmbeddingMode, EngramBeliefStore, EngramKnowledgeGraphStore,
    EngramMemoryFactStore, EngramProvider, EngramSidecarStores, EngramTaxonomyRecallExpander,
    EngramWikiStore, GovernanceCapabilityHealth, GovernanceOverlay, GovernancePolicy,
    GovernanceSelection, MigrationMode, ScopeTarget, SkosExpansionPolicy, ValidationMode,
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
    /// Present only when an active governance selection names a taxonomy.
    pub taxonomy_expander: Option<Arc<dyn zbot_stores_traits::RecallTaxonomyExpander>>,
    pub governance_health: GovernanceCapabilityHealth,
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
    let kg_store_impl = Arc::new(
        EngramKnowledgeGraphStore::from_provider(config.clone(), &provider)
            .map_err(|error| error.to_string())?,
    );
    let governance_findings = kg_store_impl
        .list_governance_findings(None, 500)
        .map_err(|error| error.to_string())?;
    let governance_health = GovernanceCapabilityHealth::from_config_and_findings(
        &config,
        provider.governance_bootstrap(),
        &governance_findings,
    );
    let kg_store: Arc<dyn KnowledgeGraphStore> = kg_store_impl;
    let wiki_store: Arc<dyn zbot_stores_traits::WikiStore> = Arc::new(
        EngramWikiStore::from_provider(config.clone(), &provider)
            .map_err(|error| error.to_string())?,
    );
    let beliefs = Arc::new(
        EngramBeliefStore::from_provider(config.clone(), &provider)
            .map_err(|error| error.to_string())?,
    );
    let sidecars = Arc::new(
        EngramSidecarStores::from_provider(config.clone(), &provider)
            .map_err(|error| error.to_string())?,
    );
    let taxonomy_expander = if config.governance.has_taxonomy_selection() {
        Some(Arc::new(
            EngramTaxonomyRecallExpander::from_provider(config.clone(), &provider)
                .map_err(|error| error.to_string())?,
        )
            as Arc<dyn zbot_stores_traits::RecallTaxonomyExpander>)
    } else {
        None
    };

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
        taxonomy_expander,
        governance_health,
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
    )
    .with_trusted_config_root(paths.config_dir());
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
    config.governance = map_governance_policy(&settings.governance);
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

fn map_governance_policy(settings: &gateway_memory::MemoryGovernanceSettings) -> GovernancePolicy {
    GovernancePolicy {
        ontology_definition_paths: settings
            .ontology_definition_paths
            .iter()
            .map(PathBuf::from)
            .collect(),
        taxonomy_definition_paths: settings
            .taxonomy_definition_paths
            .iter()
            .map(PathBuf::from)
            .collect(),
        default_selection: map_governance_selection(&settings.default_selection),
        overlays: settings
            .overlays
            .iter()
            .map(|overlay| GovernanceOverlay {
                ward_id: overlay.ward_id.clone(),
                project_id: overlay.project_id.clone(),
                session_id: overlay.session_id.clone(),
                source_id: overlay.source_id.clone(),
                task_id: overlay.task_id.clone(),
                selection: map_governance_selection(&overlay.selection),
            })
            .collect(),
        validation_mode: match settings.validation_mode {
            gateway_memory::MemoryGovernanceValidationMode::Advisory => ValidationMode::Advisory,
            gateway_memory::MemoryGovernanceValidationMode::Disabled => ValidationMode::Disabled,
        },
        allow_unclassified: match settings.allow_unclassified {
            gateway_memory::MemoryAllowUnclassifiedPolicy::Allow => AllowUnclassifiedPolicy::Allow,
            gateway_memory::MemoryAllowUnclassifiedPolicy::Warn => AllowUnclassifiedPolicy::Warn,
        },
        skos_expansion: SkosExpansionPolicy {
            max_depth: settings.skos_expansion.max_depth,
            max_fan_out: settings.skos_expansion.max_fan_out,
            max_candidates: settings.skos_expansion.max_candidates,
        },
    }
}

fn map_governance_selection(
    selection: &gateway_memory::MemoryGovernanceSelection,
) -> GovernanceSelection {
    GovernanceSelection {
        ontology_ids: selection.ontology_ids.clone(),
        taxonomy_scheme_ids: selection.taxonomy_scheme_ids.clone(),
    }
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
    use zbot_stores_traits::RecallTaxonomyExpansionRequest;

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
        std::fs::create_dir_all(paths.config_dir()).unwrap();
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
        assert!(config.governance.is_inert());
        assert_eq!(
            config.sqlite_storage_layout,
            AdapterSqliteStorageLayout::SingleFile {
                file_name: "engram_data.db".to_string()
            }
        );
    }

    #[test]
    fn adapter_config_resolves_governance_paths_under_vault_config_dir() {
        let dir = TempDir::new().unwrap();
        let paths = VaultPaths::new(dir.path().to_path_buf());
        std::fs::create_dir_all(paths.data_dir()).unwrap();
        std::fs::create_dir_all(paths.config_dir().join("governance")).unwrap();
        std::fs::write(
            paths.config_dir().join("governance/base-ontology.json"),
            "{}",
        )
        .unwrap();
        std::fs::write(
            paths.config_dir().join("governance/base-taxonomy.json"),
            "{}",
        )
        .unwrap();
        let settings = gateway_memory::MemoryProviderSettings {
            governance: gateway_memory::MemoryGovernanceSettings {
                ontology_definition_paths: vec!["governance/base-ontology.json".to_string()],
                taxonomy_definition_paths: vec!["governance/base-taxonomy.json".to_string()],
                default_selection: gateway_memory::MemoryGovernanceSelection {
                    ontology_ids: vec!["zbot.base:v1".to_string()],
                    taxonomy_scheme_ids: vec!["zbot.tasks:v1".to_string()],
                },
                ..gateway_memory::MemoryGovernanceSettings::default()
            },
            ..gateway_memory::MemoryProviderSettings::default()
        };

        let config = adapter_config_from_memory_provider_settings(&paths, &settings)
            .expect("adapter config");
        let resolved = config
            .resolve_governance_definition_paths()
            .expect("governance paths");

        assert_eq!(
            resolved.ontology_definition_paths,
            vec![paths
                .config_dir()
                .join("governance")
                .join("base-ontology.json")]
        );
        assert_eq!(
            resolved.taxonomy_definition_paths,
            vec![paths
                .config_dir()
                .join("governance")
                .join("base-taxonomy.json")]
        );
        assert_eq!(
            config.governance.default_selection.ontology_ids,
            vec!["zbot.base:v1"]
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
        assert!(bundle.taxonomy_expander.is_none());
        assert!(paths.data_dir().join("engram").exists());
    }

    #[tokio::test]
    async fn configured_taxonomy_is_wired_into_the_engram_bundle() {
        let dir = TempDir::new().unwrap();
        let paths = VaultPaths::new(dir.path().to_path_buf());
        std::fs::create_dir_all(paths.data_dir()).unwrap();
        let governance_dir = paths.config_dir().join("governance");
        std::fs::create_dir_all(&governance_dir).unwrap();
        std::fs::write(
            governance_dir.join("base-taxonomy.json"),
            r#"{
              "kind": "zbot.skos_taxonomy",
              "schemaVersion": 1,
              "schemeId": "zbot.general:v1",
              "label": "Zbot General Taxonomy",
              "concepts": [
                { "id": "memory", "prefLabel": "Memory", "altLabels": ["recall"] },
                { "id": "graph", "prefLabel": "Knowledge Graph", "altLabels": ["kg"], "broader": ["memory"] }
              ]
            }"#,
        )
        .unwrap();
        let settings = gateway_memory::MemoryProviderSettings {
            governance: gateway_memory::MemoryGovernanceSettings {
                taxonomy_definition_paths: vec!["governance/base-taxonomy.json".to_string()],
                default_selection: gateway_memory::MemoryGovernanceSelection {
                    ontology_ids: Vec::new(),
                    taxonomy_scheme_ids: vec!["zbot.general:v1".to_string()],
                },
                ..gateway_memory::MemoryGovernanceSettings::default()
            },
            ..gateway_memory::MemoryProviderSettings::default()
        };

        let bundle = build_engram_store_bundle(&paths, &settings, None).expect("selection");
        let expander = bundle
            .taxonomy_expander
            .expect("configured taxonomy expander");
        let expansion = expander
            .expand_recall_query(RecallTaxonomyExpansionRequest {
                query: "kg recall".to_string(),
                ward_id: None,
                session_id: None,
                max_depth: 1,
                max_fan_out: 8,
                max_candidates: 8,
            })
            .await
            .expect("expansion");

        assert!(expansion.expanded_query.contains("Knowledge Graph"));
        assert!(expansion
            .candidates
            .iter()
            .any(|candidate| candidate.relation.as_deref() == Some("broader")));

        let mut recall = gateway_memory::MemoryRecall::new(
            None,
            Arc::new(gateway_memory::RecallConfig::default()),
        );
        recall.set_taxonomy_expander(expander);
        let outcome = recall
            .recall_unified_outcome("agent-a", "kg recall", None, &[], 8)
            .await
            .expect("unified recall");
        let trace = outcome.taxonomy_expansion.expect("taxonomy trace");
        assert!(trace.retrieval_query.contains("Knowledge Graph"));
        assert!(trace.candidates.iter().any(|candidate| candidate.relation
            == gateway_memory::UnifiedRecallTaxonomyRelation::Broader));
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
