# Websockets Emitted and Consumed

## Architecture Overview

The WebSocket system has a two-layer event model:

1. **GatewayEvent** (backend internal) — defined in `gateway/gateway-events/src/lib.rs:24`, published via `EventBus` (tokio broadcast channels)
2. **ServerMessage** (wire protocol) — defined in `gateway/gateway-ws-protocol/src/messages.rs:111`, converted from GatewayEvent in `gateway/src/websocket/handler.rs:745` and sent to subscribed WebSocket clients

The UI has two primary modes consuming these events:
- **Research Mode** (`/research` route) — `ResearchPage` component via `useResearchSession()` hook (`features/research-v2/useResearchSession.ts`)
- **Chat Mode** (`/chat` route) — `QuickChat` component via `useQuickChat()` hook (`features/chat-v2/useQuickChat.ts`)

### Event Flow

```
Execution Engine
  └─> GatewayEvent → EventBus.publish()
       └─> Event Router (handler.rs:88)
            └─> gateway_event_to_server_message() (handler.rs:745)
                 └─> SubscriptionManager.route_event_scoped()
                      └─> WebSocket client receives ServerMessage
                           └─> Transport layer (http.ts:733)
                                ├─> Research Mode handlers (research-v2/event-map.ts → reducer.ts)
                                └─> Chat Mode handlers (chat-v2/event-map.ts → reducer.ts)
```

## List of Websocket Events

### GatewayEvent Emission Points (Backend)

These are the internal events emitted by the execution engine, lifecycle managers, and stream processors.

