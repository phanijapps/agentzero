//! # Artifact Processing
//!
//! Resolves artifact declarations from agent responses, validates file existence,
//! and persists metadata to the artifacts table.

use execution_state::{Artifact, StateService};
use std::path::Path;
use std::sync::Arc;
use zbot_stores_sqlite::DatabaseManager;

/// Process artifact declarations from a respond action.
///
/// For each declaration:
/// 1. Resolve absolute path (relative paths resolved against ward directory)
/// 2. Verify the file exists and read metadata (size)
/// 3. Detect file type from extension
/// 4. Persist to the `artifacts` table
///
/// Returns the list of successfully persisted artifacts.
pub fn process_artifact_declarations(
    declarations: &[agent_primitives::event::ArtifactDeclaration],
    session_id: &str,
    execution_id: &str,
    agent_id: &str,
    ward_id: Option<&str>,
    vault_dir: &Path,
    state_service: &Arc<StateService<DatabaseManager>>,
) -> Vec<Artifact> {
    let mut persisted = Vec::new();

    for decl in declarations {
        // Resolve absolute path
        let abs_path = if std::path::Path::new(&decl.path).is_absolute() {
            std::path::PathBuf::from(&decl.path)
        } else if let Some(ward) = ward_id {
            vault_dir.join("wards").join(ward).join(&decl.path)
        } else {
            std::path::PathBuf::from(&decl.path)
        };

        // Check file exists
        let meta = match std::fs::metadata(&abs_path) {
            Ok(m) => m,
            Err(_) => {
                tracing::warn!(path = %abs_path.display(), "Artifact declared but file not found");
                continue;
            }
        };

        let file_name = abs_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| decl.path.clone());

        let file_type = abs_path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase());

        let mut artifact = Artifact::new(
            session_id,
            abs_path.to_string_lossy().to_string(),
            &file_name,
        );
        artifact.ward_id = ward_id.map(|s| s.to_string());
        artifact.execution_id = Some(execution_id.to_string());
        artifact.agent_id = Some(agent_id.to_string());
        artifact.file_type = file_type;
        artifact.file_size = Some(meta.len() as i64);
        artifact.label = decl.label.clone();

        if let Err(e) = state_service.create_artifact(&artifact) {
            tracing::warn!(artifact_id = %artifact.id, "Failed to persist artifact: {}", e);
            continue;
        }

        tracing::info!(
            artifact_id = %artifact.id,
            path = %abs_path.display(),
            "Artifact persisted"
        );
        persisted.push(artifact);
    }

    persisted
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_primitives::event::ArtifactDeclaration;
    use gateway_services::VaultPaths;
    use tempfile::TempDir;

    /// Fully-wired harness: temp vault, real DatabaseManager, StateService,
    /// and a seeded parent session so the artifacts FK is satisfied.
    struct Harness {
        _tmp: TempDir,
        vault: std::path::PathBuf,
        state: Arc<StateService<DatabaseManager>>,
        session_id: String,
        execution_id: String,
    }

    fn setup() -> Harness {
        let tmp = TempDir::new().expect("tempdir");
        let vault = tmp.path().to_path_buf();
        let paths = Arc::new(VaultPaths::new(vault.clone()));
        paths.ensure_dirs_exist().expect("ensure vault dirs");
        let db = Arc::new(DatabaseManager::new(paths).expect("db init"));
        let state = Arc::new(StateService::new(db));
        let (session, execution) = state.create_session("agent-test").expect("seed session");
        Harness {
            _tmp: tmp,
            vault,
            state,
            session_id: session.id,
            execution_id: execution.id,
        }
    }

    /// Seed a real file under `<vault>/wards/<ward>/<rel>` and return the
    /// absolute path we wrote to, so tests can both declare a relative path
    /// and assert against the resolved absolute one.
    fn write_ward_file(vault: &Path, ward: &str, rel: &str, body: &[u8]) -> std::path::PathBuf {
        let abs = vault.join("wards").join(ward).join(rel);
        std::fs::create_dir_all(abs.parent().expect("parent")).expect("mkdir");
        std::fs::write(&abs, body).expect("write");
        abs
    }

    #[test]
    fn persists_absolute_path_artifact() {
        let h = setup();
        let tmpfile = h.vault.join("absolute.md");
        std::fs::write(&tmpfile, b"hello").expect("write");

        let decl = ArtifactDeclaration {
            path: tmpfile.to_string_lossy().to_string(),
            label: Some("Analysis".into()),
        };
        let out = process_artifact_declarations(
            std::slice::from_ref(&decl),
            &h.session_id,
            &h.execution_id,
            "agent-test",
            None,
            &h.vault,
            &h.state,
        );

        assert_eq!(out.len(), 1);
        let art = &out[0];
        assert_eq!(art.session_id, h.session_id);
        assert_eq!(art.execution_id.as_deref(), Some(h.execution_id.as_str()));
        assert_eq!(art.agent_id.as_deref(), Some("agent-test"));
        assert_eq!(art.ward_id, None);
        assert_eq!(art.file_path, tmpfile.to_string_lossy());
        assert_eq!(art.file_name, "absolute.md");
        assert_eq!(art.file_type.as_deref(), Some("md"));
        assert_eq!(art.file_size, Some(5));
        assert_eq!(art.label.as_deref(), Some("Analysis"));

        // Persisted to the DB too — list by session and it should come back.
        let listed = h
            .state
            .list_artifacts_by_session(&h.session_id)
            .expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, art.id);
    }

    #[test]
    fn persists_ward_relative_artifact_under_wards_dir() {
        let h = setup();
        let abs = write_ward_file(&h.vault, "library", "reports/summary.md", b"# hi");

        let decl = ArtifactDeclaration {
            path: "reports/summary.md".into(),
            label: None,
        };
        let out = process_artifact_declarations(
            std::slice::from_ref(&decl),
            &h.session_id,
            &h.execution_id,
            "agent-test",
            Some("library"),
            &h.vault,
            &h.state,
        );

        assert_eq!(out.len(), 1);
        let art = &out[0];
        assert_eq!(art.ward_id.as_deref(), Some("library"));
        // Resolved path must match what we wrote, not the raw declared path.
        assert_eq!(art.file_path, abs.to_string_lossy());
        assert_eq!(art.file_name, "summary.md");
        assert_eq!(art.file_type.as_deref(), Some("md"));
    }

    #[test]
    fn relative_path_without_ward_resolves_against_cwd() {
        // No ward_id + non-absolute path → PathBuf::from(&decl.path) → relative
        // to the process CWD. We simulate that by chdir'ing to the temp vault.
        let h = setup();
        std::fs::write(h.vault.join("cwd-relative.txt"), b"x").expect("write");

        let prev_cwd = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(&h.vault).expect("chdir");

        let decl = ArtifactDeclaration {
            path: "cwd-relative.txt".into(),
            label: None,
        };
        let out = process_artifact_declarations(
            std::slice::from_ref(&decl),
            &h.session_id,
            &h.execution_id,
            "agent-test",
            None,
            &h.vault,
            &h.state,
        );

        std::env::set_current_dir(prev_cwd).expect("restore cwd");

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].file_name, "cwd-relative.txt");
        assert!(out[0].ward_id.is_none());
    }

    #[test]
    fn missing_file_is_skipped_not_errored() {
        let h = setup();
        let decl = ArtifactDeclaration {
            path: h
                .vault
                .join("does-not-exist.md")
                .to_string_lossy()
                .to_string(),
            label: None,
        };
        let out = process_artifact_declarations(
            std::slice::from_ref(&decl),
            &h.session_id,
            &h.execution_id,
            "agent-test",
            None,
            &h.vault,
            &h.state,
        );
        assert!(out.is_empty());
        // And the DB must have no rows for this session.
        assert!(h
            .state
            .list_artifacts_by_session(&h.session_id)
            .expect("list")
            .is_empty());
    }

    #[test]
    fn extensionless_file_has_no_file_type() {
        let h = setup();
        let abs = write_ward_file(&h.vault, "x", "Makefile", b"all:");

        let decl = ArtifactDeclaration {
            path: "Makefile".into(),
            label: None,
        };
        let out = process_artifact_declarations(
            std::slice::from_ref(&decl),
            &h.session_id,
            &h.execution_id,
            "agent-test",
            Some("x"),
            &h.vault,
            &h.state,
        );

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].file_name, "Makefile");
        assert!(out[0].file_type.is_none(), "no extension → file_type None");
        assert_eq!(out[0].file_path, abs.to_string_lossy());
    }

    #[test]
    fn extension_is_lowercased() {
        let h = setup();
        write_ward_file(&h.vault, "x", "Data.JSON", b"{}");

        let decl = ArtifactDeclaration {
            path: "Data.JSON".into(),
            label: None,
        };
        let out = process_artifact_declarations(
            std::slice::from_ref(&decl),
            &h.session_id,
            &h.execution_id,
            "agent-test",
            Some("x"),
            &h.vault,
            &h.state,
        );
        assert_eq!(out[0].file_type.as_deref(), Some("json"));
    }

    #[test]
    fn multiple_declarations_partition_into_hits_and_misses() {
        let h = setup();
        write_ward_file(&h.vault, "w", "a.txt", b"aa");
        write_ward_file(&h.vault, "w", "b.txt", b"bbbb");
        // no c.txt on disk

        let decls = vec![
            ArtifactDeclaration {
                path: "a.txt".into(),
                label: Some("A".into()),
            },
            ArtifactDeclaration {
                path: "c.txt".into(), // missing
                label: Some("C".into()),
            },
            ArtifactDeclaration {
                path: "b.txt".into(),
                label: Some("B".into()),
            },
        ];
        let out = process_artifact_declarations(
            &decls,
            &h.session_id,
            &h.execution_id,
            "agent-test",
            Some("w"),
            &h.vault,
            &h.state,
        );

        // Two hits, in declaration order.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].label.as_deref(), Some("A"));
        assert_eq!(out[1].label.as_deref(), Some("B"));
        // File sizes match the on-disk bytes we wrote.
        assert_eq!(out[0].file_size, Some(2));
        assert_eq!(out[1].file_size, Some(4));
    }

    #[test]
    fn empty_declaration_list_is_noop() {
        let h = setup();
        let out = process_artifact_declarations(
            &[],
            &h.session_id,
            &h.execution_id,
            "agent-test",
            None,
            &h.vault,
            &h.state,
        );
        assert!(out.is_empty());
    }
}
