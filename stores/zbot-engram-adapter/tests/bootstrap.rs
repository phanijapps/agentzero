use engram_domain::types::ScopeMappingStrategy;
use engram_integration::{
    CapabilityPolicy, MigrationMode as EngramMigrationMode, SqliteStorageLayout,
};
use zbot_engram_adapter::{
    AdapterConfig, AdapterEmbeddingProviderConfig, AdapterErrorKind, AdapterFeature,
    AdapterSqliteStorageLayout, EngramMemoryFactStore, EngramProvider, GovernancePolicy,
    GovernanceSelection,
};
use zbot_stores_traits::MemoryFactStore;

// STUB: AC1/AC2/AC3 - adapter config maps to Engram's provider facade config.
#[test]
fn adapter_config_maps_to_engram_provider_config() {
    let root = tempfile::tempdir().expect("root");
    std::fs::create_dir_all(root.path().join("data")).expect("data dir");
    let mut config = AdapterConfig::engram_for_data_root(root.path(), "data/engram");
    config.embedding_provider = AdapterEmbeddingProviderConfig {
        provider_type: "ollama".to_string(),
        model: "nomic-embed-text".to_string(),
        dimensions: 768,
        prompt_profile: "query".to_string(),
        normalization: Some("l2".to_string()),
    };

    let engram_config = config.to_engram_config().expect("engram config");

    assert!(engram_config
        .storage_path
        .starts_with(root.path().canonicalize().expect("canonical root")));
    assert_eq!(
        engram_config.trusted_root,
        root.path().canonicalize().expect("canonical root")
    );
    assert_eq!(engram_config.scope_policy, ScopeMappingStrategy::Strict);
    assert_eq!(engram_config.embedding_provider.provider_type, "ollama");
    assert_eq!(engram_config.embedding_provider.model, "nomic-embed-text");
    assert_eq!(engram_config.embedding_provider.dimensions, 768);
    assert_eq!(engram_config.embedding_provider.prompt_profile, "query");
    assert_eq!(
        engram_config.embedding_provider.normalization.as_deref(),
        Some("l2")
    );
    assert_eq!(engram_config.migration_mode, EngramMigrationMode::DryRun);
    assert_eq!(
        engram_config.capability_policy,
        CapabilityPolicy::FailClosed
    );
    assert_eq!(
        engram_config.sqlite_storage_layout,
        SqliteStorageLayout::SingleFile {
            file_name: "engram_data.db".to_string()
        }
    );
}

#[test]
fn adapter_config_maps_custom_single_file_layout_to_engram() {
    let root = tempfile::tempdir().expect("root");
    let mut config = AdapterConfig::engram_for_data_root(root.path(), "engram");
    config.sqlite_storage_layout = AdapterSqliteStorageLayout::SingleFile {
        file_name: "agent_memory.sqlite".to_string(),
    };

    let engram_config = config.to_engram_config().expect("engram config");

    assert_eq!(
        engram_config.sqlite_storage_layout,
        SqliteStorageLayout::SingleFile {
            file_name: "agent_memory.sqlite".to_string()
        }
    );
}

#[test]
fn adapter_config_rejects_invalid_single_file_layout() {
    let root = tempfile::tempdir().expect("root");
    let mut config = AdapterConfig::engram_for_data_root(root.path(), "engram");
    config.sqlite_storage_layout = AdapterSqliteStorageLayout::SingleFile {
        file_name: "../engram_data.db".to_string(),
    };

    let err = config
        .to_engram_config()
        .expect_err("layout must be validated before bootstrap");

    assert_eq!(err.kind(), AdapterErrorKind::Bootstrap);
}

