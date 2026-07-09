# Pattern 3 — Agent Pool Design

**Date:** 2026-05-11
**Status:** ✅ **Shipped** — implementation live on `develop`. See file map below for the as-built layout (final landing differs slightly from the original sketch: `delegation/spawn.rs` owns the resolve/reject calls rather than `delegation/callback.rs`; `AgentResultBus::kill` consolidates the `kill_agent` path on the bus rather than a separate `handles` map). Test coverage added 2026-05-17: 11 bus tests + 8 tool tests.
**Context:** philschmid.de/subagent-patterns-2026, Pattern 3: "Agent Pool — Persistent Agents with Messaging"
**Reviewed by:** 2-agent parallel codebase analysis (delegation/callback.rs, spawn.rs, executor.rs, StateService)

## As-Built Locations

| Component | File | Notes |
|-----------|------|-------|
| `AgentResultBus` + `AgentResult` + `AgentWaitError` | `gateway/gateway-execution/src/agent_pool/result_bus.rs` | One bus owns both `waiting: HashMap<execution_id, oneshot::Sender>` AND `execution_handles: HashMap<execution_id, ExecutionHandle>` — no separate handles map needed. |
| `WaitAgentTool` | `gateway/gateway-execution/src/tools/wait_agent.rs` | Fast-path via `StateService::get_execution` + `conversation_repo.get_session_conversation` returns the last assistant message immediately. Slow-path registers a waiter and times out. |
| `KillAgentTool` | `gateway/gateway-execution/src/tools/kill_agent.rs` | Calls `bus.kill(execution_id)` which both `handle.stop()`s the running execution AND rejects any pending waiter with `Crashed { error: "killed by orchestrator" }`. |
| Resolve/reject hook | `gateway/gateway-execution/src/delegation/spawn.rs:750/771/795` | NOT `callback.rs` (the original design pointed there). Resolve fires after the subagent's `respond` call; reject on failure paths. |
| Runner construction | `gateway/gateway-execution/src/runner/core.rs:408,591,894` | `ExecutionRunner::with_config()` builds the bus; `ContinuationArgs` carries it; `invoke_continuation` threads it into the executor. |
| Tool registration | `gateway/gateway-execution/src/invoke/executor.rs:621-633` | Registered only when `agent_result_bus + state_service + conversation_repo` are all wired — i.e. the root path. Subagents don't get these tools. |

---

## What Pattern 3 Enables

The root orchestrator coordinates specialist agents incrementally:

```
root → delegate_to_agent(researcher, "find sources")
root → wait_agent(exec-r, timeout=300)     ← blocks
[researcher does work, calls respond("found 5 sources...")]
root ← {"result": "found 5 sources..."}   ← unblocks
root → delegate_to_agent(writer, "use these sources: ...")
root → wait_agent(exec-w, timeout=300)     ← blocks
[writer writes draft, calls respond("draft complete...")]
root ← {"result": "draft complete..."}    ← unblocks
root → delegate_to_agent(researcher, "fact-check this draft")
...
root → respond("here is the final post")
```

The root stays alive the entire time, blocking on individual agent results and routing them between specialists. No human in the loop. No round-trip through the continuation mechanism.

**Why this matters for enterprise z-Bot:**  
An HR bot coordinating onboarding: delegates to IT (provision access), waits for confirmation, then delegates to Finance (set up payroll) with the IT confirmation included, waits, then sends final status to Legal. Each step uses the previous step's result. The root orchestrates; it doesn't just fan out and aggregate.

---

## Gap Analysis: z-Bot Today vs Pattern 3

| Pattern 3 tool | z-Bot equivalent | Status |
|---|---|---|
| `spawn_agent(role, task)` | `delegate_to_agent` | ✅ exists |
| `send_message(agent_id, msg)` | `steer_agent` | ✅ exists (root → subagent) |
| `wait_agent(execution_id, timeout)` | nothing | ❌ missing |
| `kill_agent(execution_id)` | nothing | ❌ missing |
| `list_agents()` | derivable from memory/state | ⚠ low priority |

One critical new primitive: **`wait_agent`**. Everything else follows from it.

---

## Key Design Decisions (research-validated)

### wait_agent blocks the root executor safely
The executor runs tool calls via `join_all` / `.await` with no timeout wrapper (`executor.rs:1022–1051`). A `tokio::sync::oneshot::Receiver::await` suspends the Tokio task cheaply until resolved. The root executor task stays alive but yields — no resource waste, no watchdog to fight.

### No spurious continuations fire
`SessionContinuationReady` fires only when `pending_delegations == 0 AND continuation_needed`. `continuation_needed` is set only when root calls `respond`. While root is blocked on `wait_agent` (not having called `respond` yet), individual agent completions call `complete_delegation` which decrements the counter and returns `false`. The continuation mechanism is completely uninvolved. When root eventually calls `respond`, `pending_delegations` is already 0 and the session ends cleanly.

