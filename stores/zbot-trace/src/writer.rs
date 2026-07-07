//! `TraceWriter` — per-session `.jsonl.zst` trace files.
//!
//! Each `append` writes **one complete zstd frame** (`encode_all` + `write_all`),
//! so the file is a concatenation of independently-decodable frames: every
//! appended event is durable immediately (no separate flush needed), and a
//! crash mid-write leaves all prior complete frames recoverable — the truncated
//! tail frame is skipped by a tolerant reader. Paths are confined under
//! `traces_dir` (per `docs/architecture/security.md` §Path Confinement).
//!
//! Trade-off: per-event frames compress small/token events less densely than a
//! single streamed frame would. Acceptable now (the analytics-relevant events —
//! tool_call/tool_result — are large); a periodic re-compress-on-close can
//! tighten the ratio later.

use crate::domain::TraceEvent;
use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;

const ZSTD_LEVEL: i32 = 3;

pub struct TraceWriter {
    file: std::fs::File,
}

impl TraceWriter {
    /// Open (or append to) `<traces_dir>/<session_id>.jsonl.zst`, confined.
    pub fn open_confined(traces_dir: &Path, session_id: &str) -> Result<Self> {
        validate_session_id(session_id)?;
        let root = traces_dir
            .canonicalize()
            .with_context(|| format!("traces_dir {} does not exist", traces_dir.display()))?;
        let path = root.join(format!("{session_id}.jsonl.zst"));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("open trace file {}", path.display()))?;
        Ok(Self { file })
    }

    /// Append one event as a self-contained zstd frame (JSON line).
    pub fn append(&mut self, event: &TraceEvent) -> Result<()> {
        let mut bytes = serde_json::to_vec(event)?;
        bytes.push(b'\n');
        let frame = zstd::encode_all(bytes.as_slice(), ZSTD_LEVEL)?;
        self.file.write_all(&frame)?;
        Ok(())
    }

    /// Flush the OS file buffer (each event is already a complete frame on
    /// disk; this only matters for OS-level durability ordering).
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
