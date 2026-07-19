//! # Artifact Processing
//!
//! Resolves artifact declarations from agent responses, validates file existence,
//! and persists metadata to the artifacts table.

use execution_state::{Artifact, StateService};
use std::fs::{File, Metadata};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use zbot_runtime_sqlite::DatabaseManager;

/// The agent can declare a small set of final files in one response.
pub const MAX_ARTIFACT_DECLARATIONS_PER_RESPONSE: usize = 8;
/// A persisted artifact must remain cheap and safe to preview.
pub const MAX_ARTIFACT_BYTES: u64 = 5 * 1024 * 1024;

/// An opened artifact whose path, type, and size have all been checked.
/// Consumers must read from `file`, not reopen `path` later.
pub struct ValidatedArtifactFile {
    pub file: File,
    pub path: PathBuf,
    pub metadata: Metadata,
}

fn valid_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn ward_root(vault_dir: &Path, ward_id: &str) -> io::Result<PathBuf> {
    let ward = Path::new(ward_id);
    if ward.components().count() != 1 || !valid_relative_path(ward) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid ward id",
        ));
    }
    let wards_path = vault_dir.join("wards");
    if std::fs::symlink_metadata(&wards_path)?
        .file_type()
        .is_symlink()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "wards root must not be a symlink",
        ));
    }
    let wards_root = wards_path.canonicalize()?;
    let ward_path = wards_root.join(ward);
    if std::fs::symlink_metadata(&ward_path)?
        .file_type()
        .is_symlink()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ward root must not be a symlink",
        ));
    }
    let root = ward_path.canonicalize()?;
    if !root.starts_with(&wards_root) || !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid ward root",
        ));
    }
    Ok(root)
}

/// Open every component from the configured vault through the ward file.
/// Linux's `openat` plus `O_NOFOLLOW` prevents an attacker from swapping the
/// `wards` root, an intermediate directory, or the final file with a symlink
/// between check and read. The returned descriptor is later read directly.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn open_no_follow(vault_dir: &Path, ward_id: &str, relative_path: &Path) -> io::Result<File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
    };

    unsafe extern "C" {
        fn openat(dirfd: i32, pathname: *const std::ffi::c_char, flags: i32) -> i32;
    }

    const AT_FDCWD: i32 = -100;
    const O_RDONLY: i32 = 0;
    const O_DIRECTORY: i32 = 0o200000;
    const O_NOFOLLOW: i32 = 0o400000;
    const O_CLOEXEC: i32 = 0o2000000;
    const O_NONBLOCK: i32 = 0o4000;

    fn open_at(dirfd: i32, name: &std::ffi::OsStr, flags: i32) -> io::Result<File> {
        use std::os::unix::ffi::OsStrExt;

        let name = CString::new(name.as_bytes()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "path contains a null byte")
        })?;
        // SAFETY: `name` is a NUL-terminated CString and the returned fd is
        // immediately owned by File or converted to its OS error.
        let fd = unsafe { openat(dirfd, name.as_ptr(), flags) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `openat` returned a fresh file descriptor owned here.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    let vault_name = CString::new(vault_dir.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a null byte"))?;
    // SAFETY: `vault_name` is a NUL-terminated CString and the returned fd is
    // immediately owned by File or converted to its OS error.
    let vault_fd = unsafe {
        openat(
            AT_FDCWD,
            vault_name.as_ptr(),
            O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC,
        )
    };
    if vault_fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `openat` returned a fresh file descriptor owned here.
    let mut parent = unsafe { File::from_raw_fd(vault_fd) };
    for directory in [std::ffi::OsStr::new("wards"), std::ffi::OsStr::new(ward_id)] {
        parent = open_at(
            parent.as_raw_fd(),
            directory,
            O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC,
        )?;
    }
    let mut components = relative_path.components().peekable();
    while let Some(Component::Normal(segment)) = components.next() {
        let is_final = components.peek().is_none();
        let flags =
            O_RDONLY | O_NOFOLLOW | O_CLOEXEC | if is_final { O_NONBLOCK } else { O_DIRECTORY };
        let opened = open_at(parent.as_raw_fd(), segment, flags)?;
        if is_final {
            return Ok(opened);
        }
        parent = opened;
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "artifact path must name a file",
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn open_no_follow(_vault_dir: &Path, _ward_id: &str, _relative_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "safe component-wise artifact reads are unavailable on this platform",
    ))
}

/// Open a ward-relative artifact exactly once, with no-follow semantics, and
/// validate that same handle before a caller reads it.
pub fn open_validated_artifact(
    vault_dir: &Path,
    ward_id: &str,
    relative_path: &Path,
) -> io::Result<ValidatedArtifactFile> {
    if !valid_relative_path(relative_path) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "artifact path must be ward-relative",
        ));
    }

    let root = ward_root(vault_dir, ward_id)?;
    let candidate = root.join(relative_path);
    let file = open_no_follow(vault_dir, ward_id, relative_path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "artifact is not a regular file",
        ));
    }
    if metadata.len() > MAX_ARTIFACT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "artifact exceeds preview size limit",
        ));
    }

    Ok(ValidatedArtifactFile {
        file,
        path: candidate,
        metadata,
    })
}