| # | Event (GatewayEvent) | Emission Source | File:Line | Context |
|---|---|---|---|---|
| 1 | `AgentStarted` | Stream event converter | `gateway-execution/src/events.rs:23` | Maps `StreamEvent::Metadata` from LLM response start |
| 2 | `AgentStarted` | Lifecycle helper | `gateway-execution/src/lifecycle.rs:439` | `emit_agent_started()` publishes on EventBus |
| 3 | `AgentStarted` | Placeholder invocation | `gateway/src/services/runtime.rs:320` | `invoke_placeholder()` for no-LLM invocations |
| 4 | `AgentCompleted` | Lifecycle completion | `gateway-execution/src/lifecycle.rs:223` | `complete_execution()` publishes with final result |
| 5 | `AgentCompleted` | Placeholder completion | `gateway/src/services/runtime.rs:340` | `invoke_placeholder()` publishes after 100ms delay |
| 6 | `AgentStopped` | Lifecycle stop | `gateway-execution/src/lifecycle.rs:416` | `stop_execution()` publishes on user-initiated cancel |
| 7 | `Token` | Stream event converter | `gateway-execution/src/events.rs:29` | Maps `StreamEvent::Token` (each streaming token delta) |
| 8 | `Thinking` | Stream event converter | `gateway-execution/src/events.rs:36` | Maps `StreamEvent::Reasoning` (extended thinking content) |
| 9 | `ToolCall` | Stream event converter | `gateway-execution/src/events.rs:45` | Maps `StreamEvent::ToolCallStart` (tool invocation begins) |
| 10 | `ToolResult` | Stream event converter | `gateway-execution/src/events.rs:56` | Maps `StreamEvent::ToolResult` (tool execution result) |
| 11 | `TurnComplete` | Stream event converter | `gateway-execution/src/events.rs:65` | Maps `StreamEvent::Done` (one LLM turn finished) |
| 12 | `Error` | Stream event converter | `gateway-execution/src/events.rs:72` | Maps `StreamEvent::Error` |
| 13 | `Error` | Lifecycle crash | `gateway-execution/src/lifecycle.rs:376` | `crash_execution()` publishes on execution crash |
| 14 | `Error` | Runner error helper | `gateway-execution/src/runner.rs:2022` | `emit_error()` in ExecutionRunner |
| 15 | `Respond` | Stream event converter | `gateway-execution/src/events.rs:84` | Maps `StreamEvent::ActionRespond` (agent uses respond tool) |
| 16 | `Respond` | CLI hook | `gateway-hooks/src/cli.rs:55` | `CliHook::respond()` publishes for CLI output |
| 17 | `Respond` | Cron hook | `gateway-hooks/src/cron.rs:68` | `CronHook::respond()` publishes for cron monitoring |
| 18 | `Respond` | Web hook | `gateway/src/hooks/web.rs:64` | `WebHook::respond()` publishes for web/WS adapters |
| 19 | `Heartbeat` | Stream event converter | `gateway-execution/src/events.rs:95` | Maps `StreamEvent::Heartbeat` (execution alive signal) |
| 20 | `WardChanged` | Stream event converter | `gateway-execution/src/events.rs:103` | Maps `StreamEvent::WardChanged` (agent switched ward) |
| 21 | `IterationsExtended` | Stream event converter | `gateway-execution/src/events.rs:111` | Maps `StreamEvent::IterationsExtended` (auto-extend) |
| 22 | `PlanUpdate` | Stream event converter | `gateway-execution/src/events.rs:119` | Maps `StreamEvent::ActionPlanUpdate` (plan tool used) |
| 23 | `SessionTitleChanged` | Stream event converter | `gateway-execution/src/events.rs:126` | Maps `StreamEvent::SessionTitleChanged` |
| 24 | `DelegationStarted` | Lifecycle helper | `gateway-execution/src/lifecycle.rs:461` | `emit_delegation_started()` helper |
| 25 | `DelegationStarted` | Runner delegation | `gateway-execution/src/runner.rs:1599` | ExecutionRunner publishes when spawning child agent |
| 26 | `DelegationCompleted` | Lifecycle helper | `gateway-execution/src/lifecycle.rs:487` | `emit_delegation_completed()` helper |
| 27 | `DelegationCompleted` | Delegation callback | `gateway-execution/src/delegation/callback.rs:288` | Published after delegation result processed |
| 28 | `MessageAdded` | Delegation callback | `gateway-execution/src/delegation/callback.rs:179` | Callback message added to parent conversation |
| 29 | `TokenUsage` | Stream processor | `gateway-execution/src/invoke/stream.rs:231` | Published after each LLM call with cumulative token counts |
| 30 | `SessionContinuationReady` | Continuation spawner | `gateway-execution/src/continuation.rs:52` | Published when continuation execution is created |
| 31 | `SessionContinuationReady` | Delegation completion | `gateway-execution/src/delegation/spawn.rs:699` | Last pending delegation completes successfully |
| 32 | `SessionContinuationReady` | Delegation failure | `gateway-execution/src/delegation/spawn.rs:808` | Last pending delegation fails (still triggers continuation) |
| 33 | `IntentAnalysisStarted` | Runner pre-execution | `gateway-execution/src/runner.rs:1766` | Published before intent analysis LLM call |
| 34 | `IntentAnalysisComplete` | Runner post-analysis | `gateway-execution/src/runner.rs:1804` | Published on successful intent analysis result |
| 35 | `IntentAnalysisComplete` | Runner fallback | `gateway-execution/src/runner.rs:1873` | Fallback when intent analysis LLM call fails |
| 36 | `IntentAnalysisComplete` | Runner fallback | `gateway-execution/src/runner.rs:1902` | Fallback when LLM client creation fails |
| 37 | `IntentAnalysisSkipped` | Runner skip check | `gateway-execution/src/runner.rs:1743` | Skipped when already analyzed (continuation turn) |

### ServerMessage Variants (Wire Protocol)

These are the messages actually sent to WebSocket clients. Most are converted from GatewayEvent; some are direct responses to ClientMessage.

