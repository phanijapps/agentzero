//! Task-level golden runs — end-to-end scripted sessions asserting
//! agent-loop SIDE EFFECTS (not event replay, which `golden_trace_tests`
//! owns). Research-backlog item #1: the harness that would have caught
//! this week's live bugs as CI failures.
//!
//! Each scenario boots the full `ExecutionRunner` against a scripted SSE
//! provider, drives a scripted multi-agent session, and asserts observable
//! state: session/ward bindings (the 9ms race), planner roster manifest
//! (the 12-query discovery loop), procedure contract, memory persistence,
//! and fast-path behavior.
//!
//! Run all: `cargo test -p gateway-execution --features test-stubs
//! --test golden_tasks -- --ignored --nocapture`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use agent_primitives::vault_paths::VaultPaths;
use api_logs::LogService;
use execution_state::{SqliteWorkStore, WorkStore};
use gateway_bus::LocalWorkTransport;
use gateway_events::{EventBus, GatewayEvent};
use gateway_services::agents::Agent;
use gateway_services::providers::Provider;
use gateway_services::{AgentService, McpService, ProviderService};
use zbot_stores_traits::{MemoryFactStore, Procedure, ProcedureStore};

use execution_state::StateService;
use zbot_engram_adapter::{
    AdapterConfig, EngramMemoryFactStore, EngramProvider, EngramSidecarStores,
};
use zbot_runtime_sqlite::DatabaseManager;

// ---------------------------------------------------------------------------
// Scripted SSE provider
// ---------------------------------------------------------------------------

/// One scripted assistant turn: a single tool call.
#[derive(Clone)]
struct Turn {
    tool: &'static str,
    args: String,
}

fn turn(tool: &'static str, args: serde_json::Value) -> Turn {
    Turn {
        tool,
        args: serde_json::to_string(&args).unwrap(),
    }
}

/// Serve the scripted conversation: each accepted connection is one
/// completion request; the caller identity is derived from the request
/// body (each seeded agent carries a unique instruction marker), and the
/// next scripted turn for that agent is served. Bodies are captured for
/// prompt-level assertions (e.g. the planner roster manifest).
struct ScriptedProvider {
    base_url: String,
    bodies: Arc<tokio::sync::Mutex<Vec<String>>>,
}

impl ScriptedProvider {
    /// `scripts`: agent marker -> ordered turns. Markers must appear in the
    /// agent's instruction text so the request can be routed.
    async fn spawn(scripts: HashMap<&'static str, Vec<Turn>>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let queues = Arc::new(tokio::sync::Mutex::new(
            scripts
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect::<HashMap<_, _>>(),
        ));
        let bodies = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let bodies_clone = bodies.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let queues = queues.clone();
                let bodies = bodies_clone.clone();
                tokio::spawn(async move {
                    let body = read_request_body(&mut socket).await;
                    bodies.lock().await.push(body.clone());
                    // Intent-analyzer requests (plain chat + JSON parse)
                    // get a canned simple-intent response so they never
                    // consume scripted model turns.
                    if body.contains("You are an intent analyzer") {
                        let intent_json = r#"{\"primary_intent\":\"golden-task\",\"hidden_intents\":[],\"recommended_skills\":[],\"recommended_agents\":[],\"ward_recommendation\":{\"action\":\"use_existing\",\"ward_name\":\"general\",\"reason\":\"scripted\"},\"execution_strategy\":{\"approach\":\"simple\",\"explanation\":\"scripted\"}}"#;
                        let body = format!(
                            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{intent_json}\"}},\"finish_reason\":null}}]}}\n\n\
                             data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}}}\n\n\
                             data: [DONE]\n\n"
                        );
                        write_sse(&mut socket, &body).await;
                        return;
                    }
                    // Route by marker; unmatched markers fall to "__root__"
                    // (the root agent's prompt carries no child marker).
                    let marker = queues
                        .lock()
                        .await
                        .keys()
                        .find(|marker| body.contains(marker.as_str()))
                        .cloned()
                        .unwrap_or_else(|| "__root__".to_string());
                    let next = {
                        let mut guard = queues.lock().await;
                        let script = guard.get_mut(&marker).expect("script for caller");
                        if script.is_empty() {
                            // Fall back to a terminal respond so a
                            // mis-scripted scenario fails loudly at the
                            // assertion layer, not by hanging the engine.
                            Turn {
                                tool: "respond",
                                args: r#"{"message":"script exhausted"}"#.to_string(),
                            }
                        } else {
                            script.remove(0)
                        }
                    };
                    let escaped = next.args.replace('\\', "\\\\").replace('"', "\\\"");
                    let body = format!(
                        "data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"index\":0,\"id\":\"call-g\",\"function\":{{\"name\":\"{}\",\"arguments\":\"{}\"}}}}]}},\"finish_reason\":null}}]}}\n\n\
                         data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\n\
                         data: [DONE]\n\n",
                        next.tool, escaped
                    );
                    write_sse(&mut socket, &body).await;
                });
            }
        });
        Self {
            base_url: format!("http://{address}/v1"),
            bodies,
        }
    }
}

