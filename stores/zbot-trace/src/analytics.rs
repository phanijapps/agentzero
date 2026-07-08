//! `TraceAnalytics` — read-only DuckDB queries over `traces/*.jsonl.gz`.
//!
//! Opens an in-memory DuckDB connection per query and runs `read_json_auto` over
//! the (server-controlled, canonicalized) trace files. The user-influenced
//! filter (tool name) is bound as a `$1` parameter — never string-interpolated.
//! The file set is bounded to `MAX_FILES` per query; `ignore_errors=true` makes
//! DuckDB skip malformed/oversized lines rather than abort.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const MAX_FILES: usize = 256;

pub struct TraceAnalytics {
    dir: PathBuf,
}

impl TraceAnalytics {
    pub fn open(traces_dir: &Path) -> Result<Self> {
        Ok(Self {
            dir: traces_dir.to_path_buf(),
        })
    }

    /// Sessions where `tool` produced an error. The tool name is parameterized.
    pub fn sessions_with_failed_tool(&self, tool: &str) -> Result<Vec<String>> {
        let conn = duckdb::Connection::open_in_memory()?;
        let files = self.bounded_file_list(MAX_FILES)?;
        if files.is_empty() {
            return Ok(Vec::new());
        }
        // File paths are server-controlled (canonicalized traces_dir); only the
        // tool name (user-influenced) is bound.
        let sql = format!(
            "SELECT DISTINCT session_id FROM read_json_auto([{}], ignore_errors=true) \
             WHERE tool_name = ? AND level = 'error'",
            files.join(",")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([tool], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    /// Up to `cap` `.jsonl.gz` files under `dir`, as quoted DuckDB path
    /// literals. Bounded to keep a query from scanning an unbounded set.
    fn bounded_file_list(&self, cap: usize) -> Result<Vec<String>> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&self.dir)
            .with_context(|| format!("read traces_dir {}", self.dir.display()))?
        {
            let e = entry?;
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("gz") {
                let s = p.to_string_lossy().replace('\'', "''");
                files.push(format!("'{s}'"));
                if files.len() >= cap {
                    break;
                }
            }
        }
        Ok(files)
    }
}
