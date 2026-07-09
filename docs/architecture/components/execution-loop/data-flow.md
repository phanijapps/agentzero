# Execution Loop — Data Flow

Complete call sequence from UI message to response delivery. Each step includes the file path, function, DB operations, and events emitted.

---

## Stage 1: UI → WebSocket

**UI sends message** — `apps/ui/src/features/chat/mission-hooks.ts` (`sendMessage`, ~line 1189)

- Reads `session_id` from localStorage (`agentzero_web_session_id`)
- Calls `transport.executeAgent(agentId, conversationId, message, sessionId)`
- Sends `ClientMessage::Invoke` via WebSocket with `{ agent_id, conversation_id, message, session_id, mode }`
- UI sets phase: `"intent"` (new session) or `"executing"` (continuation)

**WebSocket handler** — `gateway/src/websocket/handler.rs` (`handle_client_message`, line 326)

- Extracts `exec_session_id` from `ClientMessage::Invoke`
- Pre-subscribes WS client to session if continuing (`subscriptions.subscribe_with_scope`)
- Creates `HookContext::web(session_id)` with conversation_id in metadata
- Creates `on_session_ready` callback for pre-subscribe before events fire
- Calls `runtime.invoke_with_hook_and_callback()`

**Runtime service** — `gateway/src/services/runtime.rs` (`invoke_with_hook_and_callback`, line 241)

- Builds `ExecutionConfig` with agent_id, conversation_id, vault_dir, hook_context, mode
- Delegates to `runner.invoke_with_callback(config, message, on_session_ready)`

---

## Stage 2: Session & Execution Setup

**`runner.invoke_with_callback()`** — `gateway/gateway-execution/src/runner.rs` (line 593)

### 2a. Get or Create Session

**`get_or_create_session()`** — `gateway/gateway-execution/src/lifecycle.rs` (line 39)

| Scenario | Action | DB Operations |
|----------|--------|---------------|
| New session (`session_id = None`) | Create session + root execution | INSERT `sessions`, INSERT `agent_executions` |
| Existing session | Reuse session, get root execution | SELECT `sessions`, SELECT `agent_executions` WHERE delegation_type='root' |
| Existing but completed | Reactivate session + execution | UPDATE `sessions` SET status='running', UPDATE `agent_executions` SET status='running' |

Returns `SessionSetup { session_id, execution_id, ward_id }`.

**Key**: For continuation, the **same root execution_id** is reused. This is how `has_intent_log(execution_id)` gates intent analysis.

### 2b. Start Execution

**`start_execution()`** — `lifecycle.rs` (line 115)

- DB: UPDATE `agent_executions` SET status='running'
- DB: INSERT `execution_logs` (category='session', message='Session started')

### 2c. Session-Ready Callback + Events

- Callback fires: `on_session_ready(session_id)` — WS client subscribes before events
- Event: `GatewayEvent::AgentStarted { agent_id, session_id, execution_id, conversation_id }`
- ServerMessage: `AgentStarted` → UI shows execution started

---

## Stage 3: Agent & History Loading

### 3a. Load Agent Configuration

**`AgentLoader`** — `gateway/gateway-execution/src/invoke/setup.rs`

- Loads `vault/agents/{agent_id}/agent.yaml` (or creates default for "root")
- Loads provider from `vault/providers/{provider_id}/provider.yaml`
- If orchestrator config exists in settings, overrides model/provider/temperature/thinking

### 3b. Load Conversation History

**`conversation_repo.get_session_conversation(session_id, 200)`** — `runner.rs` (line 684)

- DB: SELECT FROM `messages` WHERE session_id = ? ORDER BY created_at ASC LIMIT 200
- Converts to `Vec<ChatMessage>` (role: user/assistant/system/tool)
- **New session**: empty history
- **Continuation**: full conversation including prior tool calls and delegation results

---

## Stage 4: Memory Recall

**Skipped in fast mode.** For deep mode:

**`memory_recall.recall_with_graph()`** — `runner.rs` (line 699)

### 4a. Embed User Message
- Calls `embedding_client.embed(user_message)` → `Vec<f32>`

### 4b. Hybrid Search
- DB: SELECT FROM `memory_facts` — vector similarity + FTS5 full-text search
- Weights: vector (0.5) + BM25 (0.3) + high-confidence boost
- Returns top-5 relevant facts