// STUB: AC1/AC3/AC4 - bootstrap uses Engram provider facade and adapter gates.
#[test]
fn bootstrap_uses_engram_provider_facade_and_preserves_adapter_gates() {
    let root = tempfile::tempdir().expect("root");
    let config = AdapterConfig::engram_for_data_root(root.path(), "engram");

    let provider = EngramProvider::open(config).expect("provider");
    let opened = provider.opened_components();

    assert!(opened.contains(&"memory"));
    assert!(opened.contains(&"knowledge"));
    assert!(opened.contains(&"beliefs"));
    assert!(opened.contains(&"hierarchy"));
    assert!(opened.contains(&"migration"));
    assert!(opened.contains(&"provenance"));
    assert!(opened.contains(&"batch"));
    assert!(opened.contains(&"engram_unified_recall"));
    assert!(opened.contains(&"observability"));
    assert!(provider.upstream_capabilities().memory_supported());
    assert!(provider.upstream_capabilities().knowledge_supported());
    assert!(provider.upstream_capabilities().beliefs_supported());
    assert!(provider.upstream_capabilities().migration_supported());
    assert!(provider
        .upstream_capabilities()
        .episodes_evidence_supported());
    assert!(provider.upstream_capabilities().atomic_batch_supported());
    assert!(provider.upstream_capabilities().unified_recall_supported());
    assert!(provider.upstream_capabilities().observability_supported());
    assert!(!provider.upstream_capabilities().retrieval_supported());

    assert!(provider
        .capabilities()
        .supports(AdapterFeature::MemoryFacts));
    assert!(provider.capabilities().supports(AdapterFeature::Wiki));
    assert!(provider
        .capabilities()
        .supports(AdapterFeature::KnowledgeGraph));
    assert!(provider.capabilities().supports(AdapterFeature::Beliefs));
    assert!(provider
        .capabilities()
        .supports(AdapterFeature::Contradictions));
    assert!(provider.capabilities().supports(AdapterFeature::Hierarchy));
    assert!(!provider.capabilities().supports(AdapterFeature::Recall));
    assert!(!provider.capabilities().supports(AdapterFeature::Migration));
    assert!(!provider.capabilities().supports(AdapterFeature::Auxiliary));
}

#[test]
fn governance_bootstrap_is_idempotent_and_uses_single_file_layout() {
    let root = tempfile::tempdir().expect("root");
    let mut config = AdapterConfig::engram_for_data_root(root.path(), "engram");
    config.governance = GovernancePolicy {
        default_selection: GovernanceSelection {
            ontology_ids: vec!["zbot.base:v1".to_string()],
            taxonomy_scheme_ids: vec!["zbot.general:v1".to_string()],
        },
        ..GovernancePolicy::default()
    };

    let first = EngramProvider::open(config.clone()).expect("first provider");
    let second = EngramProvider::open(config).expect("second provider");
    let first_report = first
        .governance_bootstrap()
        .expect("first governance bootstrap");
    let second_report = second
        .governance_bootstrap()
        .expect("second governance bootstrap");

    assert_eq!(first_report, second_report);
    assert_eq!(first_report.ontology_id, "zbot.base:v1");
    assert_eq!(first_report.taxonomy_scheme_id, "zbot.general:v1");
    assert!(first_report.class_count > 0);
    assert!(first_report.property_count > 0);
    assert!(first_report.concept_count > 0);
    assert!(first.opened_components().contains(&"ontology"));
    assert!(first.opened_components().contains(&"taxonomy"));

    let engram_dir = root.path().join("engram");
    assert!(engram_dir.join("engram_data.db").exists());
    for file_name in ["ontology.db", "taxonomy.db", "knowledge.db"] {
        assert!(
            !engram_dir.join(file_name).exists(),
            "{file_name} should be folded into engram_data.db"
        );
    }
}

// STUB: AC2 - configured paths are confined before provider bootstrap.
#[test]
#[cfg(unix)]
fn engram_path_rejects_symlink_escape() {
    let root = tempfile::tempdir().expect("root");
    let outside = tempfile::tempdir().expect("outside");
    std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).expect("symlink");
    let config = AdapterConfig::engram_for_data_root(root.path(), "escape/engram");

    let err = config
        .resolve_engram_path()
        .expect_err("must reject escape");

    assert_eq!(err.kind(), AdapterErrorKind::PathNotConfined);
}

