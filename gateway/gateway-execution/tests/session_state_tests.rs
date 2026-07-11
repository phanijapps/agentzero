//! # Session State Builder Tests
//!
//! Integration tests for `SessionStateBuilder::build()`.
//! Each test creates a temporary database, inserts test data, and verifies
//! the assembled `SessionState`.
//!
//! **Slice 3 (T11 redo):** `extract_response`, `extract_plan`,
//! `extract_recalled_facts`, and the ward/title fallbacks now read from the
//! **messages** table and the **sessions** row (not `execution_logs.metadata`,
//! which is slimmed). Tests therefore write tool-payload data to messages via
//! the `MessageStore` and ward/title to the sessions row directly.

use std::sync::Arc;

use api_logs::LogService;
use execution_state::StateService;
use gateway_execution::session_state::{SessionPhase, SessionStateBuilder};
use gateway_services::VaultPaths;
#[allow(deprecated)]
use tempfile::tempdir;
use zbot_conversation::{Message, MessageStore, SqliteMessageStore};
use zbot_runtime_sqlite::DatabaseManager;

// ============================================================================
// HELPERS
// ============================================================================

/// Spin up a temp DB with full schema and return the builder + DB handle.
///
/// Returns `(builder, db, log_service, messages, state_service)`.
/// The MessageStore shares `conversations.db` (the same file
/// `DatabaseManager` opens) via its own r2d2 pool, mirroring the production
/// wiring in `AppState::build_conversation_stores`.
fn setup() -> (
    SessionStateBuilder,
    Arc<DatabaseManager>,
    Arc<LogService<DatabaseManager>>,
    Arc<dyn MessageStore>,
    Arc<StateService<DatabaseManager>>,
) {
    let dir = tempdir().unwrap();
    #[allow(deprecated)]
    let dir_path = dir.into_path();
    let paths = Arc::new(VaultPaths::new(dir_path));
    let db = Arc::new(DatabaseManager::new(paths.clone()).expect("DB init"));
    let log_service = Arc::new(LogService::new(db.clone()));
    let messages: Arc<dyn MessageStore> = Arc::new(SqliteMessageStore::new(
        zbot_conversation::open_conversation_pool(&paths.conversations_db())
            .expect("conversation pool"),
    ));
    let state_service = Arc::new(StateService::new(db.clone()));
    let builder =
        SessionStateBuilder::new(log_service.clone(), messages.clone(), state_service.clone());
    (builder, db, log_service, messages, state_service)
}

/// Generate a unique session-style ID.
fn uid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Insert a row into the `sessions` table with a given status.
fn insert_session_row(db: &DatabaseManager, session_id: &str, status: &str, root_agent_id: &str) {
    db.with_connection(|conn| {
        conn.execute(
            "INSERT INTO sessions (id, status, source, root_agent_id, created_at)
             VALUES (?1, ?2, 'web', ?3, datetime('now'))",
            rusqlite::params![session_id, status, root_agent_id],
        )?;
        Ok(())
    })
    .expect("insert session row");
}

/// Set `ward_id` on a session row directly — mirrors the WardChanged handler.
fn set_session_ward(db: &DatabaseManager, session_id: &str, ward_id: &str) {
    db.with_connection(|conn| {
        conn.execute(
            "UPDATE sessions SET ward_id = ?1 WHERE id = ?2",
            rusqlite::params![ward_id, session_id],
        )?;
        Ok(())
    })
    .expect("set session ward");
}

/// Set `title` on a session row directly — mirrors the SessionTitleChanged handler.
fn set_session_title(db: &DatabaseManager, session_id: &str, title: &str) {
    db.with_connection(|conn| {
        conn.execute(
            "UPDATE sessions SET title = ?1 WHERE id = ?2",
            rusqlite::params![title, session_id],
        )?;
        Ok(())
    })
    .expect("set session title");
}

/// Insert an agent_executions row so messages can reference it (FK constraint).
fn insert_execution_row(
    db: &DatabaseManager,
    execution_id: &str,
    session_id: &str,
    agent_id: &str,
) {
    db.with_connection(|conn| {
        conn.execute(
            "INSERT OR IGNORE INTO agent_executions (id, session_id, agent_id, status, started_at)
             VALUES (?1, ?2, ?3, 'running', datetime('now'))",
            rusqlite::params![execution_id, session_id, agent_id],
        )?;
        Ok(())
    })
    .expect("insert execution row");
}