### 4c. Graph Context (if available)
- DB: SELECT FROM `knowledge_graph_entities`, `knowledge_graph_relationships`
- Traverses entity neighborhood for relationship context

### 4d. Inject into History
- Formats as `## Recalled Context` system message
- Inserts at position 0 in history
- DB: INSERT `recall_log` (session_id, fact_key, recalled_at)

### 4e. Fallback
- If graph recall fails, falls back to basic recall (no graph traversal)
- Same embedding search, simpler format

---

## Stage 5: Intent Analysis

**Gate** — `runner.rs` (line 1563)

```
is_root && !already_analyzed && !is_fast_mode
```

- `already_analyzed = log_service.has_intent_log(execution_id)`
- DB: SELECT 1 FROM `execution_logs` WHERE session_id = {execution_id} AND category = 'intent'

### If Already Analyzed (Continuation)

- Event: `GatewayEvent::IntentAnalysisSkipped { session_id, execution_id }`
- ServerMessage: `IntentAnalysisSkipped` → UI skips to "Executing" phase
- No LLM call

### If Not Analyzed (First Turn)

**5a. Index Resources** — `middleware/intent_analysis.rs` (`index_resources`)
- Upserts skills, agents, wards into `memory_facts` (category: resource:skill, resource:agent, resource:ward)

**5b. Event**: `IntentAnalysisStarted` → UI shows "Analyzing intent..."

**5c. LLM Call** — `analyze_intent()` in `middleware/intent_analysis.rs`
- Builds prompt with: user message, available resources (from fact_store), recalled memory
- Model: agent's configured model, max_tokens: 2048, temperature: 0.7
- Returns `IntentAnalysis` JSON: primary_intent, hidden_intents, recommended_skills, recommended_agents, ward_recommendation, execution_strategy

**5d. Log & Emit**
- DB: INSERT `execution_logs` (category='intent', metadata={analysis JSON})
- Event: `IntentAnalysisComplete { ... all analysis fields ... }`
- ServerMessage: → UI displays intent sidebar

**5e. Inject into Agent** — `format_intent_injection()`
- Appends `## Task Analysis` section to `agent.instructions`
- Contains: original request, goal, hidden requirements, ward info, available resources, approach

---

## Stage 6: Prompt Compilation & Executor Build

### 6a. System Prompt Assembly

**`gateway-templates/src/lib.rs`** — `assemble_prompt()` (line 130)

Prompt is assembled from layered components:

| Layer | Source | Content |
|-------|--------|---------|
| SOUL.md | `vault/config/SOUL.md` (or embedded `soul_starter.md`) | Agent identity, personality |
| INSTRUCTIONS.md | `vault/config/INSTRUCTIONS.md` (or embedded `instructions_starter.md`) | Behavioral rules |
| OS.md | Auto-generated per platform | Platform-specific commands |
| Shards (required) | `vault/config/shards/` (user) or embedded | `first_turn_protocol`, `tooling_skills`, `memory_learning`, `planning_autonomy` |
| Shards (extra) | `vault/config/shards/*.md` (user only) | Any user-defined shards |
| Runtime info | Generated at runtime | Vault path, venv status |
| Intent injection | `format_intent_injection()` (Stage 5e) | Task analysis, ward, resources |

**Fast/chat mode** uses `assemble_chat_prompt()` (line 70): SOUL + chat_instructions + OS + 2 shards (chat_protocol, tooling_skills).

### 6b. Build Executor

**`ExecutorBuilder`** — `gateway/gateway-execution/src/invoke/executor.rs` (`build()`, line 161)

**LLM Client Stack** (line 298–327):
```
OpenAiClient → RetryingLlmClient (3 retries, 500ms backoff) → RateLimitedLlmClient (per-provider)
```

**Tool Registry**:

`ExecutorBuilder::build_tool_registry()` now filters first-party tools through
the actor capability policy. See
[subagent-capability-policy/overview.md](../subagent-capability-policy/overview.md)
for the exact policy.