async fn read_request_body(stream: &mut tokio::net::TcpStream) -> String {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut expected = None;
    loop {
        let read = stream.read(&mut buffer).await.unwrap();
        assert!(read > 0, "client closed before sending the request");
        bytes.extend_from_slice(&buffer[..read]);
        if expected.is_none() {
            if let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..header_end]).to_lowercase();
                let content_length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or_default();
                expected = Some(header_end + 4 + content_length);
            }
        }
        if let Some(length) = expected {
            if bytes.len() >= length {
                let header_end = bytes
                    .windows(4)
                    .position(|part| part == b"\r\n\r\n")
                    .map(|end| end + 4)
                    .unwrap_or(0);
                return String::from_utf8_lossy(&bytes[header_end..]).to_string();
            }
        }
    }
}

async fn write_sse(stream: &mut tokio::net::TcpStream, body: &str) {
    use tokio::io::AsyncWriteExt;
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).await.unwrap();
    stream.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct TaskHarness {
    _temp: tempfile::TempDir,
    runner: gateway_execution::ExecutionRunner,
    event_bus: Arc<EventBus>,
    state: Arc<StateService<DatabaseManager>>,
    paths: Arc<VaultPaths>,
    bodies: Arc<tokio::sync::Mutex<Vec<String>>>,
    fact_store: Arc<dyn MemoryFactStore>,
    procedure_store: Arc<dyn ProcedureStore>,
}

