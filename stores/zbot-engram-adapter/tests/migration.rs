use rusqlite::{params, Connection};
use zbot_engram_adapter::{
    apply_migration, run_migration_dry_run, AdapterConfig, AllowUnclassifiedPolicy,
    GovernancePolicy, GovernanceSelection, MigrationInput, MigrationMode, MigrationSource,
    SkosExpansionPolicy, ValidationMode, ZBOT_BASE_ONTOLOGY_ID, ZBOT_GENERAL_SCHEME_ID,
};

fn engram_config(root: &tempfile::TempDir, mode: MigrationMode) -> AdapterConfig {
    let mut config = AdapterConfig::engram_for_data_root(root.path(), "engram");
    config.migration_mode = mode;
    config
}

fn governed_engram_config(root: &tempfile::TempDir, mode: MigrationMode) -> AdapterConfig {
    let mut config = engram_config(root, mode);
    config.governance = GovernancePolicy {
        default_selection: GovernanceSelection {
            ontology_ids: vec![ZBOT_BASE_ONTOLOGY_ID.to_string()],
            taxonomy_scheme_ids: vec![ZBOT_GENERAL_SCHEME_ID.to_string()],
        },
        validation_mode: ValidationMode::Advisory,
        allow_unclassified: AllowUnclassifiedPolicy::Warn,
        skos_expansion: SkosExpansionPolicy {
            max_depth: 2,
            max_fan_out: 4,
            max_candidates: 10,
        },
        ..GovernancePolicy::default()
    };
    config
}

fn source_db(root: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
    root.path().join(name)
}

fn create_empty_source(path: &std::path::Path) {
    let connection = Connection::open(path).expect("source db");
    connection
        .execute_batch(
            r#"
            CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT);
            INSERT INTO schema_version (version, applied_at) VALUES (42, '2026-07-06T00:00:00Z');
            "#,
        )
        .expect("empty source schema");
}

fn create_non_empty_knowledge_source(path: &std::path::Path) {
    let connection = Connection::open(path).expect("source db");
    connection
        .execute_batch(
            r#"
            CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT);
            INSERT INTO schema_version (version, applied_at) VALUES (42, '2026-07-06T00:00:00Z');
            CREATE TABLE memory_facts (id TEXT PRIMARY KEY, content TEXT);
            "#,
        )
        .expect("knowledge schema");
    connection
        .execute(
            "INSERT INTO memory_facts (id, content) VALUES (?1, ?2)",
            params!["fact-1", "sk-test-private-row-content"],
        )
        .expect("insert fact");
}

fn create_non_empty_conversation_source(path: &std::path::Path) {
    let connection = Connection::open(path).expect("source db");
    connection
        .execute_batch(
            r#"
            CREATE TABLE schema_version (version INTEGER PRIMARY KEY);
            INSERT INTO schema_version (version) VALUES (31);
            CREATE TABLE messages (id TEXT PRIMARY KEY, body TEXT);
            "#,
        )
        .expect("conversation schema");
    connection
        .execute(
            "INSERT INTO messages (id, body) VALUES (?1, ?2)",
            params!["msg-1", "private transcript payload"],
        )
        .expect("insert message");
}

#[test]
fn dry_run_reports_sanitized_manifest_without_creating_engram_storage() {
    let root = tempfile::tempdir().expect("root");
    let source = source_db(&root, "knowledge.db");
    create_non_empty_knowledge_source(&source);
    let input = MigrationInput::new(
        engram_config(&root, MigrationMode::DryRun),
        vec![MigrationSource::knowledge(&source)],
    )
    .with_engram_revision("mem-alpha:3af132299de6747685218220a41551670dc74a83");

    let report = run_migration_dry_run(&input).expect("dry run");

    assert!(!root.path().join("engram").exists());
    assert!(!report.would_write_engram_storage);
    assert_eq!(
        report
            .manifest
            .sanitized_counts
            .get("knowledge_0.memory_facts"),
        Some(&1)
    );
    assert!(report
        .blockers
        .iter()
        .any(|blocker| blocker.code == "row_import_not_implemented"));
}

