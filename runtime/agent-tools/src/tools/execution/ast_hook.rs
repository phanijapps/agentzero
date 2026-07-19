//! Post-write AST hook for extracting function primitives.
//!
//! After write_file / edit_file successfully writes a Python source
//! file in a ward, this hook invokes a bundled Python AST extractor
//! and upserts each public function as a `primitive` row in the
//! ward's memory_facts. The next subagent's ward_snapshot will show
//! the signatures, closing the loop on "use existing code, don't
//! duplicate."
//!
//! Non-blocking, best-effort: any failure (missing python3, invalid
//! syntax, store unavailable) logs a warning and returns — the
//! successful write is never undone.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::OnceLock;

use serde::Deserialize;
use zbot_stores_traits::MemoryFactStore;

/// The extractor script, embedded at compile time. On first use the
/// runtime writes it to a stable temp path so `python3` can invoke it.
const EXTRACTOR_SCRIPT: &str = include_str!("extract_primitives.py");

/// Cached path to the materialized extractor script. Written once per
/// process on first call; reused thereafter.
static EXTRACTOR_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Materialize the embedded extractor to a temp file on first call.
/// Returns `None` if the filesystem write fails (rare — /tmp full).
fn extractor_script_path() -> Option<&'static Path> {
    EXTRACTOR_PATH
        .get_or_init(|| {
            let path = std::env::temp_dir().join("zbot_extract_primitives.py");
            // Best-effort write: if it fails, we log and the hook becomes a no-op.
            match std::fs::File::create(&path)
                .and_then(|mut f| f.write_all(EXTRACTOR_SCRIPT.as_bytes()))
            {
                Ok(_) => path,
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to materialize AST extractor script"
                    );
                    PathBuf::new()
                }
            }
        })
        .as_path()
        .to_str()
        .filter(|s| !s.is_empty())
        .map(Path::new)
}

/// Shape of one entry emitted by `extract_primitives.py`.
#[derive(Debug, Deserialize)]
struct Primitive {
    name: String,
    signature: String,
    summary: String,
}

/// File extensions the hook handles. Python only in v1.
fn is_supported_language(path: &Path) -> bool {
    matches!(path.extension().and_then(|s| s.to_str()), Some("py"))
}

/// Run the extractor against a file path and return the parsed primitives.
fn extract(path: &Path, extractor_script: &Path) -> Option<Vec<Primitive>> {
    let output = Command::new("python3")
        .arg(extractor_script)
        .arg(path)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        tracing::debug!(
            path = %path.display(),
            stderr = %String::from_utf8_lossy(&output.stderr),
            "AST extractor failed (likely syntax error); skipping primitive upsert"
        );
        return None;
    }
    match serde_json::from_slice::<Vec<Primitive>>(&output.stdout) {
        Ok(items) => Some(items),
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "AST extractor emitted invalid JSON; skipping"
            );
            None
        }
    }
}

/// Build the primitive key for a given file + symbol.
///
/// Keys are stable and ward-scoped: `primitive.<ward-relative-path>.<name>`.
/// Re-extracting the same symbol upserts in place.
fn primitive_key(ward_relative_path: &str, name: &str) -> String {
    format!("primitive.{}.{}", ward_relative_path, name)
}

/// Run the hook: extract primitives and upsert them via the fact store.
/// Fire-and-forget; all failures log and return.
///
/// - `ward_id`: the ward the file belongs to (context state `ward_id`).
/// - `absolute_path`: the file that was just written.
/// - `ward_relative_path`: the same file relative to the ward root
///   (e.g. `core/valuation.py`). Used in the primitive key.
/// - `fact_store`: MemoryFactStore trait, to persist primitives.
pub async fn run(
    ward_id: &str,
    absolute_path: &Path,
    ward_relative_path: &str,
    fact_store: &Arc<dyn MemoryFactStore>,
) {
    if !is_supported_language(absolute_path) {
        return;
    }
    let Some(script_path) = extractor_script_path() else {
        return; // extractor materialization failed; already logged
    };
    let Some(primitives) = extract(absolute_path, script_path) else {
        return;
    };
    for p in primitives {
        let key = primitive_key(ward_relative_path, &p.name);
        if let Err(e) = fact_store
            .upsert_primitive(ward_id, &key, &p.signature, &p.summary)
            .await
        {
            tracing::warn!(
                ward = %ward_id,
                key = %key,
                error = %e,
                "Failed to upsert primitive; continuing"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_supported_language() {
        assert!(is_supported_language(Path::new("core/x.py")));
        assert!(!is_supported_language(Path::new("core/x.ts")));
        assert!(!is_supported_language(Path::new("core/x.rs")));
        assert!(!is_supported_language(Path::new("data/y.json")));
        assert!(!is_supported_language(Path::new("README.md")));
    }

    #[test]
    fn test_primitive_key_shape() {
        assert_eq!(
            primitive_key("core/valuation.py", "calc_wacc"),
            "primitive.core/valuation.py.calc_wacc"
        );
        assert_eq!(
            primitive_key("analysis/relative_valuation.py", "get_multiples"),
            "primitive.analysis/relative_valuation.py.get_multiples"
        );
    }
}
