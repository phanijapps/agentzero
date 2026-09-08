//! Golden turn traces — the behavior oracle for the turn-loop rewrite.
//!
//! Records the exact `StreamEvent` sequence the engine produces for the
//! three canonical flows and replays them against checked-in fixtures.
//! A Wave-4 rewrite keeps these green or it does not merge.
//!
//! Generation: `GOLDEN_RECORD=1 cargo test -p gateway-execution --features
//! test-stubs --lib golden -- --ignored` rewrites the fixtures from live
//! engine runs. Normal runs replay against the fixtures.

use super::test_support::*;
use agent_runtime::{AgentEngine, BoxedAgentEngine, ExecutorError, StreamEvent};
use std::sync::{Arc, Mutex};

/// Normalizing recorder: captures events with timestamps replaced by
/// sequence numbers so fixtures are deterministic across runs.
struct RecordingEngine {
    inner: BoxedAgentEngine,
    events: Arc<Mutex<Vec<serde_json::Value>>>,
}

#[async_trait::async_trait]
impl AgentEngine for RecordingEngine {
    async fn execute_stream(
        &self,
        user_message: &str,
        history: &[agent_runtime::ChatMessage],
        on_event: &mut agent_runtime::StreamEventSink<'_>,
    ) -> Result<(), ExecutorError> {
        let mut sink = |event: StreamEvent| {
            let mut normalized = serde_json::to_value(&event).unwrap_or_default();
            let mut guard = self.events.lock().unwrap();
            if let Some(obj) = normalized.as_object_mut() {
                obj.insert("seq".to_string(), serde_json::json!(guard.len()));
                obj.remove("timestamp");
            }
            sanitize(&mut normalized);
            guard.push(normalized);
            drop(guard);
            on_event(event);
        };
        self.inner
            .execute_stream(user_message, history, &mut sink)
            .await
    }

    async fn execute_stream_with_stop_flag(
        &self,
        user_message: &str,
        history: &[agent_runtime::ChatMessage],
        stop_flag: Option<Arc<std::sync::atomic::AtomicBool>>,
        on_event: &mut agent_runtime::StreamEventSink<'_>,
    ) -> Result<(), ExecutorError> {
        let mut sink = |event: StreamEvent| {
            let mut normalized = serde_json::to_value(&event).unwrap_or_default();
            let mut guard = self.events.lock().unwrap();
            if let Some(obj) = normalized.as_object_mut() {
                obj.insert("seq".to_string(), serde_json::json!(guard.len()));
                obj.remove("timestamp");
            }
            sanitize(&mut normalized);
            guard.push(normalized);
            drop(guard);
            on_event(event);
        };
        self.inner
            .execute_stream_with_stop_flag(user_message, history, stop_flag, &mut sink)
            .await
    }

    async fn execute(
        &self,
        user_message: &str,
        history: &[agent_runtime::ChatMessage],
    ) -> Result<String, ExecutorError> {
        self.inner.execute(user_message, history).await
    }

    fn engine_name(&self) -> &'static str {
        self.inner.engine_name()
    }
}

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn sanitize(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            // Per-run values are normalized so fixtures are portable:
            // tempdir paths, UUIDs, and short generated id suffixes.
            while let Some(idx) = text.find("/tmp/.tmp") {
                let end = text[idx..]
                    .chars()
                    .skip("/tmp/.tmp".len())
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .count()
                    + idx
                    + "/tmp/.tmp".len();
                text.replace_range(idx..end, "<vault>");
            }
            normalize_ids(text);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(sanitize),
        serde_json::Value::Object(map) => map.values_mut().for_each(sanitize),
        _ => {}
    }
}

/// Replace generated identifiers (UUIDs, `-sub-<hex>` suffixes) with
/// stable placeholders so replays compare structurally, not by run id.
fn normalize_ids(text: &mut String) {
    fn is_hex(c: char) -> bool {
        c.is_ascii_hexdigit()
    }
    // UUID shape: 8-4-4-4-12 hex groups.
    loop {
        let bytes = text.as_bytes();
        let mut found = None;
        'outer: for i in 0..bytes.len() {
            let groups = [8usize, 4, 4, 4, 12];
            let mut pos = i;
            for (gi, len) in groups.iter().enumerate() {
                if gi > 0 {
                    if pos >= bytes.len() || bytes[pos] != b'-' {
                        continue 'outer;
                    }
                    pos += 1;
                }
                if pos + len > bytes.len()
                    || !bytes[pos..pos + len]
                        .iter()
                        .all(|b| (*b as char).is_ascii_hexdigit())
                {
                    continue 'outer;
                }
                pos += len;
            }
            found = Some((i, pos));
            break;
        }
        match found {
            Some((start, end)) => text.replace_range(start..end, "<uuid>"),
            None => break,
        }
    }
    // `-sub-<6+ hex>` conversation suffixes.
    loop {
        let lower = text.to_lowercase();
        let Some(idx) = lower.find("-sub-") else {
            break;
        };
        let start = idx + "-sub-".len();
        let end = start + text[start..].chars().take_while(|c| is_hex(*c)).count();
        if end == start || end - start < 6 {
            break;
        }
        text.replace_range(idx..end, "-sub-<id>");
    }
}