/// Re-open a previously persisted artifact only when its recorded path is
/// still lexically inside the active ward. The same no-follow validation used
/// at declaration time is repeated and the returned handle is the only handle
/// a caller may read from.
pub fn open_persisted_artifact(
    vault_dir: &Path,
    ward_id: &str,
    persisted_path: &Path,
) -> io::Result<ValidatedArtifactFile> {
    let root = ward_root(vault_dir, ward_id)?;
    let relative_path = persisted_path.strip_prefix(&root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "persisted artifact is outside its ward",
        )
    })?;

    open_validated_artifact(vault_dir, ward_id, relative_path)
}

/// Process artifact declarations from a respond action.
///
/// For each declaration:
/// 1. Open a ward-relative regular file with no-follow semantics
/// 2. Verify the opened handle's metadata and size
/// 3. Detect file type from the bounded relative path
/// 4. Persist only server-derived metadata to the `artifacts` table
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

    if declarations.len() > MAX_ARTIFACT_DECLARATIONS_PER_RESPONSE {
        tracing::warn!(
            declared = declarations.len(),
            limit = MAX_ARTIFACT_DECLARATIONS_PER_RESPONSE,
            "Artifact declaration limit exceeded; excess declarations are ignored"
        );
    }

    let Some(ward_id) = ward_id else {
        if !declarations.is_empty() {
            tracing::warn!("Artifact declarations require an active ward");
        }
        return persisted;
    };

    for decl in declarations
        .iter()
        .take(MAX_ARTIFACT_DECLARATIONS_PER_RESPONSE)
    {
        if decl.path.len() > 1024 || decl.label.as_ref().is_some_and(|label| label.len() > 160) {
            tracing::warn!("Artifact declaration metadata exceeds its size limit");
            continue;
        }

        let validated = match open_validated_artifact(vault_dir, ward_id, Path::new(&decl.path)) {
            Ok(validated) => validated,
            Err(error) => {
                tracing::warn!(path = %decl.path, "Artifact declaration rejected: {}", error);
                continue;
            }
        };

        let file_name = validated
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| decl.path.clone());

        let file_type = validated
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase());

        let mut artifact = Artifact::new(
            session_id,
            validated.path.to_string_lossy().to_string(),
            &file_name,
        );
        artifact.ward_id = Some(ward_id.to_string());
        artifact.execution_id = Some(execution_id.to_string());
        artifact.agent_id = Some(agent_id.to_string());
        artifact.file_type = file_type;
        artifact.file_size = Some(validated.metadata.len() as i64);
        artifact.label = decl.label.clone();
        artifact.is_goal_artifact = decl.is_goal_artifact;

        if let Err(e) = state_service.create_artifact(&artifact) {
            tracing::warn!(artifact_id = %artifact.id, "Failed to persist artifact: {}", e);
            continue;
        }

        tracing::info!(
            artifact_id = %artifact.id,
            path = %artifact.file_path,
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
    use std::io::Read;
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
    fn rejects_host_path_even_when_the_file_exists() {
        let h = setup();
        let decl = ArtifactDeclaration {
            path: "/etc/hosts".into(),
            label: None,
            is_goal_artifact: false,
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

        assert!(out.is_empty());
        assert!(h
            .state
            .list_artifacts_by_session(&h.session_id)
            .expect("list")
            .is_empty());
    }

    #[test]
    fn rejects_absolute_path_even_inside_the_vault() {
        let h = setup();
        let tmpfile = h.vault.join("absolute.md");
        std::fs::write(&tmpfile, b"hello").expect("write");

        let decl = ArtifactDeclaration {
            path: tmpfile.to_string_lossy().to_string(),
            label: Some("Analysis".into()),
            is_goal_artifact: false,
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

        assert!(out.is_empty());
        assert!(h
            .state
            .list_artifacts_by_session(&h.session_id)
            .expect("list")
            .is_empty());
    }

    #[test]
    fn rejects_parent_directory_traversal() {
        let h = setup();
        let outside = h.vault.join("outside.txt");
        std::fs::write(&outside, b"outside").expect("write outside fixture");
        std::fs::create_dir_all(h.vault.join("wards").join("library")).expect("create ward");

        let out = process_artifact_declarations(
            &[ArtifactDeclaration {
                path: "../outside.txt".into(),
                label: None,
                is_goal_artifact: false,
            }],
            &h.session_id,
            &h.execution_id,
            "agent-test",
            Some("library"),
            &h.vault,
            &h.state,
        );

        assert!(out.is_empty());
    }

    #[test]
    fn persists_ward_relative_artifact_under_wards_dir() {
        let h = setup();
        let abs = write_ward_file(&h.vault, "library", "reports/summary.md", b"# hi");

        let decl = ArtifactDeclaration {
            path: "reports/summary.md".into(),
            label: None,
            is_goal_artifact: true,
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
        assert!(art.is_goal_artifact);
        assert!(
            h.state
                .list_goal_artifacts_by_session(&h.session_id, 24)
                .expect("list goals")[0]
                .is_goal_artifact
        );
    }

    #[test]
    fn skips_oversized_and_excess_declarations() {
        let h = setup();
        let oversized = write_ward_file(&h.vault, "library", "oversized.bin", b"");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&oversized)
            .expect("open oversized fixture")
            .set_len(MAX_ARTIFACT_BYTES + 1)
            .expect("grow oversized fixture");

        let mut declarations = vec![ArtifactDeclaration {
            path: "oversized.bin".into(),
            label: None,
            is_goal_artifact: true,
        }];
        for index in 0..MAX_ARTIFACT_DECLARATIONS_PER_RESPONSE + 1 {
            let name = format!("result-{index}.txt");
            write_ward_file(&h.vault, "library", &name, b"final");
            declarations.push(ArtifactDeclaration {
                path: name,
                label: None,
                is_goal_artifact: true,
            });
        }

        let out = process_artifact_declarations(
            &declarations,
            &h.session_id,
            &h.execution_id,
            "agent-test",
            Some("library"),
            &h.vault,
            &h.state,
        );

        // One oversized declaration is rejected; only the first eight total
        // declarations are considered, so seven valid results survive.
        assert_eq!(out.len(), MAX_ARTIFACT_DECLARATIONS_PER_RESPONSE - 1);
        assert!(out.iter().all(|artifact| artifact.is_goal_artifact));
    }

    #[cfg(unix)]
    #[test]
    fn opened_handle_stays_bound_when_path_is_replaced_with_a_symlink() {
        use std::os::unix::fs::symlink;

        let h = setup();
        let path = write_ward_file(&h.vault, "library", "safe.txt", b"safe bytes");
        let mut opened = open_persisted_artifact(&h.vault, "library", &path).expect("open file");

        std::fs::remove_file(&path).expect("remove fixture");
        symlink("/etc/hosts", &path).expect("replace with symlink");

        let mut bytes = Vec::new();
        opened
            .file
            .read_to_end(&mut bytes)
            .expect("read opened handle");
        assert_eq!(bytes, b"safe bytes");
        assert!(open_persisted_artifact(&h.vault, "library", &path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_symlinked_ward_root() {
        use std::os::unix::fs::symlink;

        let h = setup();
        let outside = h.vault.join("outside-ward");
        std::fs::create_dir_all(&outside).expect("create outside directory");
        std::fs::write(outside.join("secret.txt"), b"not a ward artifact").expect("write secret");
        symlink(&outside, h.vault.join("wards").join("linked-ward")).expect("create ward symlink");

        let out = process_artifact_declarations(
            &[ArtifactDeclaration {
                path: "secret.txt".into(),
                label: None,
                is_goal_artifact: true,
            }],
            &h.session_id,
            &h.execution_id,
            "agent-test",
            Some("linked-ward"),
            &h.vault,
            &h.state,
        );

        assert!(out.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_symlinked_wards_directory() {
        use std::os::unix::fs::symlink;

        let h = setup();
        let outside = h.vault.join("outside-wards");
        std::fs::create_dir_all(outside.join("linked-ward")).expect("create outside ward");
        std::fs::write(outside.join("linked-ward").join("secret.txt"), b"outside")
            .expect("write outside file");
        std::fs::remove_dir(h.vault.join("wards")).expect("remove empty wards directory");
        symlink(&outside, h.vault.join("wards")).expect("replace wards with symlink");

        let out = process_artifact_declarations(
            &[ArtifactDeclaration {
                path: "secret.txt".into(),
                label: None,
                is_goal_artifact: true,
            }],
            &h.session_id,
            &h.execution_id,
            "agent-test",
            Some("linked-ward"),
            &h.vault,
            &h.state,
        );

        assert!(out.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_fifo_without_blocking() {
        use std::process::Command;

        let h = setup();
        let fifo = h.vault.join("wards").join("library").join("stream.pipe");
        std::fs::create_dir_all(fifo.parent().expect("fifo parent")).expect("create ward");
        let status = Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo available for unix test");
        assert!(status.success());

        let out = process_artifact_declarations(
            &[ArtifactDeclaration {
                path: "stream.pipe".into(),
                label: None,
                is_goal_artifact: true,
            }],
            &h.session_id,
            &h.execution_id,
            "agent-test",
            Some("library"),
            &h.vault,
            &h.state,
        );

        assert!(out.is_empty());
    }

    #[test]
    fn relative_path_without_ward_is_rejected() {
        let h = setup();
        std::fs::write(h.vault.join("cwd-relative.txt"), b"x").expect("write");

        let decl = ArtifactDeclaration {
            path: "cwd-relative.txt".into(),
            label: None,
            is_goal_artifact: false,
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
            is_goal_artifact: false,
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
            is_goal_artifact: false,
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
            is_goal_artifact: false,
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
                is_goal_artifact: false,
            },
            ArtifactDeclaration {
                path: "c.txt".into(), // missing
                label: Some("C".into()),
                is_goal_artifact: false,
            },
            ArtifactDeclaration {
                path: "b.txt".into(),
                label: Some("B".into()),
                is_goal_artifact: false,
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