| # | ServerMessage Variant | Source | File:Line | Trigger |
|---|---|---|---|---|
| 1 | `Connected` | Direct (connection) | `handler.rs:252` | New WebSocket connection established |
| 2 | `InvokeAccepted` | Direct (invoke response) | `handler.rs:407` | After `runtime.invoke_with_hook_and_callback()` succeeds |
| 3 | `Pong` | Direct (ping response) | `handler.rs:482` | In response to `ClientMessage::Ping` |
| 4 | `Subscribed` | Direct (subscribe response) | `handler.rs:665,678` | After successful subscription |
| 5 | `Unsubscribed` | Direct (unsubscribe response) | `handler.rs:736` | After unsubscription |
| 6 | `SubscriptionError` | Direct (subscribe error) | `handler.rs:688,701,714` | Subscription failures |
| 7 | `SessionPaused` | Direct (pause response) | `handler.rs:497` | After `runtime.pause()` succeeds |
| 8 | `SessionResumed` | Direct (resume response) | `handler.rs:522` | After `runtime.resume()` succeeds |
| 9 | `SessionCancelled` | Direct (cancel response) | `handler.rs:547` | After `runtime.cancel()` succeeds |
| 10 | `SessionEnded` | Direct (end response) | `handler.rs:572` | After `runtime.end_session()` succeeds |
| 11 | `AgentStarted` | From `GatewayEvent::AgentStarted` | `handler.rs:747` | Agent begins execution |
| 12 | `AgentCompleted` | From `GatewayEvent::AgentCompleted` | `handler.rs:760` | Agent finishes execution |
| 13 | `AgentStopped` | From `GatewayEvent::AgentStopped` | `handler.rs:775` | Agent stopped by user |
| 14 | `Token` | From `GatewayEvent::Token` | `handler.rs:790` | Streaming text delta (agent_id dropped) |
| 15 | `Thinking` | From `GatewayEvent::Thinking` | `handler.rs:803` | Thinking/reasoning content (agent_id dropped) |
| 16 | `ToolCall` | From `GatewayEvent::ToolCall` | `handler.rs:816` | Tool invocation (tool_id→tool_call_id, tool_name→tool) |
| 17 | `ToolResult` | From `GatewayEvent::ToolResult` | `handler.rs:833` | Tool result (tool_id→tool_call_id) |
| 18 | `TurnComplete` | From `GatewayEvent::TurnComplete` | `handler.rs:850` | One LLM turn done (message→final_message) |
| 19 | `TurnComplete` | From `GatewayEvent::Respond` | `handler.rs:908` | Respond mapped to TurnComplete with final_message |
| 20 | `Error` | From `GatewayEvent::Error` | `handler.rs:863` | Adds code: "execution_error" |
| 21 | `Error` | Direct (various) | `handler.rs:416,438,469,505,530,555,580` | Error responses to failed client actions |
| 22 | `Iteration` | From `GatewayEvent::IterationUpdate` | `handler.rs:877` | Progress iteration (variant renamed) |
| 23 | `ContinuationPrompt` | From `GatewayEvent::ContinuationPrompt` | `handler.rs:892` | Max iterations reached (agent_id dropped) |
| 24 | `DelegationStarted` | From `GatewayEvent::DelegationStarted` | `handler.rs:923` | Subagent delegation started |
| 25 | `DelegationCompleted` | From `GatewayEvent::DelegationCompleted` | `handler.rs:945` | Subagent delegation completed |
| 26 | `Heartbeat` | From `GatewayEvent::Heartbeat` | `handler.rs:967` | Execution alive signal |
| 27 | `MessageAdded` | From `GatewayEvent::MessageAdded` | `handler.rs:982` | New message in conversation |
| 28 | `TokenUsage` | From `GatewayEvent::TokenUsage` | `handler.rs:999` | Cumulative token counts |
| 29 | `WardChanged` | From `GatewayEvent::WardChanged` | `handler.rs:1016` | Agent switched ward |
| 30 | `IterationsExtended` | From `GatewayEvent::IterationsExtended` | `handler.rs:1028` | Auto-extended iterations |
| 31 | `PlanUpdate` | From `GatewayEvent::PlanUpdate` | `handler.rs:1046` | Plan updated via tool |
| 32 | `IntentAnalysisStarted` | From `GatewayEvent::IntentAnalysisStarted` | `handler.rs:1062` | Intent analysis begins |
| 33 | `IntentAnalysisComplete` | From `GatewayEvent::IntentAnalysisComplete` | `handler.rs:1072` | Intent analysis result |
| 34 | `IntentAnalysisSkipped` | From `GatewayEvent::IntentAnalysisSkipped` | `handler.rs:1103` | Intent analysis skipped |
| 35 | `SessionTitleChanged` | From `GatewayEvent::SessionTitleChanged` | `handler.rs:1094` | Session title changed |

### Not Sent to Client

| GatewayEvent | Reason |
|---|---|
| `SessionContinuationReady` | Internal only — returns `None` in conversion (`handler.rs:979`) |
| `IterationUpdate` | Defined but never emitted (no construction site found) |
| `ContinuationPrompt` | Defined but never emitted (no construction site found) |