| Actor kind | Tool profile |
|------------|--------------|
| Root | Orchestration/session tools: `delegate_to_agent`, `respond`, `memory`, `ward`, `update_plan`, `set_session_title`, `grep`, optional connector/query tools. No implementation file-write tools by default. |
| DelegatedExecutor | Implementation tools: `shell`, `write_file`, `edit_file`, `grep`, `ward`, `memory`, `load_skill`, `list_skills`, `list_mcps`, `respond`, `multimodal_analyze`. No orchestration/control tools. |
| DelegatedReviewer | Read-only review tools: `read`, `glob`, `grep`, `load_skill`, `list_skills`, `list_mcps`, `respond`, `multimodal_analyze`. No shell, file mutation, ward/memory mutation, or orchestration/control tools. |
| WardAgent | Full first-party tool profile for warm `ward:<name>` agents, including implementation and orchestration/control tools when backing services are wired. |

Optional tools: file tools (read/write/edit/glob), python, web_fetch, todos, introspection — controlled by tool settings.

**Middleware Pipeline** (line 396–422):
1. ContextEditingMiddleware — compacts long conversations (70% trigger deep, 80% fast)
2. Token counting
3. Rate limiting (per-provider)
4. Retry on transient errors

---

## Stage 7: Execution Loop (LLM ↔ Tools)

**`execute_with_tools_loop()`** — `runtime/agent-runtime/src/executor.rs` (line 483)

### Loop Structure

```
while iterations < max_turns:
    1. Call LLM (chat_stream) with history + system prompt
    2. Stream tokens → emit Token events
    3. If response has tool_calls:
       a. Emit ToolCall events
       b. Execute tools (sequential or parallel)
       c. Emit ToolResult events
       d. Append results to history
       e. Continue loop (next LLM turn)
    4. If response has no tool_calls:
       → Final response, break loop
    5. Check stop signals:
       - should_stop_after_respond (respond tool called)
       - stopped_for_delegation (delegation started)
       - handle.is_stop_requested (user cancelled)
```

### Iteration Budgets (line 586)

| Complexity | Max Turns | Soft Nudge At |
|------------|-----------|---------------|
| Small | 15 | ~12 |
| Medium | 30 | ~24 |
| Large | 50 | ~40 |
| XL | 100 | ~80 |

### DB Operations Per Iteration

- INSERT `messages` (role='assistant', content=response, tool_calls=JSON)
- INSERT `messages` (role='tool', per tool result)
- INSERT `execution_logs` (category='tool_call', per tool)
- INSERT `execution_logs` (category='tool_result', per tool)

### Events Emitted Per Iteration

- `Token { delta }` — each streamed text chunk
- `Thinking { content }` — reasoning content (if thinking enabled)
- `ToolCall { tool_call_id, tool, args }` — per tool call
- `ToolResult { tool_call_id, result, error }` — per tool result

---

## Stage 8: Delegation Flow

When the agent calls `delegate_to_agent`:

### 8a. Tool Execution

**`delegate_to_agent`** — `runtime/agent-runtime/src/tools/delegate.rs`

- Validates args (task ≤ 4000 chars, context ≤ 4000 chars, optional `mode`)
- Builds `DelegationRequest { agent_id, task, mode, context, max_iterations, skills }`
- Returns immediately (fire-and-forget)

### 8b. Delegation Handler (Background)

**`spawn_delegation_handler()`** — `runner.rs` (line 254)

- Per-session queue: only 1 delegation runs at a time per session
- Global semaphore: max 2 concurrent delegations across all sessions
- Acquires semaphore → spawns delegated agent

### 8c. Spawn Delegated Agent

**`spawn_delegated_agent()`** — `gateway/gateway-execution/src/delegation/spawn.rs`

- DB: INSERT `sessions` (child, with parent_session_id)
- DB: INSERT `agent_executions` (delegation_type='sequential')
- DB: UPDATE `sessions` SET pending_delegations += 1
- Event: `DelegationStarted { parent_agent_id, child_agent_id, task }`
- Classifies runtime actor kind:
  - `ward:<name>` → `WardAgent` (full first-party tools)
  - review-like ordinary tasks → `DelegatedReviewer` (read-only)
  - other ordinary tasks → `DelegatedExecutor` (implementation tools)
- Resolves delegation mode:
  - explicit `mode` wins
  - step-spec tasks → `step_executor`
  - ward setup/hygiene tasks → `ward_hygiene`
  - exact self-contained artifacts → `direct_artifact`
  - other builder work → `ward_backed_build`
