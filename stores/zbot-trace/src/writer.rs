//! `TraceWriter` — per-session `.jsonl.gz` trace files.
//!
//! Each `append` writes **one complete gzip member** (`GzEncoder` + `finish`),
//! so the file is a multi-member gzip stream: every appended event is durable
//! immediately, and a crash mid-write leaves all prior complete members
//! recoverable (a tolerant reader skips the truncated trailing member). DuckDB
//! reads `.jsonl.gz` natively (zstd would require the `parquet` extension — a
//! network `INSTALL`, unsuitable for a desktop app). Paths are confined under
//! `traces_dir` (per `docs/architecture/security.md` §Path Confinement).

use crate::domain::TraceEvent;
use anyhow::{Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;
use std::path::Path;

pub struct TraceWriter {
    file: std::fs::File,
}

impl TraceWriter {
    /// Open (or append to) `<traces_dir>/<session_id>.jsonl.gz`, confined.
    pub fn open_confined(traces_dir: &Path, session_id: &str) -> Result<Self> {
        validate_session_id(session_id)?;
        let root = traces_dir
            .canonicalize()
            .with_context(|| format!("traces_dir {} does not exist", traces_dir.display()))?;
        let path = root.join(format!("{session_id}.jsonl.gz"));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("open trace file {}", path.display()))?;
        Ok(Self { file })
    }

    /// Append one event as a self-contained gzip member (one JSON line).
    pub fn append(&mut self, event: &TraceEvent) -> Result<()> {
        let mut bytes = serde_json::to_vec(event)?;
        bytes.push(b'\n');
        let mut enc = GzEncoder::new(&mut self.file, Compression::default());
        enc.write_all(&bytes)?;
        enc.finish()?; // completes the gzip member, flushed to the file
        Ok(())
    }

    /// Flush the OS file buffer (each event is already a complete member).
    pub fn flush(&mut self) -> Result<()> {
        self.file.flush()?;
        Ok(())
    }

    /// Close (flush) the file.
    pub fn close(mut self) -> Result<()> {
        self.file.flush()?;
        Ok(())
    }
}

/// Reject `session_id` values that could escape `traces_dir` via path
/// traversal or platform drive prefixes.
fn validate_session_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains('\0')
        || id.contains(':')
        || id == "."
        || id == ".."
    {
        anyhow::bail!("invalid session_id: rejected for path confinement");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_hostile_session_ids() {
        for bad in ["", "..", ".", "../x", "a/b", "a\\b", "C:x", "a\0b"] {
            assert!(validate_session_id(bad).is_err(), "{bad:?} should be rejected");
        }
        assert!(validate_session_id("s1").is_ok());
        assert!(validate_session_id("550e8400-e29b-7d4a-a714-2d3a5c6b8e10").is_ok());
    }
}
