# Pattern 4 — Peer Messaging Design

**Date:** 2026-05-11  
**Status:** Northstar — not for implementation yet  
**Context:** philschmid.de/subagent-patterns-2026, Pattern 4: "Teams — Agents Talk to Each Other"  
**Reviewed by:** 3-agent parallel codebase analysis (executor.rs, StateService API, ExecutionRunner lifecycle)

---

## Northstar Vision

z-Bot is not just a personal desktop assistant. The enterprise target is **multiple z-Bot daemons, each owning an enterprise function, running on separate machines** — coordinating autonomously to handle cross-functional workflows without human routing.

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│   HR z-Bot      │     │  Finance z-Bot  │     │   Legal z-Bot   │
│   (machine A)   │────▶│   (machine B)   │◀────│   (machine C)   │
└────────┬────────┘     └────────┬────────┘     └────────┬────────┘
         │                       │                        │
         └───────────────────────┼────────────────────────┘
                    Federation layer (Phase 2)
```

A workflow like "onboard a new hire" touches HR, IT, Finance, and Legal. In the northstar, the HR bot messages Finance and Legal directly — no human routing, no copy-paste between systems, no central orchestrator bottleneck.

**Why this unlocks real enterprise value:**

| Workflow | Without peer messaging | With peer messaging |
|---|---|---|
| New hire onboarding | Human routes HR → IT → Finance → Legal | HR bot coordinates directly, waits for confirmations |
| Budget approval | Human copy-pastes contract to Finance | Legal bot fires message to Finance bot mid-task |
| Incident response | Human escalates Engineering → Legal → Finance | Engineering bot pings Legal and Finance simultaneously |
| Contract renewal | Legal finishes, human notifies Finance | Legal bot delivers contract terms directly |

---

## Two-phase delivery

### Phase 1 — Intra-daemon peer messaging
Agents within the same z-Bot daemon can message each other by `execution_id`.
Useful for: multiple specialists within one function bot coordinating on a complex task.
This is the part with a concrete technical design (see below).

### Phase 2 — Cross-daemon federation
Function bots on separate machines discover each other by **role name** and exchange messages via a lightweight federation service that all daemons connect to on startup.
- Agents address peers by role (`"finance-bot"`, `"legal-bot"`), not raw execution ID
- `PeerMessageBus` routes locally if target is on-daemon, remotely via federation if not
- `list_agents()` queries the federation registry, not just session-local executions
- Federation service is self-hosted (local-first), no cloud dependency
- Tool interface (`message_agent`, `list_agents`) is identical in both phases — agents don't know if they're talking to a local or remote peer

Phase 2 is where the enterprise value fully unlocks. Phase 1 is prerequisite plumbing.

---

## Requirements

- Any agent can message any other agent — by `execution_id` (Phase 1) or role name (Phase 2)
- Both fire-and-forget and request-reply
- Delivery via steering injection (mid-turn, immediate) into target's SteeringQueue
- If target already done → return its stored result immediately; if not found → error
- Discovery: parent injects execution_ids into context + `list_agents()` tool for dynamic lookup
- Trigger: both orchestrator pre-wires AND agents initiate dynamically

---

## Key design decisions (research-validated)

### Reply goes through oneshot, not steering queue
A blocking `wait_for_reply` call suspends the caller's executor on a `tokio::time::timeout(secs, rx).await`.
The reply resolves the oneshot directly via the bus — it does not re-enter the caller's steering drain.
This is safe because:
- A's executor is a single Tokio task; when it yields at `rx.await`, other tasks run freely
- B's executor is a completely separate Tokio task — no shared `&mut` state, no global lock
- A's SteeringQueue receives messages via `UnboundedSender` (non-blocking) while A is blocked; they drain before A's next LLM turn
- No `std::sync::Mutex` or `RwLock` is held across the `.await` point

**Latency implication**: B won't see the steering-injected peer message until B's current LLM stream completes and the loop top re-drains. If B is mid-stream on a 30s LLM call, A's timeout budget starts burning immediately. Default `timeout_secs` must account for LLM latency — use 120s, not 30s.

### Timeout is the deadlock + crash mitigation
Mutual deadlock (A waits for B, B waits for A) and target crash (nobody resolves the oneshot)
are both handled by `tokio::time::timeout` on the oneshot await. No cycle detection needed.

### Bus is session-scoped, Arc-shared
`PeerMessageBus` lives in `ExecutionRunner` (same as `SteeringRegistry`) and is cloned into
every executor via `ExecutorBuilder`. This ensures in-flight reply channels survive root resumption
after delegation.

**Continuation gap**: `invoke_continuation` currently does NOT wire `SteeringRegistry` into the builder
(`ContinuationArgs` has no such field). `PeerMessageBus` has the same gap — `ContinuationArgs` must be
extended, otherwise root-agent continuation turns cannot use `message_agent`. This is the most
important use case (root orchestrator synthesizing after parallel delegations complete).

### Drop-guard for panic-safe cleanup
The outer `tokio::spawn` closures in `delegation/spawn.rs` have no `catch_unwind`. A panic in the
outer task bypasses `steering_registry.remove()`. The same would bypass `cancel_pending_for()`.
Use a `Drop`-based guard to guarantee cleanup in all cases including panic:

```rust
struct ExecutionCleanupGuard {
    execution_id: String,
    steering_registry: Arc<SteeringRegistry>,
    peer_bus: Arc<PeerMessageBus>,
}
impl Drop for ExecutionCleanupGuard {
    fn drop(&mut self) {
        self.steering_registry.remove(&self.execution_id);
        self.peer_bus.cancel_pending_for(&self.execution_id);
    }
}
```
Replace the manual `steering_registry.remove()` at `spawn.rs:796` with this guard.

### Deregistration cancels pending replies
When a target executor finishes or crashes (via the Drop guard above), `cancel_pending_for(execution_id)`
sends `Err(Cancelled)` to all oneshot channels waiting on that target, immediately unblocking callers.

### cancel_pending_for needs a reverse index
`pending_replies: DashMap<message_id, oneshot::Sender>` cannot efficiently find all pending replies
targeting a specific `execution_id`. `cancel_pending_for` must either:
- Scan all entries (acceptable for desktop — max ~10 concurrent agents), OR
- Maintain a secondary `DashMap<target_execution_id, Vec<message_id>>` in `ReplyStore`

The secondary index is preferred for correctness and avoids O(n) scans. See `reply_store.rs` below.

### Tool placement: gateway-execution/src/tools/
`StateService` is NOT accessible from `ToolContext`. There is no path from any tool's call context
to `StateService`. Both `message_agent` and `list_session_agents` must live in
`gateway/gateway-execution/src/tools/` and hold injected `Arc<StateService<DatabaseManager>>` —
the same pattern as `SteerAgentTool` holding `Arc<SteeringRegistry>`.

### context_id vs session_id naming
`ToolContext::session_id()` returns `self.conversation_id` (the legacy `"chat-{uuid}"` field),
NOT the database `sess-...` ID. Tools must call `ctx.get_state("session_id")` to get the real
session ID. Similarly, `execution_id` is NOT currently injected into context state — it must be
added to the `executor.rs:247` injection block alongside `"session_id"`.

---

## Core abstraction: `PeerMessageBus`

```rust
pub struct PeerMessageBus {
    steering: Arc<SteeringRegistry>,
    replies: Arc<ReplyStore>,
    state: Arc<StateService<DatabaseManager>>,
}

