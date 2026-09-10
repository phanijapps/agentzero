//! Pre-cutover transport probes using the real configured MCP clients.

use agent_runtime::mcp::{McpManager, McpServerConfig};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn discover_over_event_stream(transport: &str) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 8192];
        let read = socket.read(&mut request).await.unwrap();
        assert!(read > 0, "client sends a real request");
        let body = format!(
            "event: message\ndata: {}\n\n",
            json!({"jsonrpc":"2.0","id":1,"result":{"tools":[{
                "name":"echo","description":"fixture echo","inputSchema":{"type":"object"}
            }]}})
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
    });
    let config: McpServerConfig = serde_json::from_value(json!({
        "type":transport,"id":"fixture","name":"fixture","description":"local probe",
        "url":format!("http://{address}/mcp"),"enabled":true
    }))
    .unwrap();
    let manager = McpManager::new();
    manager.start_server(config).await.unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), manager.list_all_tools())
        .await
        .expect("discovery is bounded");
    server.await.unwrap();
    let tools = result.expect("configured transport decodes an event-stream tool list");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");
}

#[tokio::test]
async fn http_configuration_decodes_event_stream_discovery() {
    discover_over_event_stream("http").await;
}

#[tokio::test]
async fn sse_configuration_decodes_event_stream_discovery() {
    discover_over_event_stream("sse").await;
}
