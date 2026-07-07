use rusqlite::{params, Connection};
use zbot_engram_adapter::{
    apply_migration, run_migration_dry_run, AdapterConfig, MigrationInput, MigrationMode,
    MigrationSource,
};

fn engram_config(root: &tempfile::TempDir, mode: MigrationMode) -> AdapterConfig {
    let mut config = AdapterConfig::engram_for_data_root(root.path(), "engram");
    config.migration_mode = mode;
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