pub struct PeerReply {
    pub from_execution_id: String,
    pub content: String,
}

pub enum SendResult {
    Delivered { message_id: String },
    CompletedResult { content: String },
}

pub enum PeerError {
    TargetNotFound(String),
    Timeout,
    Cancelled,
    ReplyNotFound(String),
}
```

**Public API:**

```rust
// Fire-and-forget. Returns immediately.
// If target done: returns its stored result. If not found: error.
pub async fn send(&self, caller_id: &str, target_id: &str, message: &str)
    -> Result<SendResult, PeerError>;

// Send and block until reply or timeout.
pub async fn send_and_wait(&self, caller_id: &str, target_id: &str,
                            message: &str, timeout_secs: u32)
    -> Result<PeerReply, PeerError>;

// Resolve a pending reply channel (called by the replier's tool).
pub fn reply(&self, message_id: &str, from_id: &str, content: &str)
    -> Result<(), PeerError>;

// Cancel all reply channels waiting on this execution_id (called from Drop guard).
pub fn cancel_pending_for(&self, execution_id: &str);
```

---

## ReplyStore: dual-indexed pending reply tracking

```rust
// reply_store.rs
pub struct ReplyStore {
    // message_id → (target_execution_id, sender)
    by_message: DashMap<String, (String, oneshot::Sender<PeerReply>)>,
    // target_execution_id → [message_ids] waiting on it
    by_target: DashMap<String, Vec<String>>,
}