/// Append a message through the MessageStore (the production write path).
/// `seq` is assigned atomically by the store.
fn append_message(
    messages: &Arc<dyn MessageStore>,
    session_id: &str,
    role: &str,
    content: &str,
    tool_calls: Option<&str>,
    tool_call_id: Option<&str>,
) {
    append_message_for_execution(
        messages,
        session_id,
        session_id,
        role,
        content,
        0,
        tool_calls,
        tool_call_id,
    );
}

fn append_message_for_execution(
    messages: &Arc<dyn MessageStore>,
    execution_id: &str,
    conversation_id: &str,
    role: &str,
    content: &str,
    token_count: i64,
    tool_calls: Option<&str>,
    tool_call_id: Option<&str>,
) {
    let msg = Message {
        id: format!("msg-{}", uid()),
        execution_id: Some(execution_id.to_string()),
        session_id: conversation_id.to_string(),
        role: role.to_string(),
        content: content.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        token_count,
        tool_calls: tool_calls.map(String::from),
        tool_call_id: tool_call_id.map(String::from),
        seq: 0, // assigned atomically by the store
    };
    messages.append(&msg).expect("append message");
}

// ============================================================================
// TESTS
// ============================================================================

#[test]
fn test_session_state_uses_conversation_ids_for_message_replay() {
    let (builder, db, log_service, messages, _state_service) = setup();
    let root_exec = format!("exec-{}", uid());
    let root_conv = format!("sess-{}", uid());
    let child_exec = format!("exec-{}", uid());
    let child_conv = format!("sess-{}", uid());
    let agent = "root";
    let child_agent = "code-agent";

    insert_session_row(&db, &root_conv, "completed", agent);
    insert_session_row(&db, &child_conv, "completed", child_agent);
    insert_execution_row(&db, &root_exec, &root_conv, agent);
    insert_execution_row(&db, &child_exec, &child_conv, child_agent);

    log_service
        .log_session_start(&root_exec, &root_conv, agent, None)
        .unwrap();
    log_service
        .log_session_end(
            &root_exec,
            &root_conv,
            agent,
            api_logs::SessionStatus::Completed,
            None,
        )
        .unwrap();
    log_service
        .log_session_start(&child_exec, &child_conv, child_agent, Some(&root_exec))
        .unwrap();
    log_service
        .log_session_end(
            &child_exec,
            &child_conv,
            child_agent,
            api_logs::SessionStatus::Completed,
            None,
        )
        .unwrap();

    append_message_for_execution(
        &messages,
        &root_exec,
        &root_conv,
        "user",
        "Build the report",
        11,
        None,
        None,
    );
    append_message_for_execution(
        &messages,
        &child_exec,
        &child_conv,
        "assistant",
        "Child finished the report",
        7,
        None,
        None,
    );

    let state = builder
        .build(&root_exec)
        .unwrap()
        .expect("session should exist");

    assert_eq!(state.user_message.as_deref(), Some("Build the report"));
    assert_eq!(state.response.as_deref(), Some("Child finished the report"));
    assert_eq!(state.session.token_count, 18);
}