// STUB: AC2 - traversal never escapes the trusted data root.
#[test]
fn engram_path_rejects_parent_traversal() {
    let root = tempfile::tempdir().expect("root");
    let config = AdapterConfig::engram_for_data_root(root.path(), "../engram");

    let err = config
        .resolve_engram_path()
        .expect_err("must reject traversal");

    assert_eq!(err.kind(), AdapterErrorKind::PathNotConfined);
}

// STUB: AC2 - nonexistent trusted roots are rejected before any provider opens.
#[test]
fn engram_path_rejects_nonexistent_data_root() {
    let root = tempfile::tempdir().expect("root");
    let missing_root = root.path().join("missing");
    let config = AdapterConfig::engram_for_data_root(&missing_root, "engram");

    let err = config
        .resolve_engram_path()
        .expect_err("must reject missing root");

    assert_eq!(err.kind(), AdapterErrorKind::PathNotConfined);
}

// STUB: AC2 - existing storage-path symlinks cannot escape the data root.
#[test]
#[cfg(unix)]
fn provider_bootstrap_rejects_existing_storage_symlink_escape() {
    let root = tempfile::tempdir().expect("root");
    let outside = tempfile::tempdir().expect("outside");
    std::os::unix::fs::symlink(outside.path(), root.path().join("engram")).expect("symlink");
    let config = AdapterConfig::engram_for_data_root(root.path(), "engram");

    let Err(err) = EngramProvider::open(config) else {
        panic!("must reject storage symlink escape");
    };

    assert_eq!(err.kind(), AdapterErrorKind::PathNotConfined);
}

#[test]
#[cfg(unix)]
fn compatibility_store_rejects_existing_single_file_symlink_escape() {
    let root = tempfile::tempdir().expect("root");
    let outside = tempfile::tempdir().expect("outside");
    std::fs::create_dir_all(root.path().join("engram")).expect("engram dir");
    std::os::unix::fs::symlink(
        outside.path().join("outside.db"),
        root.path().join("engram").join("engram_data.db"),
    )
    .expect("symlink");
    let config = AdapterConfig::engram_for_data_root(root.path(), "engram");

    let err = config
        .compatibility_store_path("zbot-memory-facts.sqlite")
        .expect_err("single-file compatibility DB symlink must be rejected");

    assert_eq!(err.kind(), AdapterErrorKind::PathNotConfined);
}

#[test]
#[cfg(unix)]
fn provider_bootstrap_rejects_existing_single_file_db_symlink_escape() {
    let root = tempfile::tempdir().expect("root");
    let outside = tempfile::tempdir().expect("outside");
    std::fs::create_dir_all(root.path().join("engram")).expect("engram dir");
    std::os::unix::fs::symlink(
        outside.path().join("outside.db"),
        root.path().join("engram").join("engram_data.db"),
    )
    .expect("symlink");
    let config = AdapterConfig::engram_for_data_root(root.path(), "engram");

    let err = match EngramProvider::open(config) {
        Ok(_) => panic!("provider open must reject final single-file DB symlink"),
        Err(err) => err,
    };

    assert_eq!(err.kind(), AdapterErrorKind::PathNotConfined);
}

// STUB: AC2 - configured absolute paths are allowed only under the data root.
#[test]
fn engram_path_rejects_absolute_path_outside_data_root() {
    let root = tempfile::tempdir().expect("root");
    let outside = tempfile::tempdir().expect("outside");
    let config = AdapterConfig::engram_for_data_root(root.path(), outside.path().join("engram"));

    let err = config
        .resolve_engram_path()
        .expect_err("must reject outside path");

    assert_eq!(err.kind(), AdapterErrorKind::PathNotConfined);
}