impl ReplyStore {
    pub fn insert(&self, message_id: String, target_id: String, tx: oneshot::Sender<PeerReply>) {
        self.by_target.entry(target_id.clone()).or_default().push(message_id.clone());
        self.by_message.insert(message_id, (target_id, tx));
    }

    pub fn resolve(&self, message_id: &str) -> Option<oneshot::Sender<PeerReply>> {
        self.by_message.remove(message_id).map(|(_, (target_id, tx))| {
            if let Some(mut ids) = self.by_target.get_mut(&target_id) {
                ids.retain(|id| id != message_id);
            }
            tx
        })
    }

    pub fn cancel_all_for_target(&self, target_id: &str) {
        if let Some((_, ids)) = self.by_target.remove(target_id) {
            for id in ids {
                if let Some((_, (_, tx))) = self.by_message.remove(&id) {
                    let _ = tx.send(Err(PeerError::Cancelled)); // oneshot::Sender<Result<PeerReply, PeerError>>
                }
            }
        }
    }
}
```

Note: `oneshot::Sender<PeerReply>` above should be `oneshot::Sender<Result<PeerReply, PeerError>>`
so `cancel_all_for_target` can send `Err(Cancelled)` without a separate error channel.

---

## Two new tools

Both tools live in `gateway/gateway-execution/src/tools/` (not `runtime/agent-tools/`).

### `message_agent`

Phase 1 target is an `execution_id`. Phase 2 target is a role name (`"finance-bot"`) or execution_id — bus resolves which.

```
message_agent(
  target:          String,           // required — execution_id (Phase 1) or role name (Phase 2)
  message:         String,           // required
  wait_for_reply:  bool    = false,
  reply_to:        String? = null,   // message_id to resolve (used by replier)
  timeout_secs:    u32     = 120     // only applies when wait_for_reply=true; 120s to absorb LLM latency
)
```

Tool struct holds:
- `Arc<PeerMessageBus>`
- Caller's `execution_id` obtained from `ctx.get_state("execution_id")` at call time

Returns (JSON):
```json
// fire-and-forget, delivered
{"delivered": true, "message_id": "msg-uuid", "target": "exec-abc"}

// fire-and-forget, target already completed
{"status": "completed", "result": "...", "from": "exec-abc"}

// wait_for_reply=true, reply received
{"reply": "...", "from": "exec-abc"}