#[test]
fn test_completed_session_with_response() {
    let (builder, db, log_service, messages, _state_service) = setup();
    let sid = uid();
    let conv_id = sid.clone();
    let agent = "root";

    // Session row: completed
    insert_session_row(&db, &conv_id, "completed", agent);

    // Execution logs
    log_service
        .log_session_start(&sid, &conv_id, agent, None)
        .unwrap();

    // Intent log (un-slimmed — still in execution_logs.metadata)
    log_service
        .log(
            api_logs::ExecutionLog::new(
                &sid,
                &conv_id,
                agent,
                api_logs::LogLevel::Info,
                api_logs::LogCategory::Intent,
                "Intent analyzed",
            )
            .with_metadata(serde_json::json!({ "intent": "report" })),
        )
        .unwrap();

    // Slimmed tool_call logs (only tool_id, tool_name) — derive_phase reads
    // these to detect respond/delegate/update_plan.
    log_service
        .log_tool_call(
            &sid,
            &conv_id,
            agent,
            "update_plan",
            "tc-1",
            &serde_json::json!({}),
        )
        .unwrap();
    log_service
        .log_tool_call(
            &sid,
            &conv_id,
            agent,
            "delegate",
            "tc-2",
            &serde_json::json!({}),
        )
        .unwrap();
    log_service
        .log_tool_call(
            &sid,
            &conv_id,
            agent,
            "respond",
            "tc-3",
            &serde_json::json!({}),
        )
        .unwrap();

    // Create agent_execution so messages FK is satisfied
    insert_execution_row(&db, &sid, &conv_id, agent);

    // User message
    append_message(&messages, &sid, "user", "Generate a report", None, None);

    // Assistant message with update_plan tool_calls (full args live here
    // after the slim — extract_plan reads this).
    let plan_tc = serde_json::json!([{
        "tool_id": "tc-1",
        "tool_name": "update_plan",
        "args": {
            "steps": [
                {"text": "Gather data", "status": "completed"},
                {"text": "Generate report", "status": "in_progress"}
            ]
        }
    }])
    .to_string();
    append_message(
        &messages,
        &sid,
        "assistant",
        "[tool calls]",
        Some(&plan_tc),
        None,
    );

    // Final assistant message — extract_response reads this.
    append_message(
        &messages,
        &sid,
        "assistant",
        "Here is your report",
        None,
        None,
    );

    // Build
    let state = builder.build(&sid).unwrap().expect("session should exist");

    assert_eq!(state.phase, SessionPhase::Completed);
    assert_eq!(state.response.as_deref(), Some("Here is your report"));
    assert!(state.user_message.is_some());
    assert!(!state.plan.is_empty(), "plan should come from messages");
    assert!(!state.is_live);
}

#[test]
fn test_crashed_session() {
    let (builder, db, log_service, _messages, _state_service) = setup();
    let sid = uid();
    let conv_id = sid.clone();
    let agent = "root";

    // Session row: crashed
    insert_session_row(&db, &conv_id, "crashed", agent);

    log_service
        .log_session_start(&sid, &conv_id, agent, None)
        .unwrap();

    // Tool name only — args not needed in slimmed metadata for derive_phase.
    log_service
        .log_tool_call(
            &sid,
            &conv_id,
            agent,
            "some_tool",
            "tc-1",
            &serde_json::json!({}),
        )
        .unwrap();

    let state = builder.build(&sid).unwrap().expect("session should exist");

    assert_eq!(state.phase, SessionPhase::Error);
    assert_eq!(state.session.status, "error");
    assert!(!state.is_live);
}