fn record_or_replay(name: &str, events: &[serde_json::Value]) {
    let path = fixture_dir().join(format!("{name}.jsonl"));
    if std::env::var("GOLDEN_RECORD").is_ok() {
        std::fs::create_dir_all(fixture_dir()).unwrap();
        let body: String = events
            .iter()
            .map(|e| serde_json::to_string(e).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, body + "\n").unwrap();
        return;
    }
    let fixture = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("golden fixture {path:?} missing (record with GOLDEN_RECORD=1): {e}")
    });
    let expected: Vec<serde_json::Value> = fixture
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        events, &expected,
        "golden trace diverged for {name}: event sequence, order, or payload changed"
    );
}

/// Flow 1 — simple QA: one model turn, respond tool, completion.
#[tokio::test]
async fn golden_simple_qa() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        let done = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-r\",\"function\":{\"name\":\"respond\",\"arguments\":\"{\\\"message\\\":\\\"done\\\"}\"}}]},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n";
        write_sse(&mut socket, done).await;
    });
    let harness = build_harness(base_url).await;
    let engine = build_root_engine(&harness).await;
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorder = RecordingEngine {
        inner: engine,
        events: events.clone(),
    };
    recorder
        .execute_stream("answer now", &[], &mut |_| {})
        .await
        .unwrap();
    record_or_replay("simple-qa", &events.lock().unwrap().clone());
}

/// Build the Rig engine exactly as production root invoke does.
async fn build_root_engine(harness: &Harness) -> BoxedAgentEngine {
    use crate::invoke::ExecutorBuilder;
    let settings = gateway_services::SettingsService::new(harness.runner.ctx.paths.clone());
    let tool_settings = settings.get_tool_settings().unwrap_or_default();
    let loader = crate::invoke::AgentLoader::new(
        &harness.runner.ctx.agent_service,
        &harness.runner.ctx.provider_service,
        harness.runner.ctx.paths.clone(),
    );
    let (agent, provider) = loader.load_or_create_root("root").await.unwrap();
    let prepared =
        ExecutorBuilder::new(harness.runner.ctx.paths.vault_dir().clone(), tool_settings)
            .with_actor_kind(crate::invoke::RuntimeActorKind::Root)
            .build(
                &agent,
                &provider,
                "golden-conv",
                "golden-sess",
                &[],
                &[],
                None,
                &harness.runner.ctx.mcp_service,
                None,
            )
            .await
            .expect("prepared execution");
    crate::invoke::build_execution_engine(prepared).expect("rig engine")
}

/// Flow 2 — stop mid-stream: tokens until stop, then Err(Stopped), no Done.
#[tokio::test]
async fn golden_stop_midstream() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        // Three chunks; the stop flag fires after the first token.
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"a\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"b\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":2,\"total_tokens\":3}}\n\ndata: [DONE]\n\n";
        write_sse(&mut socket, body).await;
    });
    let harness = build_harness(base_url).await;
    let engine = build_root_engine(&harness).await;
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorder = RecordingEngine {
        inner: engine,
        events: events.clone(),
    };
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_flag = stop.clone();
    let result = recorder
        .execute_stream_with_stop_flag("hi", &[], Some(stop), &mut |event| {
            if matches!(event, StreamEvent::Token { .. }) {
                stop_flag.store(true, std::sync::atomic::Ordering::Release);
            }
        })
        .await;
    assert!(matches!(result, Err(ExecutorError::Stopped)));
    record_or_replay("stop-midstream", &events.lock().unwrap().clone());
}

/// Flow 3 — delegation yield: delegate tool call, yield without Done.
#[tokio::test]
async fn golden_delegation_yield() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        let body = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-d\",\"function\":{\"name\":\"delegate_to_agent\",\"arguments\":\"{\\\"agent_id\\\":\\\"resume-test-agent\\\",\\\"task\\\":\\\"go\\\"}\"}}]},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n";
        write_sse(&mut socket, body).await;
    });
    let harness = build_harness(base_url).await;
    let engine = build_root_engine(&harness).await;
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorder = RecordingEngine {
        inner: engine,
        events: events.clone(),
    };
    let _ = recorder
        .execute_stream("delegate work", &[], &mut |_| {})
        .await;
    record_or_replay("delegation-yield", &events.lock().unwrap().clone());
}