async fn build_harness(provider: ScriptedProvider) -> TaskHarness {
    let temp = tempfile::tempdir().unwrap();
    let paths: Arc<VaultPaths> = Arc::new(VaultPaths::new(temp.path().to_path_buf()));
    paths.ensure_dirs_exist().unwrap();
    // Production seeds the ward archetype registry at AppState bootstrap;
    // the direct-runner harness must do the same before any ward create.
    gateway_services::seed_default_ward_archetypes(&paths).expect("seed ward archetypes");
    let db = Arc::new(DatabaseManager::new(paths.clone()).unwrap());
    let state = Arc::new(StateService::new(db.clone()));

    let provider_service = Arc::new(ProviderService::new(paths.clone()));
    provider_service
        .create(Provider {
            id: Some("provider-golden".to_owned()),
            name: "Golden".to_owned(),
            description: "scripted".to_owned(),
            api_key: "test-key".to_owned(),
            base_url: provider.base_url.clone(),
            models: vec!["test-model".to_owned()],
            embedding_models: None,
            embedding_dimensions: None,
            verified: Some(true),
            is_default: true,
            created_at: None,
            max_concurrent_requests: None,
            context_window: Some(32_768),
            default_model: Some("test-model".to_owned()),
            rate_limits: None,
            model_configs: None,
        })
        .unwrap();

    let agent_service = Arc::new(AgentService::new(paths.agents_dir()));
    for (id, instructions) in [
        ("planner-agent", "PLANNERMARK plan the work"),
        ("builder-agent", "BUILDERMARK build the work"),
        ("research-agent", "RESEARCHMARK research the work"),
    ] {
        agent_service
            .create(Agent {
                id: id.to_owned(),
                name: id.to_owned(),
                display_name: id.to_owned(),
                description: id.to_owned(),
                agent_type: Some("specialist".to_owned()),
                provider_id: "provider-golden".to_owned(),
                model: "test-model".to_owned(),
                temperature: 0.0,
                max_input_tokens: 32_768,
                max_input_tokens_explicit: true,
                max_tokens: 256,
                thinking_enabled: false,
                voice_recording_enabled: false,
                system_instruction: None,
                instructions: instructions.to_owned(),
                mcps: vec![],
                skills: vec![],
                middleware: None,
                created_at: None,
            })
            .await
            .unwrap();
    }

    let pool = zbot_conversation::open_conversation_pool(&paths.conversations_db()).unwrap();
    let messages: Arc<dyn zbot_conversation::MessageStore> =
        Arc::new(zbot_conversation::SqliteMessageStore::new(pool.clone()));
    let session_meta: Arc<dyn zbot_conversation::SessionMetaStore> =
        Arc::new(zbot_conversation::SqliteSessionMetaStore::new(pool.clone()));
    let checkpoints: Arc<dyn zbot_conversation::CheckpointStore> =
        Arc::new(zbot_conversation::SqliteCheckpointStore::new(pool));
    let work_store: Arc<dyn WorkStore> = Arc::new(SqliteWorkStore::new(db.clone()));

    // Durable stores on the engram in-memory sidecar (same construction the
    // conformance suite uses) so memory/procedure side effects are assertable.
    let adapter_config = AdapterConfig::engram_for_data_root(temp.path(), "golden-engram");
    let engram = EngramProvider::open(adapter_config.clone()).expect("engram provider");
    let fact_store: Arc<dyn MemoryFactStore> = Arc::new(
        EngramMemoryFactStore::from_provider(adapter_config.clone(), &engram).expect("fact store"),
    );
    let procedure_store: Arc<dyn ProcedureStore> =
        Arc::new(EngramSidecarStores::from_provider(adapter_config, &engram).expect("sidecars"));

    let peer_messages = Arc::new(
        gateway_execution::peer_messaging::DurablePeerMessageService::new(
            work_store,
            Arc::new(LocalWorkTransport::new()),
            state.clone(),
            gateway_execution::peer_messaging::PEER_MESSAGE_TARGET,
        ),
    );

    let event_bus = Arc::new(EventBus::new());
    let runner =
        gateway_execution::ExecutionRunner::with_config(gateway_execution::ExecutionRunnerConfig {
            event_bus: event_bus.clone(),
            agent_service,
            provider_service,
            paths: paths.clone(),
            mcp_service: Arc::new(McpService::new(paths.clone())),
            skill_service: Arc::new(gateway_services::SkillService::new(paths.skills_dir())),
            log_service: Arc::new(LogService::new(db)),
            state_service: state.clone(),
            ward_usage: Arc::new(gateway_services::WardUsage::new(paths.wards_dir())),
            messages,
            session_meta,
            checkpoints,
            connector_registry: None,
            memory_store: Some(fact_store.clone()),
            distiller: None,
            handoff_writer: None,
            memory_recall: None,
            peer_messages: Some(peer_messages),
            a2a_delegation: None,
            bridge_registry: None,
            bridge_outbox: None,
            embedding_client: None,
            procedure_store: Some(procedure_store.clone()),
            procedure_recommendation_cfg: gateway_memory::ProcedureRecommendationConfig::default(),
            max_parallel_agents: 1,
        });
    TaskHarness {
        _temp: temp,
        runner,
        event_bus,
        state,
        paths,
        bodies: provider.bodies,
        fact_store,
        procedure_store,
    }
}