#[test]
fn test_session_not_found() {
    let (builder, _db, _log_service, _messages, _state_service) = setup();

    let result = builder.build("nonexistent").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_title_from_sessions_row() {
    // Slice 3: title comes from sessions.title (persisted by
    // SessionTitleChanged handler) — set it directly rather than relying on
    // set_session_title tool args (now slimmed out of execution_logs).
    let (builder, db, log_service, _messages, _state_service) = setup();
    let sid = uid();
    let conv_id = sid.clone();
    let agent = "root";

    insert_session_row(&db, &conv_id, "completed", agent);
    set_session_title(&db, &conv_id, "My Test Session");

    log_service
        .log_session_start(&sid, &conv_id, agent, None)
        .unwrap();
    log_service
        .log_session_end(
            &sid,
            &conv_id,
            agent,
            api_logs::SessionStatus::Completed,
            None,
        )
        .unwrap();

    let state = builder.build(&sid).unwrap().expect("session should exist");

    assert_eq!(state.session.title.as_deref(), Some("My Test Session"));
}

#[test]
fn test_title_falls_back_to_intent_primary_when_tool_skipped() {
    let (builder, db, log_service, _messages, _state_service) = setup();
    let sid = uid();
    let conv_id = sid.clone();
    let agent = "root";

    insert_session_row(&db, &conv_id, "completed", agent);

    log_service
        .log_session_start(&sid, &conv_id, agent, None)
        .unwrap();

    // Record an intent-analysis log but NO set_session_title tool call.
    log_service
        .log(
            api_logs::ExecutionLog::new(
                &sid,
                &conv_id,
                agent,
                api_logs::LogLevel::Info,
                api_logs::LogCategory::Intent,
                "Intent: Short research task",
            )
            .with_metadata(serde_json::json!({
                "primary_intent": "Short research task",
                "hidden_intents": [],
                "recommended_skills": [],
                "recommended_agents": ["research-agent"],
                "ward_recommendation": {
                    "action": "use_existing",
                    "ward_name": "scratch",
                    "reason": "test",
                },
                "execution_strategy": { "approach": "simple", "explanation": "" },
            })),
        )
        .unwrap();

    log_service
        .log_session_end(
            &sid,
            &conv_id,
            agent,
            api_logs::SessionStatus::Completed,
            None,
        )
        .unwrap();

    let state = builder.build(&sid).unwrap().expect("session should exist");
    assert_eq!(state.session.title.as_deref(), Some("Short research task"));
}

#[test]
fn test_title_sessions_row_wins_over_intent_fallback() {
    // Slice 3: sessions.title (set by SessionTitleChanged handler) takes
    // priority over the intent-analysis fallback.
    let (builder, db, log_service, _messages, _state_service) = setup();
    let sid = uid();
    let conv_id = sid.clone();
    let agent = "root";

    insert_session_row(&db, &conv_id, "completed", agent);
    set_session_title(&db, &conv_id, "Chosen title");

    log_service
        .log_session_start(&sid, &conv_id, agent, None)
        .unwrap();

    log_service
        .log(
            api_logs::ExecutionLog::new(
                &sid,
                &conv_id,
                agent,
                api_logs::LogLevel::Info,
                api_logs::LogCategory::Intent,
                "Intent: ignored",
            )
            .with_metadata(serde_json::json!({
                "primary_intent": "Intent primary should not be used",
                "hidden_intents": [],
                "recommended_skills": [],
                "recommended_agents": [],
                "ward_recommendation": {
                    "action": "use_existing",
                    "ward_name": "scratch",
                    "reason": "",
                },
                "execution_strategy": { "approach": "simple", "explanation": "" },
            })),
        )
        .unwrap();

    log_service
        .log_session_end(
            &sid,
            &conv_id,
            agent,
            api_logs::SessionStatus::Completed,
            None,
        )
        .unwrap();

    let state = builder.build(&sid).unwrap().expect("session should exist");
    assert_eq!(state.session.title.as_deref(), Some("Chosen title"));
}

#[test]
fn test_response_skips_tool_calls_message() {
    let (builder, db, log_service, messages, _state_service) = setup();
    let sid = uid();
    let conv_id = sid.clone();
    let agent = "root";

    insert_session_row(&db, &conv_id, "completed", agent);

    log_service
        .log_session_start(&sid, &conv_id, agent, None)
        .unwrap();
    log_service
        .log_session_end(
            &sid,
            &conv_id,
            agent,
            api_logs::SessionStatus::Completed,
            None,
        )
        .unwrap();

    // Create agent_execution so messages FK is satisfied
    insert_execution_row(&db, &sid, &conv_id, agent);

    // User message via MessageStore (the new write path).
    append_message(&messages, &sid, "user", "Hello", None, None);

    // All assistant messages are tool-call markers (no respond tool, no real
    // assistant turn) — extract_response should return None.
    append_message(&messages, &sid, "assistant", "[tool calls]", None, None);
    append_message(&messages, &sid, "assistant", "[tool calls]", None, None);

    let state = builder.build(&sid).unwrap().expect("session should exist");

    // No respond tool, no valid assistant message => response should be None
    assert!(state.response.is_none());
}

#[test]
fn test_response_from_child_session() {
    let (builder, db, log_service, messages, _state_service) = setup();
    let root_sid = uid();
    let child_sid = uid();
    let conv_id = root_sid.clone();
    let agent = "root";
    let child_agent = "code-agent";

    insert_session_row(&db, &conv_id, "completed", agent);

    // Root session logs
    log_service
        .log_session_start(&root_sid, &conv_id, agent, None)
        .unwrap();
    log_service
        .log_session_end(
            &root_sid,
            &conv_id,
            agent,
            api_logs::SessionStatus::Completed,
            None,
        )
        .unwrap();

    // Child session logs (parent = root_sid)
    log_service
        .log_session_start(&child_sid, &child_sid, child_agent, Some(&root_sid))
        .unwrap();
    // Slimmed respond tool_call — present so derive_phase recognises the
    // respond tool, but the text now lives in the child's messages.
    log_service
        .log_tool_call(
            &child_sid,
            &child_sid,
            child_agent,
            "respond",
            "tc-child-1",
            &serde_json::json!({}),
        )
        .unwrap();
    log_service
        .log_session_end(
            &child_sid,
            &child_sid,
            child_agent,
            api_logs::SessionStatus::Completed,
            None,
        )
        .unwrap();

    // Child session messages — extract_response reads this via the
    // response_from_child_messages fallback. The legacy `messages` table
    // created by DatabaseManager FKs execution_id→agent_executions and
    // session_id→sessions, so both rows must exist.
    insert_session_row(&db, &child_sid, "completed", child_agent);
    insert_execution_row(&db, &child_sid, &child_sid, child_agent);
    append_message(
        &messages,
        &child_sid,
        "assistant",
        "Child response",
        None,
        None,
    );

    let state = builder
        .build(&root_sid)
        .unwrap()
        .expect("session should exist");

    assert_eq!(state.response.as_deref(), Some("Child response"));
}

#[test]
fn test_token_count_cumulative() {
    let (builder, db, log_service, messages, _state_service) = setup();
    let root_sid = uid();
    let child_sid = uid();
    let conv_id = root_sid.clone();
    let agent = "root";
    let child_agent = "helper";

    insert_session_row(&db, &conv_id, "completed", agent);
    insert_session_row(&db, &child_sid, "completed", child_agent);

    // Root session logs
    log_service
        .log_session_start(&root_sid, &conv_id, agent, None)
        .unwrap();
    log_service
        .log_session_end(
            &root_sid,
            &conv_id,
            agent,
            api_logs::SessionStatus::Completed,
            None,
        )
        .unwrap();

    // Child session logs
    log_service
        .log_session_start(&child_sid, &child_sid, child_agent, Some(&root_sid))
        .unwrap();
    log_service
        .log_session_end(
            &child_sid,
            &child_sid,
            child_agent,
            api_logs::SessionStatus::Completed,
            None,
        )
        .unwrap();

    // Create agent_executions so messages FK is satisfied
    insert_execution_row(&db, &root_sid, &conv_id, agent);
    insert_execution_row(&db, &child_sid, &child_sid, child_agent);

    // Insert messages with explicit token counts
    append_message_for_execution(
        &messages, &root_sid, &conv_id, "user", "hello", 1000, None, None,
    );
    append_message_for_execution(
        &messages,
        &child_sid,
        &child_sid,
        "assistant",
        "world",
        5000,
        None,
        None,
    );

    let state = builder
        .build(&root_sid)
        .unwrap()
        .expect("session should exist");

    assert_eq!(state.session.token_count, 6000);
}

#[test]
fn test_plan_completed_on_finished_session() {
    let (builder, db, log_service, messages, _state_service) = setup();
    let sid = uid();
    let conv_id = sid.clone();
    let agent = "root";

    insert_session_row(&db, &conv_id, "completed", agent);

    log_service
        .log_session_start(&sid, &conv_id, agent, None)
        .unwrap();

    // Slimmed update_plan tool_call — name only (for derive_phase).
    log_service
        .log_tool_call(
            &sid,
            &conv_id,
            agent,
            "update_plan",
            "tc-1",
            &serde_json::json!({}),
        )
        .unwrap();

    log_service
        .log_session_end(
            &sid,
            &conv_id,
            agent,
            api_logs::SessionStatus::Completed,
            None,
        )
        .unwrap();

    // Create agent_execution so messages FK is satisfied
    insert_execution_row(&db, &sid, &conv_id, agent);

    // Plan content in an assistant message's tool_calls JSON (full args
    // live here after the slim).
    let plan_tc = serde_json::json!([{
        "tool_id": "tc-1",
        "tool_name": "update_plan",
        "args": {
            "steps": [
                {"text": "Step 1", "status": "in_progress"},
                {"text": "Step 2", "status": "pending"}
            ]
        }
    }])
    .to_string();
    append_message(
        &messages,
        &sid,
        "assistant",
        "[tool calls]",
        Some(&plan_tc),
        None,
    );

    let state = builder.build(&sid).unwrap().expect("session should exist");

    // All steps should be marked completed (session phase = Completed).
    assert_eq!(state.plan.len(), 2);
    for step in &state.plan {
        assert_eq!(step.status.as_deref(), Some("completed"));
    }
}

#[test]
fn test_subagent_task_from_parent_delegation() {
    let (builder, db, log_service, _messages, _state_service) = setup();
    let root_sid = uid();
    let child_sid = uid();
    let conv_id = root_sid.clone();
    let agent = "root";
    let child_agent = "code-agent";

    insert_session_row(&db, &conv_id, "completed", agent);

    // Root session
    log_service
        .log_session_start(&root_sid, &conv_id, agent, None)
        .unwrap();

    // Parent delegation log
    log_service
        .log_delegation_start(
            &root_sid,
            &conv_id,
            agent,
            child_agent,
            &child_sid,
            "Build dashboard",
        )
        .unwrap();

    log_service
        .log_session_end(
            &root_sid,
            &conv_id,
            agent,
            api_logs::SessionStatus::Completed,
            None,
        )
        .unwrap();

    // Child session logs
    log_service
        .log_session_start(&child_sid, &child_sid, child_agent, Some(&root_sid))
        .unwrap();
    log_service
        .log_session_end(
            &child_sid,
            &child_sid,
            child_agent,
            api_logs::SessionStatus::Completed,
            None,
        )
        .unwrap();

    let state = builder
        .build(&root_sid)
        .unwrap()
        .expect("session should exist");

    assert!(!state.subagents.is_empty());
    let sub = &state.subagents[0];
    assert_eq!(sub.agent_id, "code-agent");
    assert!(
        sub.task
            .as_deref()
            .unwrap_or("")
            .contains("Build dashboard"),
        "Expected task to contain 'Build dashboard', got: {:?}",
        sub.task
    );
}

#[test]
fn test_ward_from_sessions_row() {
    // Slice 3: ward comes from sessions.ward_id (persisted by the
    // WardChanged handler) — set it directly rather than relying on
    // load_ward tool args (now slimmed out of execution_logs).
    let (builder, db, log_service, _messages, _state_service) = setup();
    let sid = uid();
    let conv_id = sid.clone();
    let agent = "root";

    insert_session_row(&db, &conv_id, "completed", agent);
    set_session_ward(&db, &conv_id, "my-ward");

    log_service
        .log_session_start(&sid, &conv_id, agent, None)
        .unwrap();

    log_service
        .log_session_end(
            &sid,
            &conv_id,
            agent,
            api_logs::SessionStatus::Completed,
            None,
        )
        .unwrap();

    let state = builder.build(&sid).unwrap().expect("session should exist");

    assert!(state.ward.is_some(), "ward should be set");
    let ward = state.ward.unwrap();
    assert_eq!(ward.name, "my-ward");
}

// ============================================================================
// DELEGATION-SESSION ROBUSTNESS (Slice 3 redo — critical case)
// ============================================================================
//
// Real sessions are delegation-based: the plan arrives as a `role=system`
// message from a planner/builder agent, NOT an `update_plan` tool call. The
// prior attempt failed because its checkpoint-based extractor only understood
// `update_plan` tool_calls. This test seeds the realistic delegation shape
// and verifies that Mission Control still renders plan + ward + response.

#[test]
fn test_delegation_session_plan_from_system_message() {
    let (builder, db, log_service, messages, _state_service) = setup();
    let sid = uid();
    let conv_id = sid.clone();
    let agent = "root";

    insert_session_row(&db, &conv_id, "completed", agent);
    set_session_ward(&db, &conv_id, "maritime-tracking");
    set_session_title(&db, &conv_id, "Track the vessel");

    log_service
        .log_session_start(&sid, &conv_id, agent, None)
        .unwrap();

    // Intent log (un-slimmed)
    log_service
        .log(
            api_logs::ExecutionLog::new(
                &sid,
                &conv_id,
                agent,
                api_logs::LogLevel::Info,
                api_logs::LogCategory::Intent,
                "Intent analyzed",
            )
            .with_metadata(serde_json::json!({
                "primary_intent": "Track a vessel",
                "ward_recommendation": { "ward_name": "maritime-tracking" }
            })),
        )
        .unwrap();

    // A delegation happened (delegation log carries task — un-slimmed).
    log_service
        .log_delegation_start(
            &sid,
            &conv_id,
            agent,
            "planner-agent",
            "child-exec-1",
            "Plan vessel tracking",
        )
        .unwrap();

    log_service
        .log_session_end(
            &sid,
            &conv_id,
            agent,
            api_logs::SessionStatus::Completed,
            None,
        )
        .unwrap();

    // Messages: the realistic delegation-session shape. No update_plan
    // tool_call exists — the plan arrives via a system message.
    insert_execution_row(&db, &sid, &conv_id, agent);
    append_message(&messages, &sid, "user", "Track the Ever Given", None, None);

    // The delegation-result system message — planner response embedded.
    let delegation_system = "## From Planner Agent\n\n\
        I analyzed the request.\n\n\
        ## Steps\n\
        1. Locate the vessel on the map\n\
        2. Estimate arrival time\n\
        3. Report back\n\n\
        ---\n\
        _Conversation: `child-exec-1`_\n\n\
        [Recall] Delegation completed.";
    append_message(&messages, &sid, "system", delegation_system, None, None);

    // Final assistant response
    append_message(
        &messages,
        &sid,
        "assistant",
        "Vessel tracked; ETA Tuesday.",
        None,
        None,
    );

    let state = builder.build(&sid).unwrap().expect("session should exist");

    // Plan must be sourced from the delegation `system` message content —
    // this is the failure mode that reverted the prior attempt.
    assert!(
        !state.plan.is_empty(),
        "plan must surface from delegation system message, got: {:?}",
        state.plan
    );
    assert!(state
        .plan
        .iter()
        .any(|s| s.text.contains("Locate the vessel")));

    // ward comes from sessions.ward_id.
    assert_eq!(state.ward.as_ref().unwrap().name, "maritime-tracking");

    // response comes from the final assistant message.
    assert_eq!(
        state.response.as_deref(),
        Some("Vessel tracked; ETA Tuesday.")
    );

    // title comes from sessions.title.
    assert_eq!(state.session.title.as_deref(), Some("Track the vessel"));
}

#[test]
fn test_delegation_session_continuation_envelope_plan() {
    // The continuation prompt at core.rs:319 wraps the plan in a
    // "[DELEGATION COMPLETED. YOUR PLAN IS BELOW....]" envelope. Verify
    // extract_plan_from_messages handles this shape too.
    let (builder, db, log_service, messages, _state_service) = setup();
    let sid = uid();
    let conv_id = sid.clone();
    let agent = "root";

    insert_session_row(&db, &conv_id, "completed", agent);

    log_service
        .log_session_start(&sid, &conv_id, agent, None)
        .unwrap();
    log_service
        .log_session_end(
            &sid,
            &conv_id,
            agent,
            api_logs::SessionStatus::Completed,
            None,
        )
        .unwrap();

    insert_execution_row(&db, &sid, &conv_id, agent);
    append_message(&messages, &sid, "user", "Build a feature", None, None);

    let system_msg = "[DELEGATION COMPLETED. YOUR PLAN IS BELOW.\n\
        Review the delegate result already in context against this plan.\n\
        If the user's goal is satisfied, respond with the final answer.]\n\n\
        ## Steps\n\
        - Write the code\n\
        - Add tests\n\
        - Document";
    append_message(&messages, &sid, "system", system_msg, None, None);

    let state = builder.build(&sid).unwrap().expect("session should exist");

    assert_eq!(state.plan.len(), 3);
    assert!(state.plan.iter().any(|s| s.text.contains("Write the code")));
    assert!(state.plan.iter().any(|s| s.text.contains("Document")));
}