#[test]
fn dry_run_manifest_includes_governance_choices_and_fingerprint() {
    let root = tempfile::tempdir().expect("root");
    let source = source_db(&root, "knowledge.db");
    create_empty_source(&source);
    let input = MigrationInput::new(
        governed_engram_config(&root, MigrationMode::DryRun),
        vec![MigrationSource::knowledge(&source)],
    );

    let report = run_migration_dry_run(&input).expect("dry run");

    assert_eq!(
        report.manifest.governance_ontology_ids,
        vec![ZBOT_BASE_ONTOLOGY_ID.to_string()]
    );
    assert_eq!(
        report.manifest.governance_taxonomy_scheme_ids,
        vec![ZBOT_GENERAL_SCHEME_ID.to_string()]
    );
    assert_eq!(
        report.manifest.governance_allow_unclassified,
        AllowUnclassifiedPolicy::Warn
    );
    assert_eq!(report.manifest.governance_skos_expansion.max_depth, 2);
    assert!(!report.manifest.governance_config_fingerprint.is_empty());
}

#[test]
fn apply_refuses_when_governance_policy_changes_after_dry_run() {
    let root = tempfile::tempdir().expect("root");
    let source = source_db(&root, "knowledge.db");
    create_empty_source(&source);
    let dry_input = MigrationInput::new(
        governed_engram_config(&root, MigrationMode::DryRun),
        vec![MigrationSource::knowledge(&source)],
    );
    let accepted = run_migration_dry_run(&dry_input).expect("dry run").manifest;

    let mut apply_config = governed_engram_config(&root, MigrationMode::Apply);
    apply_config
        .governance
        .default_selection
        .taxonomy_scheme_ids = vec!["zbot.changed:v1".to_string()];
    let apply_input = MigrationInput::new(apply_config, vec![MigrationSource::knowledge(&source)]);

    let err = apply_migration(&apply_input, &accepted).expect_err("governance drift");

    assert!(err
        .to_string()
        .contains("accepted dry-run manifest does not match"));
    assert!(!root.path().join("engram").exists());
}

#[test]
fn apply_refuses_without_matching_accepted_manifest() {
    let root = tempfile::tempdir().expect("root");
    let source = source_db(&root, "knowledge.db");
    create_empty_source(&source);
    let dry_input = MigrationInput::new(
        engram_config(&root, MigrationMode::DryRun),
        vec![MigrationSource::knowledge(&source)],
    );
    let mut accepted = run_migration_dry_run(&dry_input).expect("dry run").manifest;
    accepted.migration_code_version = "stale-code-version".to_string();

    let apply_input = MigrationInput::new(
        engram_config(&root, MigrationMode::Apply),
        vec![MigrationSource::knowledge(&source)],
    );
    let err = apply_migration(&apply_input, &accepted).expect_err("stale manifest");

    assert!(err
        .to_string()
        .contains("accepted dry-run manifest does not match"));
    assert!(!root.path().join("engram").exists());
}

#[test]
fn apply_refuses_when_dry_run_has_hard_blockers() {
    let root = tempfile::tempdir().expect("root");
    let source = source_db(&root, "knowledge.db");
    create_non_empty_knowledge_source(&source);
    let dry_input = MigrationInput::new(
        engram_config(&root, MigrationMode::DryRun),
        vec![MigrationSource::knowledge(&source)],
    );
    let accepted = run_migration_dry_run(&dry_input).expect("dry run").manifest;

    let apply_input = MigrationInput::new(
        engram_config(&root, MigrationMode::Apply),
        vec![MigrationSource::knowledge(&source)],
    );
    let err = apply_migration(&apply_input, &accepted).expect_err("blocked apply");

    assert!(err
        .to_string()
        .contains("dry-run has unresolved migration blockers"));
    assert!(!root.path().join("engram").exists());
}

