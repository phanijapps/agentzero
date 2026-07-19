# Execution Loop — File Reference

Every file involved in the execution loop with key functions and line references.

## UI Layer

| File | Function / Export | Purpose |
|------|-------------------|---------|
| `apps/ui/src/features/research-v2/useResearchSession.ts` | `useResearchSession()` | Research hook: snapshot hydrate + WS subscribe + dispatch |
| `apps/ui/src/features/research-v2/event-map.ts` | `mapGatewayEventToResearchAction()` | WS event → `ResearchAction` mapper |
| `apps/ui/src/features/research-v2/reducer.ts` | `reduceResearchSession()` | Reduces actions over per-turn state (turns, subagents, intent) |
| `apps/ui/src/features/research-v2/SessionTurnBlock.tsx` | `SessionTurnBlock` | Renders one user-prompt + assistant-response turn |
| `apps/ui/src/features/chat-v2/useQuickChat.ts` | `useQuickChat()` | Chat hook: WS subscribe + dispatch (no intent analysis UI) |
| `apps/ui/src/features/chat/mission-hooks.ts` | `useRecentSessions()`, `switchToSession()`, `timeAgo()` | Recent-sessions list + session switching helpers |
| `apps/ui/src/services/transport/http.ts` | `executeAgent()` | Sends invoke command via WebSocket |

## Gateway — WebSocket & Routing

| File | Function | Purpose |
|------|----------|---------|
| `gateway/src/websocket/handler.rs` | `handle_client_message()` line 326 | Routes ClientMessage::Invoke |
| | `gateway_event_to_server_message()` line 745 | Converts GatewayEvent → ServerMessage |
| `gateway/src/services/runtime.rs` | `invoke_with_hook_and_callback()` line 241 | Entry point from WS to runner |
| | `invoke()` line 164 | Simplified invoke (no hooks) |
| `gateway/gateway-ws-protocol/src/messages.rs` | `ServerMessage` enum line 111 | All WS messages (AgentStarted, Token, ToolCall, IntentAnalysis*, etc.) |
| | `ClientMessage` enum | Invoke, Stop, Pause, Resume, EndSession |

## Gateway — Execution Runner

| File | Function | Purpose |
|------|----------|---------|
| `gateway/gateway-execution/src/runner.rs` | `invoke_with_callback()` line 593 | Main orchestration: session → agent → history → recall → intent → executor → spawn |
| | `create_executor()` line 1474 | Intent analysis + executor build |
| | `spawn_execution_task()` line 848 | Async task: LLM loop + event processing |
| | `spawn_delegation_handler()` line 254 | Background: per-session delegation queue |
| | `spawn_continuation_handler()` line 477 | Background: resumes root after delegations |
| `gateway/gateway-execution/src/lifecycle.rs` | `get_or_create_session()` line 39 | Session reuse or creation |
| | `start_execution()` line 115 | QUEUED → RUNNING transition |
| | `complete_execution()` line 147 | RUNNING → COMPLETED transition |
| | `emit_agent_started()` | Publishes AgentStarted event |

## Gateway — Intent Analysis

| File | Function | Purpose |
|------|----------|---------|
| `gateway/gateway-execution/src/middleware/intent_analysis.rs` | `analyze_intent()` | LLM call for intent analysis |
| | `format_intent_injection()` | Formats analysis as prompt injection |
| | `index_resources()` | Upserts skills/agents/wards to memory_facts |

## Gateway — Delegation

| File | Function | Purpose |
|------|----------|---------|
| `gateway/gateway-execution/src/delegation/spawn.rs` | `spawn_delegated_agent()` | Creates child session + execution, runs subagent |
| `gateway/gateway-execution/src/delegation/callback.rs` | `handle_delegation_success()` | Completes delegation, sends result to parent |

## Gateway — Executor Build

| File | Function | Purpose |
|------|----------|---------|
| `gateway/gateway-execution/src/invoke/executor.rs` | `ExecutorBuilder.build()` line 161 | Assembles LLM client + tools + middleware |
| `gateway/gateway-execution/src/invoke/setup.rs` | `AgentLoader.load_or_create_root()` | Loads agent YAML + provider |

## Gateway — Prompt Templates

| File | Function | Purpose |
|------|----------|---------|
| `gateway/gateway-templates/src/lib.rs` | `assemble_prompt()` line 130 | Full prompt: SOUL + INSTRUCTIONS + OS + shards + runtime |
| | `assemble_chat_prompt()` line 70 | Chat mode: SOUL + chat_instructions + OS + 2 shards |
| | `load_shards()` line 243 | Loads required + user shards |

## Gateway — Events

| File | Type | Purpose |
|------|------|---------|
| `gateway/gateway-events/src/lib.rs` | `GatewayEvent` enum | All events: AgentStarted, Token, ToolCall, IntentAnalysis*, Delegation*, etc. |
| `gateway/gateway-bus/src/lib.rs` | `EventBus` | Pub/sub event routing |

## Gateway — Distillation

| File | Function | Purpose |
|------|----------|---------|
| `gateway/gateway-execution/src/distillation.rs` | `SessionDistiller.distill()` line 269 | Post-session fact/entity/episode extraction |

## Runtime — Executor

| File | Function | Purpose |
|------|----------|---------|
| `runtime/agent-runtime/src/executor.rs` | `execute_stream()` line 416 | Streaming entry point |
| | `execute_with_tools_loop()` line 483 | Main LLM ↔ tool iteration loop |
| | Max iterations check line 548 | Hard stop at max_turns |
| | Tool execution line 938 | Sequential/parallel tool dispatch |
| | Stop signals line 1019, 1043 | respond/delegation exit conditions |

## Runtime — Tools

| File | Tool Name | Purpose |
|------|-----------|---------|
| `runtime/agent-runtime/src/tools/respond.rs` | `respond` | Send final response to user |
| `runtime/agent-runtime/src/tools/delegate.rs` | `delegate_to_agent` | Spawn subagent with task |
| `runtime/agent-tools/src/tools/memory.rs` | `memory` | Save/recall facts |
| `runtime/agent-tools/src/tools/intent.rs` | `analyze_intent` | Intent analysis tool |
| `runtime/agent-tools/src/tools/skill.rs` | `load_skill` | Load skill into context |
| `runtime/agent-tools/src/tools/ward.rs` | `ward` | Ward management |
| `runtime/agent-tools/src/tools/shell.rs` | `shell` | Command execution |

## Services — State & Logging

| File | Service | Purpose |
|------|---------|---------|
| `services/execution-state/src/service.rs` | `StateService` | Session/execution CRUD, reactivation, continuation |
| `services/api-logs/src/service.rs` | `LogService` | Execution logs, `has_intent_log()` line 370 |
| `services/api-logs/src/repository.rs` | `has_category_log()` line 304 | SQL check for intent log existence |

## Services — Memory

| File | Service | Purpose |
|------|---------|---------|
| `services/memory/src/recall.rs` | `MemoryRecall` | Hybrid search: vector + FTS5 + graph |
| `services/memory/src/repository.rs` | `MemoryRepository` | CRUD for memory_facts |
| `services/knowledge-graph/src/storage.rs` | `GraphStorage` | Entity/relationship persistence |
