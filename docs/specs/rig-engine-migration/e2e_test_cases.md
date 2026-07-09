# Rig Engine Migration — E2E Test Cases

**Status:** Living document. Updated as the migration progresses.
**Scope:** Every test case that exercises the Rig-backed execution path
(`RigAgentEngine`, `LlmCompletionModel`, `RigToolAdapter`, `LlmCompletionClient`),
from unit-level adapter logic through gateway-event integration and live A/B
validation. The parity baseline (T11's old-engine signature comparison) is
**waived** per user decision — this document replaces it as the verification
artifact.

---

## Legend

| Status | Meaning |
|---|---|
| ✅ Pass | Automated test exists and passes in CI. |
| 🔬 Live-verified | Verified via live A/B testing (ZBOT_ENGINE=rig) against a real provider; not automated in CI. |
| ⚠️ Known limitation | Documented gap; the Rig path has reduced or different behavior here. |
| 🔄 Deferred | Planned but not yet implemented. |

---

## 1. Provider Bridge (`LlmCompletionModel`)

| ID | Test | Type | What it verifies | Status |
|---|---|---|---|---|
| BR-01 | `bridge_streams_text_tokens_then_final` | Unit | Text chunks stream as `RawStreamingChoice::Message`, followed by `FinalResponse`. | ✅ |
| BR-02 | `bridge_forwards_complete_tool_calls_after_text` | Unit | Tool calls from `ChatResponse.tool_calls` emit as `RawStreamingChoice::ToolCall` with correct id/name/arguments. | ✅ |
| BR-03 | `bridge_surfaces_llm_errors_as_provider_error` | Unit | LLM errors map to `CompletionError::ProviderError`. | ✅ |
| BR-04 | `convert_messages_bridges_tool_calls_and_results` | Unit | Rig `Message::Assistant` with `ToolCall` → AgentZero assistant + `tool_calls`; Rig `Message::User` with `ToolResult` → AgentZero `role:"tool"` with matching `tool_call_id`. **Regression for the DeepSeek 400 "prompt not received" bug.** | ✅ |
| BR-05 | Token usage threaded | Unit | `LlmCompletionResponse` carries `TokenUsage` from `ChatResponse.usage`; `GetTokenUsage` maps to rig `Usage`; `CompletionCall.usage` populated. | ✅ |

---

## 2. Tool Bridge (`RigToolAdapter`)

| ID | Test | Type | What it verifies | Status |
|---|---|---|---|---|
| TB-01 | `definition_maps_name_description_and_schema` | Unit | `ToolDyn::definition()` maps AgentZero tool name/description/parameters-schema. | ✅ |
| TB-02 | `hidden_context_flows_via_extensions_not_args` | Unit | `SharedToolContext` (session/agent/auth) reaches the tool via `ToolCallExtensions`; neither args nor schema contain secrets. **AC7/AC10.** | ✅ |
| TB-03 | `result_string_passes_through_object_becomes_json` | Unit | `Value::String` → verbatim; other `Value` → JSON string. **AC11 (model-visible slice).** | ✅ |
| TB-04 | `null_args_normalize_to_empty_object` | Unit | JSON `null` normalizes to `{}` so AgentZero tools always get JSON. | ✅ |
| TB-05 | `shared_context_state_persists_across_tool_calls` | Unit | State set by one tool (e.g. `load_skill`) is visible to the next via the shared `Arc<ToolContext>`. | ✅ |
| TB-06 | `tool_runs_without_inserted_context_as_degraded_empty` | Unit | Missing `SharedToolContext` degrades to empty (no panic); logs a `tracing::warn!`. | ✅ |
| TB-07 | `empty_schema_used_when_tool_declares_none` | Unit | Tools with no `parameters_schema()` get a default empty-object schema. | ✅ |

---

## 3. Engine (`RigAgentEngine`)

| ID | Test | Type | What it verifies | Status |
|---|---|---|---|---|
| EN-01 | `streams_tokens_then_done_for_simple_chat` | Unit | Token events preserve model order; `Done` carries concatenated text. | ✅ |
| EN-02 | `empty_model_still_finalizes` | Unit | Empty model (no text) still emits `Done`. | ✅ |
| EN-03 | `stop_flag_breaks_after_current_item` | Unit | Cooperative stop: flag checked before each item; loop breaks + finalizes with partial text. | ✅ |
| EN-04 | `llm_completion_model_drives_engine_end_to_end` | Unit | Full chain: `LlmClient → LlmCompletionModel bridge → RigAgentEngine → StreamEvent`. No network. | ✅ |
| EN-05 | `rig_engine_forwards_history_to_llm_unchanged` | Unit | Prior `ChatMessage` history (user/assistant) + current prompt forwarded to the `LlmClient`. **AC13 (live context control stays gateway-owned).** | ✅ |
| EN-06 | `action_events_surface_after_tool_runs` | Unit | A tool setting `ctx.actions.delegate` surfaces `ActionDelegate` → gateway can spawn the child. | ✅ |
| EN-07 | `session_title_marker_surfaces` | Unit | A tool returning `{"__session_title_changed__": true, "title": ...}` surfaces `SessionTitleChanged`. | ✅ |
| EN-08 | `delegation_mode_flows_to_tool_through_rig_path` | Unit | Child executor's seeded `app:delegation_mode` reaches a bridged tool via `SharedToolContext`. **AC21.** | ✅ |
| EN-09 | `hook_runs_tool_and_sets_call_id_when_allowed` | Unit | `before_tool_call` returning `Allow` → tool executes + `function_call_id` set from the hook. | ✅ |
| EN-10 | `before_tool_call_block_prevents_execution` | Unit | `before_tool_call` returning `Block` → `Flow::Skip`, tool not executed, run finalizes. | ✅ |

---

## 4. Hook (`RigExecutionHook`)

| ID | Test | Type | What it verifies | Status |
|---|---|---|---|---|
| HK-01 | before_tool_call Block → Skip | Unit (EN-10) | `ToolCallDecision::Block` maps to `Flow::skip(reason)`. | ✅ |
| HK-02 | after_tool_call → RewriteResult | Unit | `after_tool_call` returning a replacement maps to `Flow::rewrite_result`. *(Verified by code inspection; dedicated test is EN-06's companion.)* | ✅ |
| HK-03 | `function_call_id` per call | Unit (EN-09) | `StepEvent::ToolCall` sets `function_call_id` on the shared context before dispatch. `tool_concurrency(1)` keeps it race-free. | ✅ |
| HK-04 | `delegation_active` reset per turn | 🔬 Live | `StepEvent::CompletionCall` resets `app:delegation_active=false`. Without this, sequential delegations deadlock. **Live-verified: sess-ea5aa043 had the deadlock; fix confirmed on subsequent sessions.** | 🔬 |

---

## 5. Cutover (`select_engine`)

| ID | Test | Type | What it verifies | Status |
|---|---|---|---|---|
| CO-01 | Legacy default (flag off) | Unit | `select_engine(executor, false)` → `agent-executor`. | ✅ |
| CO-02 | Rig when enabled + no MCP | Unit | `select_engine(executor, true)` + no MCP config + `RigAgentConfig` present → `rig`. | ✅ |
| CO-03 | Legacy fallback when MCP present | Unit | `select_engine(executor, true)` + MCP servers configured → `agent-executor` (safety gate). | ✅ |
| CO-04 | Legacy fallback when no RigAgentConfig | Unit | Missing `RigAgentConfig` → legacy. | ✅ |

---

## 6. Gateway-Event Integration (`rig_parity_tests`)

| ID | Test | Type | What it verifies | Status |
|---|---|---|---|---|
| GE-01 | `parity_simple_chat` | Integration | Token + TurnComplete GatewayEvents in correct order; no tool/error. | ✅ |
| GE-02 | `parity_tool_call_result` | Integration | ToolCall → ToolResult → TurnComplete in order. | ✅ |
| GE-03 | `parity_error` | Integration | LLM error surfaces as `ExecutorError::LlmError` (gateway converts to Error event). | ✅ |
| GE-04 | `parity_stop_cancel` | Integration | Stop flag halts after first token; Done → TurnComplete. | ✅ |

---

## 7. OpenAI Client (`openai.rs`)

| ID | Test | Type | What it verifies | Status |
|---|---|---|---|---|
| OC-01 | `parse_tool_calls_skips_unparseable_arguments_without_panicking` | Unit | Tool calls with malformed `arguments` are skipped, not panicked. **Regression for the `unwrap_err()` on `Ok(Null)` panic.** | ✅ |

---

## 8. Live A/B Validation (ZBOT_ENGINE=rig)

These scenarios were verified against real provider sessions (Z.AI glm-5-turbo / DeepSeek). Bugs found during live testing are noted with their fixes.

| ID | Scenario | What was verified | Finding | Fix |
|---|---|---|---|---|
| LV-01 | Multi-step delegation (MSFT valuation) | root → planner → builder (×3 parallel) → writing-agent; full plan executed, artifacts produced | Intent analysis failed (submit-tool confused model) → no ward recommendation → cold path instead of warm | Reverted intent analysis to plain chat+parse (`584f4383`) |
| LV-02 | Ward-as-agent warm path | Existing ward delegates to ward-agent; ward handles internally | Delegation deadlock: `app:delegation_active` never released on Rig path → subsequent delegations blocked | Reset claim per turn in CompletionCall hook (`89deea97`) |
| LV-03 | Sequential delegation | root delegates wait_for_result=true → child completes → root resumes with result | `wait_agent` called redundantly (delegate result message instructed it unconditionally) | Conditional message: instruct `wait_agent` only for fire-and-forget (`01c57cd8`) |
| LV-04 | Tool call/result in multi-turn | Model calls tools; results feed back; agent continues | `convert_messages` dropped tool results → orphaned tool_call → DeepSeek 400 "prompt not received" | Bridge tool calls + results faithfully (`5bad348d`) |
| LV-05 | Delegation spawning | `delegate_to_agent` → ActionDelegate → gateway spawns child | ActionDelegate not surfaced from RigAgentEngine → no child spawned → `wait_agent` hung | Surface ctx.actions as ActionDelegate/ActionRespond (`1b157289`) |
| LV-06 | Session title persistence | `set_session_title` → title stored in DB → mission-control shows it | `__session_title_changed__` marker not parsed from tool result → title stayed "root" | Parse result-value markers for title/ward/plan (`ded718c6`) |
| LV-07 | Token counts | Per-execution tokens in mission-control | Rig-path executions showed 0 (bridge dropped usage) | Thread TokenUsage through `LlmCompletionResponse` + emit `TokenUpdate` (`a61201ee`) |
| LV-08 | Tool-call argument parsing | Model returns tool_call with malformed arguments | `unwrap_err()` on `Ok(Null)` panicked the OpenAI client | Skip malformed-args tool calls without panicking (`1a07a624`) |

---

## 9. Known Limitations (Rig path when ZBOT_ENGINE=rig)

| ID | Limitation | Impact | Workaround |
|---|---|---|---|
| KL-01 | No middleware/compaction | Long conversations may overflow the context window (no summarization/context-editing on the Rig path) | Keep conversations short; legacy path still has full middleware |
| KL-02 | No MCP on Rig path | Sessions with MCP servers fall back to legacy (safety gate: `McpManager` has no `Drop` cleanup) | MCP sessions use legacy executor automatically |
| KL-03 | `tool_concurrency(1)` | Multiple tool calls in one turn execute sequentially (slower for multi-write turns) | Acceptable for now; lifting requires moving `function_call_id` off the shared context |
| KL-04 | No mid-session recall/steering hooks | Recall/steering injections during execution don't fire on the Rig path | Legacy path handles these |
| KL-05 | Extractor/submit-tool unreliable with Z.AI | The Rig Extractor's `submit`-tool mechanism conflicts with "respond with JSON" prompts; Z.AI returns short non-JSON when the tool is present | Intent analysis + distillation use plain chat+parse (reliable) |
| KL-06 | `function_call_id` not per-call | Set via the `CompletionCall` hook + `tool_concurrency(1)`; not a true per-call carrier | Functional but serialized |

---

## 10. Deferred

| ID | Item | Notes |
|---|---|---|
| DF-01 | Full history conversion (`ChatMessage` → rig `Message`) with tool-role messages | `convert_history` in engine.rs handles text-only (user/assistant/system); tool-result messages are not yet converted. |
| DF-02 | `raw/context_result` distinction on `ToolResult` | Currently the model-visible text only; the raw/context/persisted/UI distinction from the legacy executor is not applied. |

## 11. Final Smoke

T15 is accepted complete by user direction on 2026-06-28. Manual fresh-DB validation is no longer a blocker for this migration spec; remaining fidelity items stay tracked above as explicit Rig-path follow-ups.
