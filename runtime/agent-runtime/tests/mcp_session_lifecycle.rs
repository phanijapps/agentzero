//! Production MCP clients exercised against real processes and wire endpoints.
use agent_runtime::mcp::{McpManager, McpServerConfig};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
const LIMIT: Duration = Duration::from_secs(5);
const SECRET: &str = "transport-auth-canary";

#[derive(Clone, Default)]
struct TraceCapture(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for TraceCapture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TraceCapture {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(LIMIT, future)
        .await
        .expect("bounded fixture operation")
}
fn stdio_config(dir: &tempfile::TempDir, mode: &str) -> McpServerConfig {
    serde_json::from_value(json!({"type":"stdio", "id":"fixture", "name":"fixture", "description":"local lifecycle fixture", "command":"python3",
        "args":["-u",concat!(env!("CARGO_MANIFEST_DIR"),"/tests/fixtures/mcp_stdio_probe.py")],
        "env":{"PROBE_MODE":mode,"PROBE_SECRET":"stdio-auth-canary","DEBUG":"1","PROBE_PID_FILE":dir.path().join("pid"),"PROBE_CALL_FILE":dir.path().join("call")}, "enabled":true})).unwrap()
}
async fn wait_file(path: &std::path::Path) {
    bounded(async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
}
async fn assert_child_gone(dir: &tempfile::TempDir) {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string(dir.path().join("pid")).unwrap();
        bounded(async {
            for pid in text.lines() {
                while std::path::Path::new(&format!("/proc/{pid}")).exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        })
        .await;
    }
    #[cfg(not(target_os = "linux"))]
    let _ = dir;
}
#[tokio::test]
async fn stdio_persists_one_process_and_isolates_sessions() {
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let first = McpManager::new();
    let second = McpManager::new();
    bounded(first.start_server(stdio_config(&first_dir, "normal")))
        .await
        .unwrap();
    bounded(second.start_server(stdio_config(&second_dir, "normal")))
        .await
        .unwrap();
    assert_eq!(first.list_all_tools().await.unwrap().len(), 1);
    for value in ["one", "two", "normal DEBUG=1"] {
        assert!(first
            .execute_tool("fixture", "echo", json!({"value":value}))
            .await
            .unwrap()
            .to_string()
            .contains(value));
    }
    let secret_result = first
        .execute_tool("fixture", "echo", json!({"value":"stdio-auth-canary"}))
        .await
        .unwrap();
    assert!(!secret_result.to_string().contains("stdio-auth-canary"));
    assert_eq!(
        std::fs::read_to_string(first_dir.path().join("pid"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    let retained = first.get_client("fixture").await.unwrap();
    bounded(first.close()).await;
    assert_child_gone(&first_dir).await;
    assert!(retained.list_tools().await.is_err());
    assert_eq!(second.list_all_tools().await.unwrap().len(), 1);
    bounded(second.close()).await;
    assert_child_gone(&second_dir).await;
}
#[tokio::test]
async fn stdio_startup_failure_and_canceled_startup_release_children() {
    for mode in ["startup_fail", "startup_hang"] {
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(McpManager::new());
        let cloned = manager.clone();
        let config = stdio_config(&dir, mode);
        let task = tokio::spawn(async move { cloned.start_server(config).await });
        wait_file(&dir.path().join("pid")).await;
        if mode == "startup_hang" {
            bounded(manager.close()).await;
        }
        assert!(bounded(task).await.unwrap().is_err());
        assert_child_gone(&dir).await;
        assert!(manager.get_client("fixture").await.is_none());
    }
}

#[tokio::test]
async fn stdio_initialization_and_call_deadlines_release_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(McpManager::new());
    let cloned = manager.clone();
    let config = stdio_config(&dir, "startup_hang");
    let startup = tokio::spawn(async move { cloned.start_server(config).await });
    wait_file(&dir.path().join("pid")).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(6)).await;
    tokio::time::resume();
    assert!(bounded(startup).await.unwrap().is_err());
    assert_child_gone(&dir).await;

    let dir = tempfile::tempdir().unwrap();
    manager
        .start_server(stdio_config(&dir, "call_hang"))
        .await
        .unwrap();
    let cloned = manager.clone();
    let call = tokio::spawn(async move {
        cloned
            .execute_tool("fixture", "echo", json!({"value":"hang"}))
            .await
    });
    wait_file(&dir.path().join("call")).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    assert!(bounded(call).await.unwrap().is_err());
    bounded(manager.close()).await;
    assert_child_gone(&dir).await;
}
#[tokio::test]
async fn stdio_eof_pending_close_replacement_and_discovery_failure_cleanup() {
    for mode in ["eof", "call_hang"] {
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(McpManager::new());
        bounded(manager.start_server(stdio_config(&dir, mode)))
            .await
            .unwrap();
        let cloned = manager.clone();
        let call = tokio::spawn(async move {
            cloned
                .execute_tool("fixture", "echo", json!({"value":"pending"}))
                .await
        });
        if mode == "call_hang" {
            wait_file(&dir.path().join("call")).await;
            bounded(manager.close()).await;
        }
        assert!(bounded(call).await.unwrap().is_err());
        bounded(manager.close()).await;
        assert_child_gone(&dir).await;
    }
    let old = tempfile::tempdir().unwrap();
    let replacement = tempfile::tempdir().unwrap();
    let manager = McpManager::new();
    manager
        .start_server(stdio_config(&old, "normal"))
        .await
        .unwrap();
    manager
        .start_server(stdio_config(&replacement, "normal"))
        .await
        .unwrap();
    assert_child_gone(&old).await;
    bounded(manager.mark_startup_failed("fixture")).await;
    assert_child_gone(&replacement).await;
}

struct HttpFixture {
    url: String,
    calls: Arc<Mutex<Vec<String>>>,
    entered: Arc<tokio::sync::Notify>,
    disconnected: Arc<tokio::sync::Notify>,
    task: tokio::task::JoinHandle<()>,
}

#[tokio::test]
async fn canceled_close_and_manager_drop_still_release_owned_stdio() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(McpManager::new());
    manager
        .start_server(stdio_config(&dir, "call_hang"))
        .await
        .unwrap();
    let retained = manager.get_client("fixture").await.unwrap();
    let caller = retained.clone();
    let call = tokio::spawn(async move { caller.call_tool("echo", json!({"value":"hang"})).await });
    wait_file(&dir.path().join("call")).await;
    let closer = manager.clone();
    let close = tokio::spawn(async move { closer.close().await });
    bounded(async {
        while manager.get_client("fixture").await.is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    close.abort();
    let _ = close.await;
    assert!(bounded(call).await.unwrap().is_err());
    assert_child_gone(&dir).await;
    assert!(retained.list_tools().await.is_err());

    let dir = tempfile::tempdir().unwrap();
    let manager = McpManager::new();
    manager
        .start_server(stdio_config(&dir, "normal"))
        .await
        .unwrap();
    let retained = manager.get_client("fixture").await.unwrap();
    drop(manager);
    assert_child_gone(&dir).await;
    assert!(retained.list_tools().await.is_err());
}
impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl HttpFixture {
    async fn new(native: bool, sse: bool, reject: bool, hang: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let calls = Arc::new(Mutex::new(Vec::new()));
        let entered = Arc::new(tokio::sync::Notify::new());
        let disconnected = Arc::new(tokio::sync::Notify::new());
        let records = calls.clone();
        let notify = entered.clone();
        let closed = disconnected.clone();
        let task = tokio::spawn(async move {
            let mut workers = tokio::task::JoinSet::new();
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let records = records.clone();
                let notify = notify.clone();
                let closed = closed.clone();
                workers.spawn(async move {
                    let mut request = Vec::new();
                    let (headers, body) = loop {
                        let mut chunk = [0;4096];
                        let n = socket.read(&mut chunk).await.unwrap();
                        if n == 0 { return; }
                        request.extend_from_slice(&chunk[..n]);
                        if let Some(split) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                            let headers = String::from_utf8(request[..split].to_vec()).unwrap();
                            let size = headers.lines().find_map(|line| {
                                let (name,value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
                            }).unwrap_or(0);
                            if request.len() >= split+4+size { break (headers, request[split+4..split+4+size].to_vec()); }
                        }
                        assert!(request.len() < 65536);
                    };
                    assert!(headers.lines().any(|line| line.eq_ignore_ascii_case(&format!("authorization: Basic {SECRET}"))),"configured raw authorization preserved");
                    let verb = headers.split_whitespace().next().unwrap();
                    let request: Value = if body.is_empty() { Value::Null } else { serde_json::from_slice(&body).unwrap() };
                    let method = request["method"].as_str().unwrap_or(verb);
                    records.lock().unwrap().push(method.into());
                    if method == "initialize" && headers.lines().next().unwrap().contains("hang-initialize") {
                        notify.notify_one();
                        let mut byte = [0];
                        let _ = socket.read(&mut byte).await;
                        closed.notify_one();
                        return;
                    }
                    let (status, response, session) = if reject {
                        ("401 Unauthorized", json!({"error":SECRET}).to_string(), false)
                    } else if method == "GET" { ("405 Method Not Allowed", String::new(), false)
                    } else if method == "DELETE" { ("200 OK", String::new(), false)
                    } else if method == "notifications/initialized" { ("202 Accepted", String::new(), false)
                    } else {
                        let result = match method {
                            "initialize" => { assert!(native); json!({"protocolVersion":request["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}}) },
                            "tools/list" => json!({"tools":[{"name":"echo","description":format!("echo {SECRET}"),"inputSchema":{"type":"object"}}]}),
                            "tools/call" => {
                                notify.notify_one();
                                if request["params"]["arguments"]["value"] == "eof" { return; }
                                if hang { let mut byte=[0]; let _ = socket.read(&mut byte).await; closed.notify_one(); return; }
                                json!({"content":[{"type":"text","text":format!("{} {SECRET}",request["params"]["arguments"]["value"])}]})
                            },
                            other => panic!("unexpected MCP method {other}"),
                        };
                        if native && method != "initialize" {
                            assert!(records.lock().unwrap().iter().any(|method| method=="notifications/initialized"));
                            assert!(headers.to_ascii_lowercase().contains("mcp-session-id: fixture-session"));
                        }
                        let response = json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string();
                        ("200 OK", response, native)
                    };
                    let content_type = if sse && !response.is_empty() && status=="200 OK" { "text/event-stream" } else { "application/json" };
                    let response = if content_type=="text/event-stream" { format!(": {SECRET}\nevent: message\ndata: {response}\n\n") } else {response};
                    let session_header = if session {"Mcp-Session-Id: fixture-session\r\n"} else {""};
                    let wire = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n{session_header}Content-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len());
                    let _ = socket.write_all(wire.as_bytes()).await;
                });
                while let Some(result) = workers.try_join_next() {
                    result.unwrap();
                }
            }
        });
        Self {
            url,
            calls,
            entered,
            disconnected,
            task,
        }
    }
    fn config(&self, transport: &str) -> McpServerConfig {
        serde_json::from_value(json!({"type":transport,"id":"fixture","name":"fixture","description":"local lifecycle fixture","url":self.url,"headers":{"Authorization":format!("Basic {SECRET}")},"enabled":true})).unwrap()
    }
}
#[tokio::test]
async fn http_sse_and_native_streamable_handshake_call_auth_and_close() {
    for transport in ["http", "sse", "streamable-http"] {
        let native = transport == "streamable-http";
        let fixture = HttpFixture::new(native, true, false, false).await;
        let manager = McpManager::new();
        bounded(manager.start_server(fixture.config(transport)))
            .await
            .unwrap();
        let tools = bounded(manager.list_all_tools()).await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
        assert!(!tools[0].description.contains(SECRET));
        let result = bounded(manager.execute_tool("fixture", "echo", json!({"value":"hello"})))
            .await
            .unwrap();
        assert!(result.to_string().contains("hello"));
        assert!(!result.to_string().contains(SECRET));
        bounded(manager.close()).await;
        if native {
            assert!(fixture
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|method| method == "DELETE"));
        }
        assert!(
            !fixture.task.is_finished(),
            "fixture protocol assertions passed"
        );
    }
}
#[tokio::test]
async fn http_transports_redact_auth_failures_and_cancel_pending_calls() {
    for transport in ["http", "sse", "streamable-http"] {
        let native = transport == "streamable-http";
        let reject = HttpFixture::new(native, false, true, false).await;
        let manager = McpManager::new();
        let error = if native {
            bounded(manager.start_server(reject.config(transport)))
                .await
                .unwrap_err()
        } else {
            manager
                .start_server(reject.config(transport))
                .await
                .unwrap();
            bounded(manager.list_all_tools()).await.unwrap_err()
        };
        assert!(!error.to_string().contains(SECRET));
        manager.close().await;
        let fixture = HttpFixture::new(native, false, false, true).await;
        let manager = Arc::new(McpManager::new());
        bounded(manager.start_server(fixture.config(transport)))
            .await
            .unwrap();
        let cloned = manager.clone();
        let call = tokio::spawn(async move {
            cloned
                .execute_tool("fixture", "echo", json!({"value":"hang"}))
                .await
        });
        bounded(fixture.entered.notified()).await;
        bounded(manager.close()).await;
        assert!(bounded(call).await.unwrap().is_err());
        bounded(fixture.disconnected.notified()).await;
    }
}

#[tokio::test]
async fn http_transports_bound_pending_calls_without_retrying() {
    for transport in ["http", "sse", "streamable-http"] {
        let fixture = HttpFixture::new(transport == "streamable-http", false, false, true).await;
        let manager = Arc::new(McpManager::new());
        bounded(manager.start_server(fixture.config(transport)))
            .await
            .unwrap();
        let cloned = manager.clone();
        let call = tokio::spawn(async move {
            cloned
                .execute_tool("fixture", "echo", json!({"value":"hang"}))
                .await
        });
        bounded(fixture.entered.notified()).await;
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(31)).await;
        tokio::time::resume();
        assert!(bounded(call).await.unwrap().is_err());
        bounded(manager.close()).await;
        assert_eq!(
            fixture
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|method| *method == "tools/call")
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn http_transports_fail_on_eof_and_close_the_session() {
    for transport in ["http", "sse", "streamable-http"] {
        let fixture = HttpFixture::new(transport == "streamable-http", false, false, false).await;
        let manager = McpManager::new();
        bounded(manager.start_server(fixture.config(transport)))
            .await
            .unwrap();
        assert!(
            bounded(manager.execute_tool("fixture", "echo", json!({"value":"eof"})))
                .await
                .is_err()
        );
        bounded(manager.close()).await;
        assert!(manager.get_client("fixture").await.is_none());
    }
}

#[tokio::test]
async fn native_http_pending_initialization_cancels_and_times_out() {
    for cancel in [true, false] {
        let mut fixture = HttpFixture::new(true, false, false, false).await;
        fixture.url.push_str("?hang-initialize");
        let manager = Arc::new(McpManager::new());
        let cloned = manager.clone();
        let config = fixture.config("streamable-http");
        let startup = tokio::spawn(async move { cloned.start_server(config).await });
        bounded(fixture.entered.notified()).await;
        if cancel {
            bounded(manager.close()).await;
        } else {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(6)).await;
            tokio::time::resume();
        }
        assert!(bounded(startup).await.unwrap().is_err());
        assert!(manager.get_client("fixture").await.is_none());
        bounded(fixture.disconnected.notified()).await;
    }
}

#[tokio::test]
async fn native_mcp_trace_diagnostics_do_not_expose_payload_canaries() {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
    let capture = TraceCapture::default();
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            "trace,rmcp::service=trace",
        ))
        .with(tracing_subscriber::filter::filter_fn(
            agent_runtime::logging::safe_runtime_diagnostics,
        ))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(capture.clone()),
        )
        .try_init()
        .unwrap();
    tracing::trace!("trace canary fixture entered");
    tracing::warn!(target: "agent_runtime::mcp", "redacted operational warning fixture");
    // Pin the parser target even when its optional tracing feature is disabled.
    tracing::warn!(target: "sse_stream::stream", line = "sse-diagnostic-canary", "invalid SSE line");

    let directory = tempfile::tempdir().unwrap();
    let stdio = McpManager::new();
    stdio
        .start_server(stdio_config(&directory, "normal"))
        .await
        .unwrap();
    stdio.list_all_tools().await.unwrap();
    stdio
        .execute_tool("fixture", "echo", json!({"value":"stdio-auth-canary"}))
        .await
        .unwrap();
    bounded(stdio.close()).await;
    let fixture = HttpFixture::new(true, true, false, false).await;
    let http = McpManager::new();
    http.start_server(fixture.config("streamable-http"))
        .await
        .unwrap();
    http.list_all_tools().await.unwrap();
    http.execute_tool("fixture", "echo", json!({"value":"hello"}))
        .await
        .unwrap();
    bounded(http.close()).await;

    let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(
        logs.contains("trace canary fixture entered"),
        "positive logging control"
    );
    assert!(logs.contains("redacted operational warning fixture"));
    assert!(!logs.contains("sse-diagnostic-canary"));
    assert!(
        !logs.contains("stdio-auth-canary"),
        "stdio payload reached diagnostics"
    );
    assert!(!logs.contains(SECRET), "HTTP payload reached diagnostics");
}
