//! Real child-process probes for the pinned Rig/rmcp boundary.

use std::time::Duration;

use rig::tool::{rmcp::McpTool, ToolDyn};
use rmcp::{transport::TokioChildProcess, ServiceExt};

fn fixture_transport() -> TokioChildProcess {
    let mut command = tokio::process::Command::new("python3");
    command.arg("-u").arg(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mcp_stdio_probe.py"
    ));
    command.kill_on_drop(true);
    TokioChildProcess::new(command).expect("spawn isolated MCP fixture")
}

#[tokio::test]
async fn native_rig_mcp_calls_and_closes_owned_stdio_session() {
    let transport = fixture_transport();
    let pid = transport.id().expect("owned subprocess PID");
    let mut session = tokio::time::timeout(Duration::from_secs(5), ().serve(transport))
        .await
        .expect("bounded initialization")
        .expect("MCP initialized");
    let definitions = session.list_all_tools().await.unwrap();
    assert_eq!(definitions.len(), 1);
    let tool = McpTool::from_mcp_server(definitions[0].clone(), session.peer().clone())
        .with_timeout(Duration::from_secs(1));
    let output = tool
        .call(r#"{"value":"native-rig-echo"}"#.into())
        .await
        .unwrap();
    assert!(output.contains("native-rig-echo"));
    tokio::time::timeout(Duration::from_secs(5), session.close())
        .await
        .expect("bounded session cleanup")
        .expect("session cleanup succeeds");
    // Closing the owning session must release the process even while a tool
    // still holds a cloned peer. Linux CI additionally observes OS lifetime.
    #[cfg(target_os = "linux")]
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    #[cfg(not(target_os = "linux"))]
    let _ = pid;
    assert!(tool
        .call(r#"{"value":"after-close"}"#.into())
        .await
        .is_err());
}

#[tokio::test]
async fn closing_one_native_mcp_session_preserves_another() {
    let mut first = ().serve(fixture_transport()).await.unwrap();
    let mut second = ().serve(fixture_transport()).await.unwrap();
    first.close().await.unwrap();
    assert_eq!(second.list_all_tools().await.unwrap().len(), 1);
    second.close().await.unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn canceled_native_mcp_startup_releases_pending_child() {
    let mut command = tokio::process::Command::new("python3");
    command.args(["-c", "import time; time.sleep(60)"]);
    command.kill_on_drop(true);
    let transport = TokioChildProcess::new(command).unwrap();
    let pid = transport.id().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), ().serve(transport))
            .await
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while std::path::Path::new(&format!("/proc/{pid}")).exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("canceled initialization kills and reaps its child");
}