// STUB: AC2 - absolute paths are accepted only when they stay under the root.
#[test]
fn engram_path_accepts_absolute_path_inside_data_root() {
    let root = tempfile::tempdir().expect("root");
    let absolute_storage = root.path().join("data").join("engram");
    let config = AdapterConfig::engram_for_data_root(root.path(), &absolute_storage);

    let engram_config = config.to_engram_config().expect("engram config");

    assert!(engram_config
        .storage_path
        .starts_with(root.path().canonicalize().expect("canonical root")));
}

// STUB: AC2 - user-deserialized settings cannot redefine the trusted root.
#[test]
fn serialized_config_cannot_supply_data_root() {
    let config: AdapterConfig = serde_json::from_value(serde_json::json!({
        "providerMode": "engram",
        "engramPath": "engram",
        "dataRoot": "/",
        "tenant": "agentzero",
        "wardScopeTarget": "workspace",
        "partitionScopeTarget": "workspace",
        "embeddingMode": "preserve_bytes",
        "embeddingProvider": {
            "providerType": "fastembed",
            "model": "BAAI/bge-small-en-v1.5",
            "dimensions": 384,
            "promptProfile": "query",
            "normalization": null
        },
        "migrationMode": "dry_run"
    }))
    .expect("config");

    let err = config
        .resolve_engram_path()
        .expect_err("trusted root must be injected");

    assert_eq!(err.kind(), AdapterErrorKind::MissingConfig);
}

// STUB: AC3 - Engram mode fails closed instead of falling back.
#[test]
fn engram_mode_unsupported_feature_has_no_implicit_fallback() {
    let root = tempfile::tempdir().expect("root");
    let provider = EngramProvider::open(AdapterConfig::engram_for_data_root(root.path(), "engram"))
        .expect("provider");

    let err = provider
        .require_feature(AdapterFeature::Recall)
        .expect_err("unsupported");

    assert_eq!(err.kind(), AdapterErrorKind::UnsupportedFeature);
}

#[test]
fn current_sqlite_mode_does_not_open_engram_provider() {
    let result = EngramProvider::open(AdapterConfig::default());

    let Err(err) = result else {
        panic!("current sqlite mode must not open Engram provider");
    };
    assert_eq!(err.kind(), AdapterErrorKind::UnsupportedFeature);
}

#[tokio::test]
async fn pre_facade_fixture_is_readable_through_supported_adapter_stores() {
    let root = tempfile::tempdir().expect("root");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pre-facade-engram-data.db");
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pre-facade-engram-data.manifest.json");
    assert!(fixture.is_file(), "fixture must be committed");
    assert!(manifest.is_file(), "fixture manifest must be committed");

    let storage_dir = root.path().join("engram");
    std::fs::create_dir_all(&storage_dir).expect("storage directory");
    std::fs::copy(&fixture, storage_dir.join("engram_data.db")).expect("install fixture");

    let config = AdapterConfig::engram_for_data_root(root.path(), "engram");
    let memory = EngramMemoryFactStore::open(config.clone()).expect("memory store");
    let facts = memory
        .list_memory_facts(Some("fixture-agent"), Some("pre-facade"), None, 10, 0)
        .await
        .expect("list fixture facts");
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0]["key"], "compatibility");
    assert_eq!(facts[0]["content"], "Synthetic pre-facade memory fact");
    let context = memory
        .get_ctx_fact("fixture-ward", "ctx.fixture.intent")
        .await
        .expect("get fixture context")
        .expect("fixture context");
    assert_eq!(context["content"], "Synthetic pre-facade sidecar");

    drop(memory);
    let database = root.path().join("engram").join("engram_data.db");
    let connection = rusqlite::Connection::open(&database).expect("fixture database");
    let integrity = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .expect("integrity check");
    assert_eq!(integrity, "ok");
    let database_files = std::fs::read_dir(storage_dir)
        .expect("storage files")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".db"))
        .count();
    assert_eq!(database_files, 1, "fixture must remain single-file SQLite");
}