// errors
{"error": "timeout"} | {"error": "target_not_found"} | {"error": "cancelled"}
```

Note on `message_id` in `Delivered` response: this is informational for the LLM's own context.
It is NOT a handle to any follow-up API. The replier sees it as `reply_id` in the steering
message and can pass it back via `reply_to`.

### `list_agents`

Phase 1: lists agents in the current session. Phase 2: queries the federation registry — returns all active function bots across all daemons.

```
list_agents()   // no args
```

Tool struct holds:
- `Arc<StateService<DatabaseManager>>`
- Session ID obtained from `ctx.get_state("session_id")` at call time (NOT `ctx.session_id()`)

Phase 1 returns all executions in current session via `StateService::list_executions(ExecutionFilter { session_id: Some(sess_id), ..Default::default() })`:

```json
[
  {
    "execution_id": "exec-abc",
    "agent_id": "research-agent",
    "status": "running",
    "task": "Research Java strengths for systems programming",
    "started_at": "2026-05-11T00:38:02Z"
  }
]
```

Notes:
- DB column is `id` (not `execution_id`) — serialize as `"execution_id"` in JSON
- `task` is nullable — omit or return `null` for root executions with no task
- Returns all delegation types (root, sequential, parallel) — filter by status if needed

---

## Steering message format (what target LLM sees)

Injected into target's SteeringQueue by `bus.send()`:

```
[Peer message from research-agent (exec-abc) | reply_id: msg-123]
Here are the Java findings I collected so far...
```

No reply expected:
```
[Peer message from research-agent (exec-abc)]
Here are the Java findings I collected so far...
```

Handled by existing steering drain — no executor changes required.

---

## Data flow

### Fire-and-forget

```
A: message_agent(exec_B, "here are my findings")
   → bus.send(exec_A, exec_B, msg)
   → B running?  → SteeringRegistry.steer(exec_B, "[Peer:exec_A] msg")
                 ← SendResult::Delivered { message_id }
   → B done?     → StateService lookup → SendResult::CompletedResult { content }
   → B not found → PeerError::TargetNotFound
```

### Request-reply

```
A: message_agent(exec_B, "what did you find?", wait_for_reply=true, timeout_secs=120)
   → bus.send_and_wait(exec_A, exec_B, msg, 120)
   → generates msg_id = "msg-123"
   → SteeringRegistry.steer(exec_B, "[Peer:exec_A|reply_id:msg-123] what did you find?")
   → reply_store.insert("msg-123", exec_B, tx)
   → tokio::time::timeout(120s, rx).await   ← A's executor blocks here
   → A's loop is suspended; A's SteeringQueue accepts messages non-blockingly during wait
   → A's SteeringQueue will drain BEFORE A's next LLM turn (after tool resolves)

B: currently mid-LLM-stream — will not see message until stream completes (could be seconds)
B: loop top drain → sees "[Peer:exec_A|reply_id:msg-123]..." before next LLM call
B: message_agent(exec_A, "found 3 sources", reply_to="msg-123")
   → bus.reply("msg-123", exec_B, "found 3 sources")
   → reply_store.resolve("msg-123") → tx.send(Ok(PeerReply { ... }))
   → A unblocks

A: tool returns {"reply": "found 3 sources", "from": "exec_B"}
```

### Target crash while A is waiting

```
B crashes → Drop guard fires
          → steering_registry.remove(exec_B)
          → peer_bus.cancel_pending_for(exec_B)   ← via ExecutionCleanupGuard::drop
          → reply_store.cancel_all_for_target(exec_B)
          → all waiting oneshots → Err(Cancelled)
          → A's tool returns {"error": "cancelled"}
```

---

## Context injection requirements (new)

Two new injections needed in `executor.rs` alongside the existing `"session_id"` injection at line ~247:

```rust
// Already exists:
executor_config = executor_config.with_initial_state("session_id", session_id.into());
// NEW — needed for message_agent to know caller's execution_id:
executor_config = executor_config.with_initial_state("execution_id", execution_id.into());
```

Tools must use:
- `ctx.get_state("session_id")` — NOT `ctx.session_id()` (that returns conversation_id)
- `ctx.get_state("execution_id")` — injected above

---

## Lifecycle wiring

```
ExecutionRunner::with_config() (runner/core.rs:360)
  → steering_registry = Arc::new(SteeringRegistry::new())   // already exists at line 400
  → reply_store = Arc::new(ReplyStore::new())               // NEW
  → peer_bus = Arc::new(PeerMessageBus::new(                // NEW
        steering_registry.clone(),
        reply_store.clone(),
        state_service.clone()))

