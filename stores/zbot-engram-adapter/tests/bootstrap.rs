use engram_domain::types::ScopeMappingStrategy;
use engram_integration::{
    CapabilityPolicy, MigrationMode as EngramMigrationMode, SqliteStorageLayout,
};
use zbot_engram_adapter::{
    AdapterConfig, AdapterEmbeddingProviderConfig, AdapterErrorKind, AdapterFeature,
    AdapterSqliteStorageLayout, EngramProvider,
};

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
    assert!(provider.upstream_capabilities().memory_supported());
    assert!(provider.upstream_capabilities().knowledge_supported());
    assert!(provider.upstream_capabilities().beliefs_supported());
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