#[test]
fn apply_writes_marker_under_confined_engram_path_after_acceptance() {
    let root = tempfile::tempdir().expect("root");
    let source = source_db(&root, "knowledge.db");
    create_empty_source(&source);
    let dry_input = MigrationInput::new(
        engram_config(&root, MigrationMode::DryRun),
        vec![MigrationSource::knowledge(&source)],
    );
    let accepted = run_migration_dry_run(&dry_input).expect("dry run").manifest;

    let apply_input = MigrationInput::new(
        engram_config(&root, MigrationMode::Apply),
        vec![MigrationSource::knowledge(&source)],
    );
    let receipt = apply_migration(&apply_input, &accepted).expect("apply");

    assert_eq!(receipt.manifest_fingerprint, accepted.fingerprint);
    assert_eq!(receipt.marker_component, "zbot-migration.sqlite");

    let marker = root.path().join("engram").join("engram_data.db");
    assert!(marker.exists());
    let connection = Connection::open(marker).expect("marker");
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM migration_apply_runs", [], |row| {
            row.get(0)
        })
        .expect("marker count");
    assert_eq!(count, 1);
}

#[test]
fn dry_run_diagnostics_redact_paths_sql_and_row_content() {
    let root = tempfile::tempdir().expect("root");
    let source = source_db(&root, "secret-api-key-source.db");
    create_non_empty_conversation_source(&source);
    let input = MigrationInput::new(
        engram_config(&root, MigrationMode::DryRun),
        vec![MigrationSource::conversation(&source)],
    );

    let report = run_migration_dry_run(&input).expect("dry run");
    let json = serde_json::to_string(&report).expect("json");

    assert!(!json.contains(root.path().to_string_lossy().as_ref()));
    assert!(!json.contains("secret-api-key-source"));
    assert!(!json.contains("private transcript"));
    assert!(!json.contains("sk-test"));
    assert!(!json.contains("SELECT "));
    assert_eq!(
        report
            .manifest
            .sanitized_counts
            .get("conversation_0.messages"),
        Some(&1)
    );
}

#[test]
fn governance_definition_fingerprints_are_path_free() {
    let root = tempfile::tempdir().expect("root");
    let source = source_db(&root, "knowledge.db");
    create_empty_source(&source);
    let config_root = root.path().join("config");
    let governance_dir = config_root.join("secret-governance");
    std::fs::create_dir_all(&governance_dir).expect("governance dir");
    std::fs::write(
        governance_dir.join("base-ontology-secret.json"),
        r#"{"ontologyId":"zbot.base:v1"}"#,
    )
    .expect("ontology file");
    std::fs::write(
        governance_dir.join("base-taxonomy-secret.json"),
        r#"{"schemeId":"zbot.general:v1"}"#,
    )
    .expect("taxonomy file");
    let mut config =
        governed_engram_config(&root, MigrationMode::DryRun).with_trusted_config_root(&config_root);
    config.governance.ontology_definition_paths = vec![std::path::PathBuf::from(
        "secret-governance/base-ontology-secret.json",
    )];
    config.governance.taxonomy_definition_paths = vec![std::path::PathBuf::from(
        "secret-governance/base-taxonomy-secret.json",
    )];
    let input = MigrationInput::new(config, vec![MigrationSource::knowledge(&source)]);

    let report = run_migration_dry_run(&input).expect("dry run");
    let json = serde_json::to_string(&report).expect("json");

    assert!(!json.contains(root.path().to_string_lossy().as_ref()));
    assert!(!json.contains("secret-governance"));
    assert!(!json.contains("base-ontology-secret"));
    assert!(!json.contains("base-taxonomy-secret"));
    assert!(!report.manifest.governance_config_fingerprint.is_empty());
}