ExecutorBuilder (root and continuation executors)
  → .with_steering_registry(registry.clone())   // already exists
  → .with_peer_message_bus(bus.clone())          // NEW

// ContinuationArgs MUST be extended:
ContinuationArgs {
    peer_bus: Option<Arc<PeerMessageBus>>,  // NEW — otherwise root-agent continuation turns can't message peers
    // ...existing fields...
}

// invoke_continuation builder (core.rs ~line 1153) MUST call:
builder.with_peer_message_bus(args.peer_bus.clone())  // NEW

On executor completion/crash — replace manual cleanup with Drop guard:
// In delegation/spawn.rs, replace:
//   steering_registry.remove(&execution_id);   // line 796
// With:
let _guard = ExecutionCleanupGuard {
    execution_id: execution_id.clone(),
    steering_registry: steering_registry.clone(),
    peer_bus: peer_bus.clone(),
};
// _guard's Drop fires at end of scope, covering Ok / Err / panic

Tool registration in invoke/executor.rs (alongside SteerAgentTool):
if let (Some(ref sr), Some(ref bus), Some(ref state)) =
    (self.steering_registry, self.peer_bus, self.state_service) {
    tool_registry.register(Arc::new(MessageAgentTool::new(bus.clone())));
    tool_registry.register(Arc::new(ListSessionAgentsTool::new(state.clone())));
}
```

---

## File layout

```
gateway/gateway-execution/src/peer_messaging/
  mod.rs               — re-exports
  bus.rs               — PeerMessageBus, SendResult, PeerReply, PeerError
  reply_store.rs       — ReplyStore (dual-indexed DashMap), ExecutionCleanupGuard

gateway/gateway-execution/src/tools/
  message_agent.rs
  list_session_agents.rs

runtime/agent-runtime/src/executor.rs
  — Add "execution_id" injection alongside existing "session_id" at ~line 247

gateway/gateway-execution/src/runner/
  core.rs              — construct bus in with_config(), extend ContinuationArgs,
                         wire bus into invoke_continuation builder
  continuation_watcher.rs — carry bus through RunnerContinuationInvoker if needed
```

---

## Extension points

| Future capability | Touches only |
|-------------------|-------------|
| Broadcast to all agents | `bus.broadcast()` — new method, new tool |
| Message audit / history | `bus.rs` — write to store before routing |
| ACL (siblings only) | `bus.send()` — check session membership |
| Cross-session messaging | `bus.rs` — add session_id routing |
| Typed messages | Add `MessageKind` enum to `PeerMessage` |

---

## Out of scope (intentional)

- Pub/sub or message filtering
- Persisting peer messages in conversation history
- Agent-to-agent spawning (already works: subagents can call `delegate_to_agent`)
- Cross-session messaging

---

## Design gaps closed by this review

| Gap | How closed |
|-----|-----------|
| `execution_id` unavailable in tool context | Inject `"execution_id"` in executor.rs alongside `"session_id"` |
| `ctx.session_id()` returns wrong value | Tools use `ctx.get_state("session_id")` explicitly |
| StateService unreachable from ToolContext | Both tools live in gateway-execution/src/tools/ with Arc injection |
| Continuation executors miss bus | Extend ContinuationArgs; wire in invoke_continuation builder |
| `cancel_pending_for` has no reverse index | ReplyStore dual-index: by_message + by_target |
| Panic bypasses cancel_pending_for | ExecutionCleanupGuard with Drop impl |
| 30s timeout too short for LLM latency | Default changed to 120s |
| message_id purpose unclear | Documented: informational only, not a follow-up API handle |
