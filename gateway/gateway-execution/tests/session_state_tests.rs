//! # Session State Builder Tests
//!
//! Integration tests for `SessionStateBuilder::build()`.
//! Each test creates a temporary database, inserts test data, and verifies
//! the assembled `SessionState`.

use std::sync::Arc;

use api_logs::LogService;
use gateway_execution::session_state::{SessionPhase, SessionStateBuilder};
use gateway_services::VaultPaths;
#[allow(deprecated)]
use tempfile::tempdir;
use zbot_conversation::{CheckpointStore, SqliteCheckpointStore};
use zbot_stores_sqlite::{ConversationRepository, DatabaseManager};

// ============================================================================
// HELPERS
// ============================================================================

/// Spin up a temp DB with full schema and return the builder + DB handle.
///
/// Returns the builder plus the CheckpointStore so tests can seed turn-
/// boundary snapshots when verifying the checkpoint-read path (Slice 3).
fn setup() -> (
    SessionStateBuilder,
    Arc<DatabaseManager>,
    Arc<LogService<DatabaseManager>>,
    Arc<ConversationRepository>,
    Arc<SqliteCheckpointStore>,
) {
    let dir = tempdir().unwrap();
    #[allow(deprecated)]
    let dir_path = dir.into_path();
    let paths = Arc::new(VaultPaths::new(dir_path));
    let db = Arc::new(DatabaseManager::new(paths.clone()).expect("DB init"));
    let log_service = Arc::new(LogService::new(db.clone()));
    let conversations = Arc::new(ConversationRepository::new(db.clone()));

    // CheckpointStore shares the same conversations.db (messages + checkpoints
    // tables are in the zbot-conversation schema).
    let pool = zbot_conversation::open_conversation_pool(&paths.conversations_db())
        .expect("conversation pool init");
    let checkpoints = Arc::new(SqliteCheckpointStore::new(pool));

    let builder = SessionStateBuilder::new(
        log_service.clone(),
        conversations.clone(),
        checkpoints.clone() as Arc<dyn zbot_conversation::CheckpointStore>,
    );
    (builder, db, log_service, conversations, checkpoints)
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

/// Insert a message with an explicit token_count via raw SQL.
fn insert_message_raw(
    db: &DatabaseManager,
    execution_id: &str,
    role: &str,
    content: &str,
    token_count: i32,
) {
    let id = format!("msg-{}", uid());
    db.with_connection(|conn| {
        conn.execute(
            "INSERT INTO messages (id, execution_id, role, content, created_at, token_count)
             VALUES (?1, ?2, ?3, ?4, datetime('now'), ?5)",
            rusqlite::params![id, execution_id, role, content, token_count],
        )?;
        Ok(())
    })
    .expect("insert message");
}

// ============================================================================
// TESTS
// ============================================================================

#[test]
fn test_completed_session_with_response() {
    let (builder, db, log_service, conversations, _checkpoints) = setup();
    let sid = uid();
    let conv_id = sid.clone();
    let agent = "root";

    // Session row: completed
    insert_session_row(&db, &conv_id, "completed", agent);

    // Execution logs
    log_service
        .log_session_start(&sid, &conv_id, agent, None)
        .unwrap();

    // Intent log
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

    // update_plan tool call
    log_service
        .log_tool_call(
            &sid,
            &conv_id,
            agent,
            "update_plan",
            "tc-1",
            &serde_json::json!({
                "steps": [
                    {"text": "Gather data", "status": "completed"},
                    {"text": "Generate report", "status": "in_progress"}
                ]
            }),
        )
        .unwrap();

    // delegate tool call (external tool)
    log_service
        .log_tool_call(
            &sid,
            &conv_id,
            agent,
            "delegate",
            "tc-2",
            &serde_json::json!({"agent": "code-agent"}),
        )
        .unwrap();

    // respond tool call
    log_service
        .log_tool_call(
            &sid,
            &conv_id,
            agent,
            "respond",
            "tc-3",
            &serde_json::json!({"message": "Here is your report"}),
        )
        .unwrap();

    // Create agent_execution so messages FK is satisfied
    insert_execution_row(&db, &sid, &conv_id, agent);

    // User message in conversation
    conversations
        .add_message(&sid, "user", "Generate a report", None, None)
        .unwrap();

    // Build
    let state = builder.build(&sid).unwrap().expect("session should exist");

    assert_eq!(state.phase, SessionPhase::Completed);
    assert_eq!(state.response.as_deref(), Some("Here is your report"));
    assert!(state.user_message.is_some());
    assert!(!state.plan.is_empty());
    assert!(!state.is_live);
}

#[test]
fn test_crashed_session() {
    let (builder, db, log_service, _conversations, _checkpoints) = setup();
    let sid = uid();
    let conv_id = sid.clone();
    let agent = "root";

    // Session row: crashed
    insert_session_row(&db, &conv_id, "crashed", agent);

    log_service
        .log_session_start(&sid, &conv_id, agent, None)
        .unwrap();

    log_service
        .log_tool_call(
            &sid,
            &conv_id,
            agent,
            "some_tool",
            "tc-1",
            &serde_json::json!({"arg": "value"}),
        )
        .unwrap();

    let state = builder.build(&sid).unwrap().expect("session should exist");

    assert_eq!(state.phase, SessionPhase::Error);
    assert_eq!(state.session.status, "error");
    assert!(!state.is_live);
}

#[test]
fn test_session_not_found() {
    let (builder, _db, _log_service, _conversations, _checkpoints) = setup();

    let result = builder.build("nonexistent").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_title_from_tool_call() {
    let (builder, db, log_service, _conversations, _checkpoints) = setup();
    let sid = uid();
    let conv_id = sid.clone();
    let agent = "root";

    insert_session_row(&db, &conv_id, "completed", agent);

    log_service
        .log_session_start(&sid, &conv_id, agent, None)
        .unwrap();

    log_service
        .log_tool_call(
            &sid,
            &conv_id,
            agent,
            "set_session_title",
            "tc-1",
            &serde_json::json!({"title": "My Test Session"}),
        )
        .unwrap();

    // End session so it has an ended_at
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
    let (builder, db, log_service, _conversations, _checkpoints) = setup();
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
fn test_title_tool_call_wins_over_intent_fallback() {
    let (builder, db, log_service, _conversations, _checkpoints) = setup();
    let sid = uid();
    let conv_id = sid.clone();
    let agent = "root";

    insert_session_row(&db, &conv_id, "completed", agent);

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
        .log_tool_call(
            &sid,
            &conv_id,
            agent,
            "set_session_title",
            "tc-1",
            &serde_json::json!({"title": "Chosen title"}),
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
    let (builder, db, log_service, conversations, _checkpoints) = setup();
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

    // User message
    conversations
        .add_message(&sid, "user", "Hello", None, None)
        .unwrap();

    // All assistant messages are tool-call markers
    conversations
        .add_message(&sid, "assistant", "[tool calls]", None, None)
        .unwrap();
    conversations
        .add_message(&sid, "assistant", "[tool calls]", None, None)
        .unwrap();

    let state = builder.build(&sid).unwrap().expect("session should exist");

    // No respond tool, no valid assistant message => response should be None
    assert!(state.response.is_none());
}

#[test]
fn test_response_from_child_session() {
    let (builder, db, log_service, _conversations, _checkpoints) = setup();
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
    log_service
        .log_tool_call(
            &child_sid,
            &child_sid,
            child_agent,
            "respond",
            "tc-child-1",
            &serde_json::json!({"message": "Child response"}),
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

    let state = builder
        .build(&root_sid)
        .unwrap()
        .expect("session should exist");

    assert_eq!(state.response.as_deref(), Some("Child response"));
}

#[test]
fn test_token_count_cumulative() {
    let (builder, db, log_service, _conversations, _checkpoints) = setup();
    let root_sid = uid();
    let child_sid = uid();
    let conv_id = root_sid.clone();
    let agent = "root";
    let child_agent = "helper";

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
    insert_execution_row(&db, &child_sid, &conv_id, child_agent);

    // Insert messages with explicit token counts
    insert_message_raw(&db, &root_sid, "user", "hello", 1000);
    insert_message_raw(&db, &child_sid, "assistant", "world", 5000);

    let state = builder
        .build(&root_sid)
        .unwrap()
        .expect("session should exist");

    assert_eq!(state.session.token_count, 6000);
}

#[test]
fn test_plan_completed_on_finished_session() {
    let (builder, db, log_service, _conversations, _checkpoints) = setup();
    let sid = uid();
    let conv_id = sid.clone();
    let agent = "root";

    insert_session_row(&db, &conv_id, "completed", agent);

    log_service
        .log_session_start(&sid, &conv_id, agent, None)
        .unwrap();

    // Plan with in_progress / pending steps
    log_service
        .log_tool_call(
            &sid,
            &conv_id,
            agent,
            "update_plan",
            "tc-1",
            &serde_json::json!({
                "steps": [
                    {"text": "Step 1", "status": "in_progress"},
                    {"text": "Step 2", "status": "pending"}
                ]
            }),
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

    // All steps should be marked completed
    assert_eq!(state.plan.len(), 2);
    for step in &state.plan {
        assert_eq!(step.status.as_deref(), Some("completed"));
    }
}

#[test]
fn test_subagent_task_from_parent_delegation() {
    let (builder, db, log_service, _conversations, _checkpoints) = setup();
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
fn test_ward_from_tool_call() {
    let (builder, db, log_service, _conversations, _checkpoints) = setup();
    let sid = uid();
    let conv_id = sid.clone();
    let agent = "root";

    insert_session_row(&db, &conv_id, "completed", agent);

    log_service
        .log_session_start(&sid, &conv_id, agent, None)
        .unwrap();

    // Ward tool call — tool name contains "ward"
    log_service
        .log_tool_call(
            &sid,
            &conv_id,
            agent,
            "load_ward",
            "tc-1",
            &serde_json::json!({"action": "use", "name": "my-ward"}),
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

    assert!(state.ward.is_some(), "ward should be set");
    let ward = state.ward.unwrap();
    assert_eq!(ward.name, "my-ward");
}

// ============================================================================
// SLICE 3: checkpoint-read path tests
// ============================================================================

/// Verify that `build()` reads tool-payload-derived fields (plan,
/// recalled_facts, response) from the turn-boundary `context_state`
/// checkpoint — NOT from `execution_logs.metadata.args/result`.
///
/// This test seeds **messages** (with full tool_calls JSON) and a
/// **checkpoint** (built via `build_context_state`), but deliberately
/// inserts **no** tool args in `execution_logs` (simulating slimmed metadata).
/// If `build()` tried to read plan/response from logs, it would return empty.
#[test]
fn test_checkpoint_path_populates_plan_and_response() {
    let (builder, db, log_service, conversations, checkpoints) = setup();
    let sid = uid();
    let conv_id = sid.clone();
    let agent = "root";

    insert_session_row(&db, &conv_id, "completed", agent);
    insert_execution_row(&db, &sid, &conv_id, agent);

    // Session log start (no tool-call logs with args — simulating slimmed metadata)
    log_service
        .log_session_start(&sid, &conv_id, agent, None)
        .unwrap();

    // Messages with full tool_calls JSON (this is where payloads live post-slimming)
    conversations
        .add_message(&sid, "user", "Build a report", None, None)
        .unwrap();

    // Assistant message with update_plan + respond tool calls
    let tool_calls_json = serde_json::json!([{
        "tool_id": "tc-plan",
        "tool_name": "update_plan",
        "args": {
            "steps": [
                {"text": "Step A", "status": "completed"},
                {"text": "Step B", "status": "in_progress"}
            ]
        }
    }, {
        "tool_id": "tc-respond",
        "tool_name": "respond",
        "args": {"message": "Final answer from checkpoint"}
    }])
    .to_string();
    db.with_connection(|conn| {
        conn.execute(
            "INSERT INTO messages (id, execution_id, session_id, role, content, created_at, token_count, tool_calls)
             VALUES (?1, ?2, ?3, 'assistant', '[tool calls]', datetime('now'), 0, ?4)",
            rusqlite::params![format!("msg-{}", uid()), &sid, &sid, tool_calls_json],
        )?;
        Ok(())
    })
    .expect("insert assistant message with tool_calls");

    // Build and write a turn-boundary checkpoint (the production write path).
    // We construct the messages vector directly — in production, write_turn_checkpoint
    // fetches them via MessageStore::replay, but the extraction logic is identical.
    let logs = log_service.get_session_detail(&sid).unwrap().unwrap().logs;
    let msgs = vec![zbot_conversation::Message {
        id: "msg-test".to_string(),
        execution_id: Some(sid.clone()),
        session_id: sid.clone(),
        role: "assistant".to_string(),
        content: "[tool calls]".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        token_count: 0,
        tool_calls: Some(tool_calls_json),
        tool_call_id: None,
        seq: 1,
    }];

    // Use the public build_context_state API (same as write_turn_checkpoint)
    let ctx = gateway_execution::build_context_state(
        &logs,
        &msgs,
        "Final answer from checkpoint",
        None, // ward_id
        None, // title
    );
    let context_state_json = serde_json::to_string(&ctx).unwrap();

    checkpoints
        .write(&zbot_conversation::Checkpoint {
            id: format!("cp-{}", uid()),
            execution_id: sid.clone(),
            session_id: sid.clone(),
            llm_turn: 1,
            last_message_id: String::new(),
            pending_tool_calls: None,
            context_state: Some(context_state_json),
            child_executions: None,
            schema_version: 1,
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .unwrap();

    let state = builder.build(&sid).unwrap().expect("session should exist");

    // Plan comes from messages via context_state checkpoint, NOT from logs
    assert_eq!(state.plan.len(), 2);
    assert_eq!(state.plan[0].text, "Step A");
    assert_eq!(state.plan[1].text, "Step B");

    // Response comes from the checkpoint's context_state
    assert_eq!(
        state.response.as_deref(),
        Some("Final answer from checkpoint")
    );
}