## List of WebSocket Events Consumed by UI

### Research Mode (`/research` route) — `useResearchSession()` hook

Files: `apps/ui/src/features/research-v2/event-map.ts`, `apps/ui/src/features/research-v2/reducer.ts`

The hook subscribes to the WS stream and pipes each `ConversationEvent` through
`mapGatewayEventToResearchAction()` (event-map) into a `ResearchAction` that the
reducer applies to `ResearchSessionState` (turns, subagents, intent flags,
status). Pill events are mapped separately via `mapGatewayEventToPillEvent()`
and fed to the shared `useStatusPill` aggregator (`features/shared/statusPill`).

Event coverage at a glance: `agent_started`, `agent_completed`, `agent_stopped`,
`delegation_started`, `delegation_completed`, `ward_changed`, `thinking`,
`tool_call`, `tool_result`, `token`, `respond`, `turn_complete`,
`session_title_changed`, `intent_analysis_started`,
`intent_analysis_complete`, `intent_analysis_skipped`, `plan_update`,
`invoke_accepted` / `session_initialized` (session-bound), `error`. See
`event-map.ts` for the full case list and `reducer.ts` for the action handlers.

### Chat Mode (`/chat` route) — `useQuickChat()` hook

Files: `apps/ui/src/features/chat-v2/event-map.ts`, `apps/ui/src/features/chat-v2/reducer.ts`

Same shape as Research Mode: WS events pass through
`mapGatewayEventToQuickChatAction()` and get reduced by `reduceQuickChat()` into
`QuickChatState` (assistant message stream, inline activity chips). Pill state
is driven by `mapGatewayEventToPillEvent()` feeding the shared `useStatusPill`
aggregator. Token deltas, respond events, ward changes, session-init events,
and tool_call dispatches (`delegate_to_agent`, `load_skill`, `memory`) all map
to actions consumed by the reducer.

### Past-turn Replay (Research)