### single_action_mode means one step per LLM turn
Root has `single_action_mode = true` — one tool call per LLM response. Pattern 3 naturally maps to alternating turns:
```
Turn 1: delegate_to_agent(researcher, task)   → gets execution_id
Turn 2: wait_agent(execution_id, 300)          → blocks, returns result
Turn 3: delegate_to_agent(writer, result)      → gets execution_id
Turn 4: wait_agent(execution_id, 300)          → blocks, returns result
Turn 5: respond(synthesis)
```

### Interception point: handle_delegation_success
`delegation/callback.rs:handle_delegation_success` receives `response: &str` — the complete accumulated text from the subagent's `respond()` call plus any streaming tokens. This is the canonical result. Inject `agent_result_bus.resolve(execution_id, result)` here, before the callback is written to the DB.

For the crash path: `handle_delegation_failure` is the equivalent — inject `agent_result_bus.reject(execution_id, Crashed)`.

### Event bus subscription is NOT used
`GatewayEvent::DelegationCompleted` carries the result and could be subscribed to. But direct injection in callback.rs is simpler, more reliable, and keeps the resolution path synchronous with the callback — no race conditions.

### kill_agent uses existing ExecutionHandle
`ExecutionHandle::stop()` already exists. The handle is in `ExecutionRunner::handles` keyed by `conversation_id`. `kill_agent` needs the `handles` map — same access pattern used by `ExecutionRunner::stop()`.

---

## Core Abstraction: `AgentResultBus`

```rust
// gateway/gateway-execution/src/agent_pool/result_bus.rs

pub struct AgentResult {
    pub execution_id: String,
    pub agent_id: String,
    pub response: String,
}

pub enum AgentWaitError {
    Timeout,
    Crashed { error: String },
    NotFound(String),
}

pub struct AgentResultBus {
    // execution_id → pending oneshot sender
    waiting: Mutex<HashMap<String, oneshot::Sender<Result<AgentResult, AgentWaitError>>>>,
}

impl AgentResultBus {
    pub fn new() -> Self { ... }

    // Called by wait_agent tool: registers and returns receiver
    pub async fn register(&self, execution_id: String)
        -> oneshot::Receiver<Result<AgentResult, AgentWaitError>>;

    // Called from handle_delegation_success: resolves any registered waiter
    pub async fn resolve(&self, execution_id: &str, agent_id: &str, response: &str);

    // Called from handle_delegation_failure: rejects any registered waiter
    pub async fn reject(&self, execution_id: &str, error: AgentWaitError);
}
```

