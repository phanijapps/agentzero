//! Full root → child → callback → continuation flow through the shared
//! execution factory (T8 / AC7). One scripted provider serves all three
//! model turns: the root's delegate tool call, the child's respond, and the
//! parent's final continuation answer. The flow runs on the sole Rig engine
//! (construction is unconditional since the cutover).

use super::test_support::*;
use super::*;
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn delegated_child_callback_and_continuation_run_through_shared_factory() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let (bodies_tx, mut bodies_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        // Turn 1 — root asks to delegate and waits for the result.
        let (mut first, _) = listener.accept().await.unwrap();
        let body = read_request_body(&mut first).await;
        let _ = bodies_tx.send(body);
        let delegate = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-delegate\",\"function\":{\"name\":\"delegate_to_agent\",\"arguments\":\"{\\\"agent_id\\\":\\\"resume-test-agent\\\",\\\"task\\\":\\\"produce the artifact\\\",\\\"wait_for_result\\\":true}\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        write_sse(&mut first, delegate).await;

        // Turn 2 — the child responds with the artifact.
        let (mut second, _) = listener.accept().await.unwrap();
        let body = read_request_body(&mut second).await;
        let _ = bodies_tx.send(body);
        let child = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-respond-child\",\"function\":{\"name\":\"respond\",\"arguments\":\"{\\\"message\\\":\\\"child produced the artifact\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        write_sse(&mut second, child).await;

        // Turn 3 — the resumed parent answers.
        let (mut third, _) = listener.accept().await.unwrap();
        let body = read_request_body(&mut third).await;
        let _ = bodies_tx.send(body);
        let done = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"all done after delegation\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        );
        write_sse(&mut third, done).await;
    });

    let harness = build_harness(base_url).await;
    let mut events = harness.runner.ctx.event_bus.subscribe_all();
    let (_, session_id) = harness
        .runner
        .invoke_with_callback(
            ExecutionConfig::new(
                "root".to_owned(),
                "delegation-flow".to_owned(),
                harness.paths.vault_dir().clone(),
            )
            .with_mode("chat".to_owned()),
            "delegate the work".to_owned(),
            None,
        )
        .await
        .unwrap();
    let root_execution_id = harness
        .state
        .get_root_execution(&session_id)
        .unwrap()
        .expect("root execution")
        .id;

    let mut delegation_started = 0_usize;
    let mut delegation_completed = 0_usize;
    let mut child_terminal_seen = false;
    let mut root_completed = 0_usize;
    timeout(Duration::from_secs(30), async {
        loop {
            match events.recv().await.unwrap() {
                GatewayEvent::DelegationStarted { session_id: s, .. } if s == session_id => {
                    delegation_started += 1;
                }
                GatewayEvent::DelegationCompleted { session_id: s, .. } if s == session_id => {
                    delegation_completed += 1;
                }
                // Each execution owns exactly one terminal outcome (AC4):
                // the child's AgentCompleted (routed to the parent session,
                // keyed by the child execution id) must precede the root's.
                GatewayEvent::AgentCompleted {
                    session_id: s,
                    execution_id,
                    ..
                } if s == session_id => {
                    if execution_id == root_execution_id {
                        assert!(
                            child_terminal_seen,
                            "child terminal must precede the root's final completion"
                        );
                        root_completed += 1;
                        break;
                    } else {
                        assert!(!child_terminal_seen, "one child terminal");
                        child_terminal_seen = true;
                    }
                }
                _ => {}
            }
        }
    })
    .await
    .expect("root completion after continuation");

    // Drain stragglers: no second terminal for either execution.
    tokio::time::sleep(Duration::from_millis(300)).await;
    while let Ok(event) = events.try_recv() {
        if let GatewayEvent::AgentCompleted {
            session_id: s,
            execution_id,
            ..
        } = &event
        {
            if *s == session_id {
                assert_ne!(
                    execution_id, &root_execution_id,
                    "the root execution completes exactly once"
                );
            }
        }
    }
    assert_eq!(root_completed, 1, "exactly one root terminal outcome");
    assert_eq!(delegation_started, 1, "one child spawned");
    assert_eq!(delegation_completed, 1, "one child completed");

    // The child answer is durable in the child session before the parent
    // continuation ran (its request came after the callback row).
    let root_rows = harness
        .runner
        .ctx
        .messages
        .replay(&session_id, None, 100)
        .unwrap();
    assert!(
        root_rows
            .iter()
            .any(|row| row.role == "system" && row.content.contains("## From Resume Test Agent")),
        "callback row is durable in the parent session: {root_rows:?}"
    );
    let final_answers: Vec<_> = root_rows
        .iter()
        .filter(|row| row.role == "assistant" && row.content == "all done after delegation")
        .collect();
    assert_eq!(final_answers.len(), 1, "final answer durable exactly once");

    // The three model requests observed the whole flow in order: root
    // delegation prompt, child task, parent continuation with callback.
    let first_body = bodies_rx.recv().await.unwrap_or_default();
    assert!(first_body.contains("delegate the work"), "root prompt");
    let child_body = bodies_rx.recv().await.unwrap_or_default();
    assert!(
        child_body.contains("produce the artifact"),
        "child received the delegated task"
    );
    let continuation_body = bodies_rx.recv().await.unwrap_or_default();
    assert!(
        continuation_body.contains("Resume Test Agent")
            || continuation_body.contains("produced the artifact"),
        "continuation saw the callback: {continuation_body}"
    );
    assert!(continuation_body.contains("delegate the work"));
}