- Prepends mode-specific executor rules and records
  `app:delegation_mode` in child initial state.
- Runs the same execution loop (Stage 7) for the child agent
- No intent analysis (subagent, not root)

### 8d. Delegation Completion

- DB: UPDATE `agent_executions` SET status='completed'
- DB: UPDATE `sessions` SET pending_delegations -= 1
- Event: `DelegationCompleted { child_agent_id, result }`

---

## Stage 9: Continuation After Delegation

**`spawn_continuation_handler()`** — `runner.rs` (line 477)

Triggered when: root execution completed AND all delegations done.

### 9a. Root Resumes

- Listens for `SessionContinuationReady { session_id, root_agent_id, root_execution_id }`
- Reloads full history (including delegation results as tool_result messages)
- Runs another LLM loop — agent sees delegation results and synthesizes final response
- Same execution loop (Stage 7)

### 9b. No More Delegations

When the continuation turn has no pending delegations → complete_execution (Stage 10).

---

## Stage 10: Completion

**`complete_execution()`** — `lifecycle.rs` (line 147)

- DB: UPDATE `agent_executions` SET status='completed', completed_at=NOW()
- DB: UPDATE `sessions` SET total_tokens_in=SUM(...), total_tokens_out=SUM(...)
- DB: INSERT `execution_logs` (category='session', message='Execution completed successfully')
- Event: `AgentCompleted { agent_id, session_id, execution_id, result }`

**`try_complete_session()`** — checks all executions are terminal:
- DB: UPDATE `sessions` SET status='completed'
- Triggers distillation (Stage 11)

---

## Stage 11: Distillation (Post-Session)

**`SessionDistiller.distill()`** — `gateway/gateway-execution/src/distillation.rs` (line 269)

### 11a. Load Transcript
- DB: SELECT FROM `messages` WHERE session_id=? (full conversation)

### 11b. LLM Extraction
- Prompt: session transcript → extract facts, entities, relationships, episode assessment
- Model: agent's model, temperature: 0.3, max_tokens: 4000

### 11c. Verify & Persist

| Data | DB Table | Operation |
|------|----------|-----------|
| Facts | `memory_facts` | UPSERT (key conflict → update confidence) |
| Fact embeddings | `embedding_cache` | INSERT (hash-based dedup) |
| Entities | `knowledge_graph_entities` | UPSERT (name conflict → update properties) |
| Relationships | `knowledge_graph_relationships` | INSERT |
| Episode | `session_episodes` | INSERT |
| Run record | `distillation_runs` | INSERT |

---

## Database Tables Summary

| Table | Read At | Written At |
|-------|---------|------------|
| `sessions` | Stage 2a, 10 | Stage 2a, 8c, 8d, 10 |
| `agent_executions` | Stage 2a | Stage 2a, 2b, 8c, 8d, 10 |
| `messages` | Stage 3b, 11a | Stage 7 (per iteration), 2a |
| `execution_logs` | Stage 5 (intent gate) | Stage 2b, 5d, 7, 10 |
| `memory_facts` | Stage 4b, 5a | Stage 5a, 11c |
| `embedding_cache` | Stage 4a | Stage 11c |
| `knowledge_graph_entities` | Stage 4c | Stage 11c |
| `knowledge_graph_relationships` | Stage 4c | Stage 11c |
| `recall_log` | — | Stage 4d |
| `session_episodes` | Stage 4c | Stage 11c |
| `distillation_runs` | — | Stage 11c |

---

## Event Sequence (New Session)

```
1. AgentStarted
2. IntentAnalysisStarted
3. IntentAnalysisComplete (with analysis data)
4. Token* (streaming response text)
5. ToolCall* (if tools used)
6. ToolResult* (tool outputs)
7. DelegationStarted* (if delegating)
8. DelegationCompleted* (when subagent finishes)
9. TurnComplete / Respond (final message)
10. AgentCompleted
```

## Event Sequence (Continuation)

```
1. AgentStarted
2. IntentAnalysisSkipped          ← KEY DIFFERENCE
3. Token* / ToolCall* / ToolResult*
4. DelegationStarted* / DelegationCompleted*
5. TurnComplete / Respond
6. AgentCompleted
```
