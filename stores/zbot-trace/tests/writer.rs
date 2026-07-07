use std::io::Read;
use std::path::Path;
use tempfile::TempDir;
use zbot_trace::{TraceEvent, TraceWriter};

fn ev(span: &str) -> TraceEvent {
    TraceEvent {
        trace_id: "tr".to_string(),
        span_id: span.to_string(),
        session_id: "s1".to_string(),
        execution_id: "e1".to_string(),
        agent_id: "root".to_string(),
        parent_session_id: None,
        timestamp: "2026-07-07T00:00:00Z".to_string(),
        level: "info".to_string(),
        category: "tool_call".to_string(),
        message: format!("event {span}"),
        duration_ms: None,
        tool_name: Some("read_file".to_string()),
        payload: Some(serde_json::json!({ "args": span })),
        usage: None,
        model: None,
    }
}

/// Decode the concatenated zstd frames in `path`, tolerating a trailing
/// partial frame (the crash-recovery path): returns the complete JSON lines
/// decoded before any truncation.
fn read_zstd_lines(path: &Path) -> Vec<String> {
    let bytes = std::fs::read(path).unwrap();
    let mut dec = zstd::Decoder::new(&bytes[..]).unwrap();
    let mut buf = Vec::new();
    let _ = dec.read_to_end(&mut buf); // ignore trailing-partial error
    String::from_utf8_lossy(&buf)
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

#[test]
fn append_close_round_trip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("s1.jsonl.zst");
    {
        let mut w = TraceWriter::open_confined(dir.path(), "s1").unwrap();
        w.append(&ev("a")).unwrap();
        w.append(&ev("b")).unwrap();
        w.append(&ev("c")).unwrap();
        w.close().unwrap();
    }
    let lines = read_zstd_lines(&path);
    assert_eq!(lines.len(), 3, "all three events decoded");
    assert!(lines[0].contains(r#""span_id":"a""#));
    assert!(lines[2].contains(r#""span_id":"c""#));
    // Full payload survived (analytics source of truth).
    assert!(lines[0].contains(r#""args":"a""#));
}

#[test]
fn each_append_is_durable_without_close() {
    // AC: a writer dropped without close still leaves every appended event
    // decodable (each append writes a complete frame).
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("s2.jsonl.zst");
    {
        let mut w = TraceWriter::open_confined(dir.path(), "s2").unwrap();
        w.append(&ev("a")).unwrap();
        w.append(&ev("b")).unwrap();
        // dropped without close
    }
    let lines = read_zstd_lines(&path);
    assert_eq!(lines.len(), 2, "both events survive without close");
}

#[test]
fn truncated_tail_is_tolerated() {
    // AC: a crash mid-write leaves all prior complete frames recoverable.
    // Simulate by truncating the file after writing complete frames.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("s3.jsonl.zst");
    {
        let mut w = TraceWriter::open_confined(dir.path(), "s3").unwrap();
        w.append(&ev("a")).unwrap();
        w.append(&ev("b")).unwrap();
        w.append(&ev("c")).unwrap();
        w.close().unwrap();
    }
    // Truncate ~25% off the tail (cuts into the last frame).
    let full = std::fs::read(&path).unwrap();
    let cut = (full.len() * 3) / 4;
    std::fs::write(&path, &full[..cut]).unwrap();

    let lines = read_zstd_lines(&path);
    // At least the first frame survived; the partial tail did not panic.
    assert!(!lines.is_empty(), "complete frames before the cut decode");
    assert!(lines.iter().any(|l| l.contains(r#""span_id":"a""#)));
}

#[test]
fn hostile_session_ids_rejected_and_path_confined() {
    let dir = TempDir::new().unwrap();
    for bad in ["../evil", "a/b", "..", "C:x"] {
        assert!(
            TraceWriter::open_confined(dir.path(), bad).is_err(),
            "{bad:?} must be rejected for path confinement"
        );
    }
    assert!(dir.path().read_dir().unwrap().count() == 0, "no stray files from rejected ids");
    let w = TraceWriter::open_confined(dir.path(), "s4").unwrap();
    drop(w);
    assert!(dir.path().join("s4.jsonl.zst").exists(), "valid id creates the file in-dir");
}