impl TaskHarness {
    /// Run one session to root completion; assert the root completes
    /// within the timeout. Returns the session id.
    async fn run_to_completion(&self, prompt: &str) -> String {
        let (_, session_id) = self
            .runner
            .invoke_with_callback(
                gateway_execution::ExecutionConfig::new(
                    "root".to_owned(),
                    "golden-tasks".to_owned(),
                    self.paths.vault_dir().clone(),
                )
                .with_mode("chat".to_owned()),
                prompt.to_owned(),
                None,
            )
            .await
            .expect("invoke");
        let root_execution = self
            .state
            .get_root_execution(&session_id)
            .unwrap()
            .expect("root execution")
            .id;
        let mut completion = self.event_bus.subscribe_all();
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                match completion.recv().await.expect("event stream open") {
                    GatewayEvent::AgentCompleted {
                        session_id: s,
                        execution_id,
                        ..
                    } if s == session_id && execution_id == root_execution => break,
                    _ => {}
                }
            }
        })
        .await
        .expect("root completion within timeout");
        session_id
    }
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// Scenario 1 — ward-then-plan: the full graph flow plus the two live-bug
/// regressions (ward binding race, planner roster manifest).
///
/// Would have caught: sess-70d057a3 (planner spawned ward-less 9ms after
/// ward creation), sess-fd588249 (12 lookup_capabilities calls hunting for
/// the roster).
#[tokio::test]
#[ignore = "full scripted session; use --ignored"]
async fn golden_task_ward_then_plan() {
    let mut scripts = HashMap::new();
    scripts.insert(
        "__root__",
        vec![
            turn("ward", serde_json::json!({"action": "use", "name": "goldward"})),
            turn(
                "delegate_to_agent",
                serde_json::json!({"agent_id": "planner-agent", "task": "plan the build", "wait_for_result": true}),
            ),
            turn("respond", serde_json::json!({"message": "done after plan"})),
        ],
    );
    scripts.insert(
        "PLANNERMARK",
        vec![turn(
            "delegate_to_agent",
            serde_json::json!({"agent_id": "builder-agent", "task": "build the artifact", "wait_for_result": true}),
        )],
    );
    scripts.insert(
        "BUILDERMARK",
        vec![turn(
            "respond",
            serde_json::json!({"message": "built the artifact"}),
        )],
    );
    let provider = ScriptedProvider::spawn(scripts).await;
    let harness = build_harness(provider).await;

    let session_id = harness
        .run_to_completion("research and build in goldward")
        .await;

    // (a) Ward binding: the ROOT session row is bound — the async
    // __ward_changed__ write may lag, so poll briefly like production
    // continuations would observe it.
    let mut bound = false;
    for _ in 0..50 {
        if let Some(session) = harness.state.get_session(&session_id).unwrap() {
            if session.ward_id.as_deref() == Some("goldward") {
                bound = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(bound, "root session must be ward-bound after the ward tool");

    // (b) THE 9ms-race regression: the planner CHILD session row must carry
    // the ward the root entered — the fallback reads the durable tool
    // result, so this must hold even when the async write lags.
    let _child_messages = harness
        .state
        .get_session_messages(
            &session_id,
            &execution_state::handlers::SessionMessagesQuery {
                scope: execution_state::handlers::MessageScope::Delegates,
                execution_id: None,
                agent_id: None,
            },
        )
        .unwrap();
    let child_sessions = child_sessions_of(&harness, &session_id);
    let planner_session = child_sessions
        .iter()
        .find(|(agent, _)| agent == "planner-agent")
        .expect("planner child session");
    let planner_row = harness
        .state
        .get_session(&planner_session.1)
        .unwrap()
        .expect("planner session row");
    assert_eq!(
        planner_row.ward_id.as_deref(),
        Some("goldward"),
        "planner child must inherit the ward binding (9ms-race regression)"
    );

    // (c) The planner's actual request carried the roster manifest —
    // asserted against the captured LLM request body, not internals.
    let bodies = harness.bodies.lock().await;
    let planner_body = bodies
        .iter()
        .find(|body| body.contains("PLANNERMARK"))
        .expect("planner made an LLM call");
    assert!(
        planner_body.contains("## Available Agents"),
        "planner prompt must carry the roster manifest (12-query loop regression)"
    );
    assert!(
        planner_body.contains("builder-agent"),
        "roster must list the delegatable builder"
    );

    // (d) No discovery groping: no request body ever asked for
    // lookup_capabilities (it would appear as a tool_call in messages).
    let all_messages = harness
        .state
        .get_session_messages(
            &session_id,
            &execution_state::handlers::SessionMessagesQuery::default(),
        )
        .unwrap();
    assert!(
        all_messages
            .iter()
            .all(|message| !message.content.contains("lookup_capabilities")),
        "no lookup_capabilities anywhere in the session (roster is inline)"
    );

    // (e) The ward directory exists on disk.
    assert!(
        harness
            .paths
            .ward_dir("goldward")
            .join("AGENTS.md")
            .exists(),
        "ward scaffolding written"
    );
}

fn child_sessions_of(harness: &TaskHarness, session_id: &str) -> Vec<(String, String)> {
    // Child sessions are derived rows: parent_session_id = root session.
    // The state service exposes them through the executions join.
    let db = harness.state.db_handle();
    db.with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT s.id FROM sessions s \
                 WHERE s.parent_session_id = ?1",
            )
            .unwrap();
        let rows = stmt
            .query_map(rusqlite::params![session_id], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        Ok(rows)
    })
    .unwrap()
    .into_iter()
    .map(|id| {
        let agent = harness
            .state
            .get_session(&id)
            .unwrap()
            .map(|s| s.root_agent_id.clone())
            .unwrap_or_default();
        (agent, id)
    })
    .collect()
}

/// Scenario 2 — procedure contract: a seeded procedure with declared
/// parameters is invoked with those arguments.
///
/// Would have caught: sess-05ba0fd4 (procedure recalled without its
/// Parameters contract; model called it bare and burned turns).
#[tokio::test]
#[ignore = "full scripted session; use --ignored"]
async fn golden_task_procedure_contract() {
    let provider = ScriptedProvider::spawn(HashMap::from([(
        "__root__",
        vec![
            turn(
                "run_procedure",
                serde_json::json!({"name": "golden_greet", "args": {"who": "world"}}),
            ),
            turn("respond", serde_json::json!({"message": "procedure ran"})),
        ],
    )]))
    .await;
    let harness = build_harness(provider).await;

    // Seed the procedure with a declared parameter contract and one echo step.
    harness
        .procedure_store
        .upsert_procedure(
            Procedure {
                id: "proc-golden".to_owned(),
                agent_id: "root".to_owned(),
                ward_id: None,
                name: "golden_greet".to_owned(),
                description: "greets".to_owned(),
                trigger_pattern: None,
                steps: r#"[{"action":"shell","args":{"command":"echo hello {args.who}"}}]"#
                    .to_owned(),
                parameters: Some(r#"["who"]"#.to_owned()),
                success_count: 0,
                failure_count: 0,
                avg_duration_ms: None,
                avg_token_cost: None,
                last_used: None,
                embedding: None,
                created_at: "2026-09-11T00:00:00Z".to_owned(),
                updated_at: "2026-09-11T00:00:00Z".to_owned(),
            },
            None,
        )
        .await
        .expect("seed procedure");

    let session_id = harness
        .run_to_completion("run the golden greet procedure for world")
        .await;

    // The procedure executed with the supplied args: the tool result
    // carries all_steps, and the shell step's stdout proves {args.who}
    // interpolated end-to-end.
    let messages = harness
        .state
        .get_session_messages(
            &session_id,
            &execution_state::handlers::SessionMessagesQuery::default(),
        )
        .unwrap();
    let tape: String = messages
        .iter()
        .map(|message| message.content.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        tape.contains("hello world"),
        "procedure step must interpolate args.who into the step output: tape={tape}"
    );
    assert!(
        tape.contains("\"procedure\":\"golden_greet\""),
        "run_procedure result must name the executed procedure"
    );
    // The counter accounting incremented on use.
    let stored = harness
        .procedure_store
        .get_procedure_by_name("root", "golden_greet")
        .await
        .unwrap()
        .expect("procedure persisted");
    assert!(
        stored.success_count + stored.failure_count >= 1,
        "run_procedure must record the outcome (success={}, failure={})",
        stored.success_count,
        stored.failure_count
    );
}

/// Scenario 3 — memory persistence: facts written mid-session are durable
/// and recallable.
#[tokio::test]
#[ignore = "full scripted session; use --ignored"]
async fn golden_task_memory_persistence() {
    let provider = ScriptedProvider::spawn(HashMap::from([(
        "__root__",
        vec![
            turn(
                "memory_write",
                serde_json::json!({
                    "category": "pattern",
                    "key": "pattern.golden.marker",
                    "content": "golden-task fact persisted",
                    "confidence": 0.9
                }),
            ),
            turn("respond", serde_json::json!({"message": "saved"})),
        ],
    )]))
    .await;
    let harness = build_harness(provider).await;

    let session_id = harness
        .run_to_completion("remember something important")
        .await;
    assert!(!session_id.is_empty());

    // The fact is durable in the store with the written shape. The
    // harness wires no embedding client, so semantic recall degrades —
    // assert through the list path (production read used by /api/memory).
    let rows = harness
        .fact_store
        .list_memory_facts(Some("root"), Some("pattern"), None, 50, 0)
        .await
        .expect("list facts");
    let hit = rows.iter().any(|row| {
        row.get("key").and_then(|k| k.as_str()) == Some("pattern.golden.marker")
            && row
                .get("content")
                .and_then(|c| c.as_str())
                .is_some_and(|c| c.contains("golden-task fact persisted"))
    });
    assert!(hit, "memory_write fact must be durable: {rows:?}");
}

/// Scenario 4 — simple fast path: no delegation, no ward, direct respond.
#[tokio::test]
#[ignore = "full scripted session; use --ignored"]
async fn golden_task_simple_fast_path() {
    let provider = ScriptedProvider::spawn(HashMap::from([(
        "__root__",
        vec![turn(
            "respond",
            serde_json::json!({"message": "quick answer"}),
        )],
    )]))
    .await;
    let harness = build_harness(provider).await;

    let mut events = harness.event_bus.subscribe_all();
    let session_id = harness.run_to_completion("quick question").await;

    // No delegation events fired.
    let mut stray = None;
    while let Ok(event) = events.try_recv() {
        if let GatewayEvent::DelegationStarted { session_id: s, .. } = &event {
            if *s == session_id {
                stray = Some(event);
            }
        }
    }
    assert!(stray.is_none(), "fast path must not delegate");
    // No ward directory created.
    let wards = std::fs::read_dir(harness.paths.wards_dir()).unwrap();
    assert_eq!(wards.count(), 0, "fast path must not create wards");
}

/// Scenario 5 — parallel delegation join: two `parallel: true` children
/// fire back-to-back without per-session claim blocking, and the root
/// resumes only after BOTH complete (the continuation-watcher join).
///
/// Harness semaphore is `max_parallel_agents: 1`, so the second child
/// queues at the DISPATCHER (semaphore) — never at the per-session
/// delegation claim. Both orderings (true-concurrent with a higher
/// semaphore, dispatcher-queued with 1) satisfy this scenario; what it
/// pins is: both accepted, both complete, root joins, no claim block.
#[tokio::test]
#[ignore = "full scripted session; use --ignored"]
async fn golden_task_parallel_join() {
    let mut scripts = HashMap::new();
    scripts.insert(
        "__root__",
        vec![
            turn(
                "delegate_to_agent",
                serde_json::json!({"agent_id": "builder-agent", "task": "build it", "wait_for_result": true, "parallel": true}),
            ),
            turn(
                "delegate_to_agent",
                serde_json::json!({"agent_id": "research-agent", "task": "research it", "wait_for_result": true, "parallel": true}),
            ),
            turn("respond", serde_json::json!({"message": "done after both"})),
        ],
    );
    scripts.insert(
        "BUILDERMARK",
        vec![turn("respond", serde_json::json!({"message": "built"}))],
    );
    scripts.insert(
        "RESEARCHMARK",
        vec![turn(
            "respond",
            serde_json::json!({"message": "researched"}),
        )],
    );
    let provider = ScriptedProvider::spawn(scripts).await;
    let harness = build_harness(provider).await;

    // Event ordering witness: subscribe BEFORE invoke; arrival order in a
    // single subscription records the join semantics (root completion must
    // arrive after both child completions).
    let mut ordering = harness.event_bus.subscribe_all();
    let session_id = harness
        .run_to_completion("build and research in parallel")
        .await;

    // (a) Both children spawned: two distinct child sessions with the
    // right agents, linked to the parent session.
    let child_sessions = child_sessions_of(&harness, &session_id);
    let builder = child_sessions
        .iter()
        .find(|(agent, _)| agent == "builder-agent")
        .expect("builder child session");
    let research = child_sessions
        .iter()
        .find(|(agent, _)| agent == "research-agent")
        .expect("research child session");
    assert_ne!(builder.1, research.1, "children are distinct sessions");

    // (b) THE claim-bypass regression: the second parallel delegation must
    // be accepted — its tool result confirms the delegation, never the
    // per-session claim rejection ("You already have an active delegation").
    let messages = harness
        .state
        .get_session_messages(
            &session_id,
            &execution_state::handlers::SessionMessagesQuery::default(),
        )
        .unwrap();
    let tape: String = messages
        .iter()
        .map(|message| message.content.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !tape.contains("You already have an active delegation"),
        "parallel delegation must bypass the per-session claim — tape={tape}"
    );
    assert!(
        tape.contains("Task delegated to research-agent"),
        "the second (parallel) delegation must be accepted; tape={tape}"
    );

    // (c) The join: root completion arrives AFTER both children complete.
    // Drain the ordering witness; indices prove the continuation watcher
    // resumed the root only once every child finished.
    #[derive(Default)]
    struct Seen {
        builder_completed: Option<usize>,
        research_completed: Option<usize>,
        root_completed: Option<usize>,
        delegations: Vec<String>,
    }
    let mut seen = Seen::default();
    let mut index = 0usize;
    while let Ok(event) = ordering.try_recv() {
        match event {
            GatewayEvent::AgentCompleted { agent_id, .. } => {
                if agent_id == "builder-agent" {
                    seen.builder_completed = Some(index);
                } else if agent_id == "research-agent" {
                    seen.research_completed = Some(index);
                } else if agent_id == "root" {
                    seen.root_completed = Some(index);
                }
            }
            GatewayEvent::DelegationStarted { child_agent_id, .. } => {
                seen.delegations.push(child_agent_id);
            }
            _ => {}
        }
        index += 1;
    }
    assert!(
        seen.delegations
            .iter()
            .any(|agent| agent == "builder-agent")
            && seen
                .delegations
                .iter()
                .any(|agent| agent == "research-agent"),
        "both delegations started: {:?}",
        seen.delegations
    );
    let (builder_at, research_at, root_at) = (
        seen.builder_completed.expect("builder completed event"),
        seen.research_completed.expect("research completed event"),
        seen.root_completed.expect("root completed event"),
    );
    assert!(
        root_at > builder_at && root_at > research_at,
        "root must resume only after BOTH children complete \
         (builder@{builder_at}, research@{research_at}, root@{root_at})"
    );

    // (d) The final respond reached and no discovery groping.
    assert!(
        tape.contains("done after both"),
        "root responded after the join"
    );
    assert!(
        !tape.contains("lookup_capabilities"),
        "no lookup_capabilities anywhere"
    );
}

// ---------------------------------------------------------------------------
// Smoke (non-ignored): fixtures parse and the stack boots.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scripted_provider_and_harness_boot() {
    let provider = ScriptedProvider::spawn(HashMap::from([(
        "__root__",
        vec![turn("respond", serde_json::json!({"message": "boot"}))],
    )]))
    .await;
    let harness = build_harness(provider).await;
    // The harness constructs the full runner with durable stores wired.
    let session = harness
        .state
        .create_session("root")
        .expect("state service live");
    assert!(!session.0.id.is_empty());
}