Past sessions in `/research` are rendered as a chronological list of
`SessionTurnBlock` components, hydrated from the `/api/sessions/:id/state` +
`/api/logs/sessions` + `/api/sessions/:id/messages` snapshot built by
`features/research-v2/session-snapshot.ts` and dispatched via the reducer's
`HYDRATE` action. (The previous `apps/ui/src/components/SessionChatViewer.tsx`
slide-out was deleted in PR #112.)

### Transport Layer (Internal Routing)

File: `apps/ui/src/services/transport/http.ts`

| Event Type | Purpose |
|---|---|
| `subscribed` | Confirms subscription, records sequence and root_execution_ids |
| `unsubscribed` | Logs unsubscription |
| `subscription_error` | Routes error to `onError` callbacks |
| `pong` | Heartbeat response, resets ping timer |
| `heartbeat` | Resets pong timer, sets `hasActiveExecution=true` |
| `agent_started` | Sets `hasActiveExecution=true` (enables unlimited reconnects) |
| `agent_completed` | Sets `hasActiveExecution=false` |
| `agent_stopped` | Sets `hasActiveExecution=false` |
| `turn_complete` | Sets `hasActiveExecution=false` |
| `invoke_accepted` | Routes to conversation subscribers |
| `stats_update` | Routed to `globalEventCallbacks` |
| `session_notification` | Routed to `globalEventCallbacks` |

### Events NOT Consumed by UI

These ServerMessage types are defined in the protocol but not currently consumed by any UI handler:

| Event Type | Status |
|---|---|
| `iteration` | Defined, never emitted, not consumed |
| `continuation_prompt` | Defined, never emitted, not consumed |
| `message_added` | Emitted but not consumed (delegation callback only) |
| `token_usage` | Emitted but not consumed (tokens tracked inline from `token` events) |
| `iterations_extended` | Emitted but not consumed |
| `plan_update` | Emitted but not consumed (plans tracked via `tool_call` for `update_plan` tool) |
| `connected` | Not consumed (connection state tracked via `onConnectionStateChange`) |
| `session_paused` | Emitted but not consumed |
| `session_resumed` | Emitted but not consumed |
| `session_cancelled` | Emitted but not consumed |
| `session_ended` | Emitted but not consumed |

## Additional Info

### Field Transformations: GatewayEvent → ServerMessage

| GatewayEvent Field | ServerMessage Field | Notes |
|---|---|---|
| `agent_id` (on Token, Thinking, ToolCall, ToolResult, ContinuationPrompt) | _dropped_ | Agent ID not included in wire protocol for streaming events |
| `tool_id` | `tool_call_id` | Renamed for consistency |
| `tool_name` | `tool` | Shortened |
| `message` (TurnComplete) | `final_message` | Clarified naming |
| _(no code)_ | `code: "execution_error"` | Added during Error conversion |
| `IterationUpdate` variant | `Iteration` variant | Variant renamed |
| `Respond` | Mapped to `TurnComplete` | `final_message: Some(message)` |

### Subscription Scopes

| Scope | Events Delivered | Use Case |
|---|---|---|
| `"all"` | Everything (default, backward compatible) | Full debug view |
| `"session"` | Root execution events + delegation lifecycle only | Clean Research Mode chat view |
| `"execution:{id}"` | All events for a specific execution | Subagent detail view |

### Event Routing Paths

1. **Path A: EventBus → Event Router → ServerMessage** — All execution events flow through `EventBus.publish()` → event router task (handler.rs:88) → `gateway_event_to_server_message()` → `SubscriptionManager.route_event_scoped()` → WebSocket clients
2. **Path B: Direct ServerMessage** — Protocol responses (Pong, Connected, InvokeAccepted, Subscribed, SessionPaused, etc.) sent directly from `handle_client_message()` via `session.send()`
3. **Path C: WebHook dual send** — `WebHook::respond()` both publishes `GatewayEvent::Respond` to EventBus AND directly sends `ServerMessage::TurnComplete` to the WebSocket session (parallel delivery for immediate response)

### Client Messages (UI → Server)

| ClientMessage | Description | Used By |
|---|---|---|
| `Invoke` | Start/continue conversation (agent_id, message, session_id?, mode?) | Both modes |
| `Subscribe` | Subscribe to session events (conversation_id, scope) | Both modes |
| `Unsubscribe` | Unsubscribe from session events | Both modes |
| `Stop` | Stop current execution | Research Mode |
| `Continue` | Continue after iteration limit | Research Mode |
| `Pause` | Pause running session | Ops Dashboard |
| `Resume` | Resume paused/crashed session | Ops Dashboard |
| `Cancel` | Cancel session | Ops Dashboard |
| `EndSession` | End session (mark completed) | Both modes (on /new) |
| `Ping` | Keepalive | Transport layer |

### Key Implementation Files

| File | Purpose |
|---|---|
| `gateway/gateway-events/src/lib.rs` | GatewayEvent enum definition (26 variants) |
| `gateway/gateway-events/src/broadcast.rs` | EventBus (tokio broadcast channels) |
| `gateway/gateway-ws-protocol/src/messages.rs` | ServerMessage + ClientMessage enums |
| `gateway/src/websocket/handler.rs` | Event router, conversion, client message handling |
| `gateway/src/websocket/subscriptions.rs` | SubscriptionManager (scope-based routing) |
| `gateway/gateway-execution/src/events.rs` | StreamEvent → GatewayEvent converter |
| `gateway/gateway-execution/src/lifecycle.rs` | Execution lifecycle events |
| `gateway/gateway-execution/src/runner.rs` | Intent analysis events, delegation events |
| `apps/ui/src/services/transport/http.ts` | WebSocket client, message parsing, event routing |
| `apps/ui/src/services/transport/types.ts` | Transport type definitions |
| `apps/ui/src/features/research-v2/event-map.ts` | Research Mode: WS event → reducer action mapper |
| `apps/ui/src/features/research-v2/reducer.ts` | Research Mode: state reducer (turns, subagents, intent) |
| `apps/ui/src/features/research-v2/useResearchSession.ts` | Research Mode hook: subscribe + dispatch |
| `apps/ui/src/features/chat-v2/event-map.ts` | Chat Mode: WS event → reducer action mapper |
| `apps/ui/src/features/chat-v2/reducer.ts` | Chat Mode: state reducer (assistant stream, chips) |
| `apps/ui/src/features/chat-v2/useQuickChat.ts` | Chat Mode hook: subscribe + dispatch |