No secondary index needed (unlike Pattern 4's ReplyStore) — keyed only by `execution_id`, and resolution is always point-to-point (one waiter per execution).

---

## Two New Tools

Both live in `gateway/gateway-execution/src/tools/` alongside `SteerAgentTool`.

### `wait_agent`

```
wait_agent(
  execution_id: String,    // required — from delegate_to_agent response
  timeout_secs: u32 = 300  // 5 minutes default (subagents do real work)
)
```

Tool struct holds `Arc<AgentResultBus>` and `Arc<StateService<DatabaseManager>>`.

Before registering, checks StateService: if the execution is already completed (fast path — agent finished before root called wait_agent), return the result from conversation history immediately. If still running, register and block.

Returns (JSON):
```json
// success
{"execution_id": "exec-x", "agent_id": "researcher", "result": "Found 5 sources: ..."}

// timeout — agent still running, root can retry or kill
{"error": "timeout", "execution_id": "exec-x"}

// agent crashed
{"error": "crashed", "execution_id": "exec-x", "details": "shell command failed: ..."}

// unknown execution_id
{"error": "not_found", "execution_id": "exec-x"}
```

### `kill_agent`

```
kill_agent(
  execution_id: String    // required
)
```

Tool struct holds `Arc<RwLock<HashMap<String, ExecutionHandle>>>` (the runner's handles map).

Calls `handle.stop()`. Returns `{"stopped": true}` or `{"status": "not_running"}`.
Also calls `agent_result_bus.reject(execution_id, AgentWaitError::Crashed { error: "killed by orchestrator" })` to unblock any pending `wait_agent` on this execution.

---

## Data Flow

```
root: delegate_to_agent(researcher, "find WebAssembly benchmarks")
  → register_delegation(session_id)    pending=1
  → returns {execution_id: "exec-r"}

root: wait_agent("exec-r", timeout=300)
  → checks StateService: exec-r status=running → not done yet
  → result_bus.register("exec-r") → gets rx
  → tokio::time::timeout(300s, rx).await   ← root executor suspends

[researcher runs independently on separate Tokio task]
[researcher calls respond("Found 5 sources: Chrome 1.2x native...")]
  → executor fires StreamEvent::ActionRespond
  → ResponseAccumulator appends the message
  → executor exits, spawn.rs handle_execution_success fires
  → complete_execution("exec-r") → status=Completed in DB
  → complete_delegation(session_id) → pending=0, continuation_needed=false → returns false (no continuation)
  → agent_result_bus.resolve("exec-r", "researcher", "Found 5 sources...")  ← NEW
  → handle_delegation_success writes callback to parent session DB

root: wait_agent unblocks
  → tool returns {"execution_id": "exec-r", "agent_id": "researcher", "result": "Found 5 sources..."}

root: delegate_to_agent(writer, "Write a post using: Found 5 sources...")
  → pending=1
  → returns {execution_id: "exec-w"}

root: wait_agent("exec-w", timeout=300)
  → ...same pattern...

root: respond("Here is the final blog post: ...")
  → executor stops, session ends cleanly
```

---

## Fast-path: agent already completed before wait_agent is called

```
root: delegate_to_agent(researcher)    → exec-r running
[researcher completes immediately]
[agent_result_bus.resolve("exec-r", ...) — no waiter registered yet, dropped]

root: wait_agent("exec-r")
  → checks StateService: exec-r status=Completed
  → fetches result from conversation_repo.get_session_conversation(child_session_id, 1)
    (the child's last assistant message = the respond() payload)
  → returns result immediately, no blocking
```

The fast-path requires `child_session_id` to be available in `wait_agent`. This is retrievable from `StateService::get_execution(execution_id).child_session_id`.

---

## Lifecycle Wiring

```
ExecutionRunner::with_config() (runner/core.rs:360)
  → agent_result_bus = Arc::new(AgentResultBus::new())    // NEW
  → InvokeBootstrap.agent_result_bus = Some(bus.clone())  // NEW

ExecutorBuilder (root and continuation executors)
  → .with_agent_result_bus(bus.clone())                   // NEW
  → registers WaitAgentTool + KillAgentTool in build_tool_registry root path

ContinuationArgs (core.rs:172)
  → agent_result_bus: Option<Arc<AgentResultBus>>         // NEW
  invoke_continuation builder wires it in

delegation/callback.rs::handle_delegation_success (new param):
  → agent_result_bus: &Arc<AgentResultBus>
  → call agent_result_bus.resolve(execution_id, agent_id, response)
    immediately after complete_execution, before writing DB callback

delegation/callback.rs::handle_delegation_failure (new param):
  → agent_result_bus.reject(execution_id, AgentWaitError::Crashed { error })
```

---

## File Layout

```
gateway/gateway-execution/src/agent_pool/
  mod.rs          — re-exports AgentResultBus, AgentResult, AgentWaitError
  result_bus.rs   — AgentResultBus implementation

gateway/gateway-execution/src/tools/
  wait_agent.rs
  kill_agent.rs

gateway/gateway-execution/src/delegation/callback.rs
  — add agent_result_bus parameter to handle_delegation_success
  — add agent_result_bus parameter to handle_delegation_failure
  — call resolve/reject before DB writes

gateway/gateway-execution/src/runner/core.rs
  — construct AgentResultBus in with_config()
  — add agent_result_bus field to ExecutionRunner
  — add to ContinuationArgs + invoke_continuation builder

gateway/gateway-execution/src/runner/invoke_bootstrap.rs
  — add agent_result_bus: Option<Arc<AgentResultBus>> to InvokeBootstrap
  — wire into ExecutorBuilder in finish_setup()
```

---

## What Stays Unchanged

- Delegation spawn flow — untouched
- `respond` tool — untouched
- `steer_agent` — untouched  
- `SessionContinuationReady` / continuation mechanism — uninvolved when root uses wait_agent
- `single_action_mode` — unchanged, naturally enforces the alternating pattern
- `pending_delegations` counter — still works correctly; decrements on each completion

---

## Relationship to Pattern 4 (Peer Messaging)

Pattern 3 and Pattern 4 use different primitives but share infrastructure:

| | Pattern 3 | Pattern 4 |
|---|---|---|
| Who resolves | system (delegation callback) | the target agent (reply tool) |
| Trigger | subagent calls `respond()` | target agent calls `message_agent(reply_to=...)` |
| Store | `AgentResultBus` (by execution_id) | `PeerMessageBus` → `ReplyStore` (by message_id) |
| Blocking tool | `wait_agent` | `message_agent(wait_for_reply=true)` |

Both use `tokio::sync::oneshot` + `tokio::time::timeout`. When implementing, the `AgentResultBus` and `ReplyStore` can share the same underlying `Mutex<HashMap<...>>` pattern.

---

## Out of Scope (intentional)

- `wait_agent()` with no arguments (global mailbox — wait for any agent)
- Priority ordering of which agent to wait for
- Agent result history (re-fetching a past result after it was consumed)
- Cross-session agent pool (handled by Pattern 4 Phase 2 federation)
