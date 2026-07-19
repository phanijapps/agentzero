# gateway-execution

Agent execution engine. Converts runtime stream events to gateway events, manages agent delegation with callbacks, handles continuation turns, and emits lifecycle events.

## Build & Test

```bash
cargo test -p gateway-execution    # 370 tests
```

For hot-path safety (runner.rs, delegation/spawn.rs), unit tests aren't enough on their own — run the Mode Full E2E after any change to `ExecutionRunner::spawn_execution_task`, `create_executor`, `invoke_continuation`, or the delegation spawn path:

```bash
cd e2e && ./scripts/boot-full-mode.sh simple-qa &
cd playwright && npx playwright test full-mode/simple-qa.full.spec.ts
```

Real zbotd + mock-llm + real UI build + scripted browser. Drift report confirms the LLM call shape hasn't changed.

## Key Types

| Type | Purpose |
|------|---------|
| `ExecutionRunner` | Main execution orchestrator |
| `ExecutionRunnerConfig` | Construction-time input bundle for `ExecutionRunner::with_config` |
| `ExecutionHandle` | Control handle (stop, pause, resume, cancel) |
| `ExecutionConfig` | Execution configuration |
| `GatewayFileSystem` | `FileSystemContext` implementation with ward support |
| `SessionSetup` | Session initialization parameters |
| `DelegationRegistry` | Tracks active delegations |
| `DelegationRequest` / `DelegationContext` | Delegation types |
| `BatchWriter` | Decouples DB writes from streaming (100ms flush, token coalescing) |

## Public API

| Function | Purpose |
|----------|---------|
| `ExecutionRunner::run()` | Run root agent execution |
| `spawn_delegated_agent()` | Spawn a subagent |
| `handle_delegation_success/failure()` | Delegation callbacks |
| `check_and_spawn_continuation()` | Spawn continuation turns |
| `convert_stream_event()` | StreamEvent → GatewayEvent |
| `get_or_create_session()` | Session lifecycle |
| `start_execution()` / `complete_execution()` | Execution lifecycle |

## File Structure

```
gateway-execution/src/
├── lib.rs                  # Public exports
├── config.rs               # ExecutionConfig, GatewayFileSystem
├── events.rs               # Event conversion (StreamEvent → GatewayEvent)
├── handle.rs               # ExecutionHandle
├── lifecycle.rs            # Session/execution lifecycle
├── continuation.rs         # Continuation spawning
├── distillation.rs         # SessionDistiller
├── archiver.rs             # SessionArchiver
├── artifacts.rs            # Artifact management
├── resource_provider.rs    # GatewayResourceProvider
├── composite_provider.rs   # CompositeResourceProvider
├── ward_artifact_indexer.rs
├── ward_wiki.rs
├── session_state.rs        # SessionState, SessionStateBuilder
├── runner/                 # Decomposed ExecutionRunner (see runner/AGENTS.md)
│   ├── core.rs             # ExecutionRunner struct + lifecycle methods
│   ├── session_invoker.rs  # Narrow traits for handler DI
│   ├── invoke_bootstrap.rs # Pre-execution two-phase setup
│   ├── execution_stream.rs # Per-execution event loop
│   ├── delegation_dispatcher.rs # Long-lived subagent queue
│   └── continuation_watcher.rs  # SessionContinuationReady listener
├── delegation/             # Agent delegation subsystem
│   ├── spawn.rs, context.rs, registry.rs, callback.rs
├── invoke/                 # Executor building + ingest adapter
├── ingest/                 # Ingest queue (chunker, extractor, etc.)
├── recall/                 # Memory recall (MemoryRecall, format_scored_items)
├── indexer/                # Ward artifact indexer
├── middleware/             # Working memory middleware
├── session_ctx/            # Session context helpers
└── sleep/                  # Execution sleep/wake
```

## Dependencies

Depends on most other gateway sub-crates: `gateway-events`, `gateway-services`, `gateway-connectors`, `gateway-templates`, `gateway-hooks`, `gateway-bus`. Also depends on `agent-runtime`, `agent-tools`, and `zbot-stores-sqlite`.

## Conventions

### Context structs over positional arguments

Functions that take ≥7 arguments must take a named-field context struct instead of a positional arg list. Rationale:

- Same-type fields (e.g. four consecutive `&str` ids, or three `Option<Arc<…>>`) can be silently swapped on a positional call — the bug only shows up at runtime in the wrong DB row / event / session.
- Adding a new dependency to a positional signature touches every call site, in order. Adding a new field to a context struct is one line per call site, order-independent.
- `#[allow(clippy::too_many_arguments)]` is a confession, not a solution. Don't use it; introduce a `FooContext` / `FooConfig` struct instead.

Established examples:
- Construction: `ExecutionRunner::with_config(ExecutionRunnerConfig { … })`
- Runner internals: `ExecutionRunner::spawn_execution_task(ExecutionTaskArgs { … })`, `ExecutionRunner::create_executor(CreateExecutorArgs { … })`, `spawn_with_notification(request, &SpawnNotificationDeps { … }, done_tx)`, `invoke_continuation(ContinuationArgs { … })`
- Lifecycle: `complete_execution(CompleteExecution { … })`, `crash_execution(CrashExecution { … })`, `stop_execution(StopExecution { … })`, `emit_delegation_completed(DelegationCompletedEvent { … })`
- Delegation: `spawn_execution_task(SpawnContext { … })`, `handle_execution_success(HandleExecutionSuccess { … })`, `handle_execution_failure(HandleExecutionFailure { … })`
- Batch writer: `BatchWrite::SessionMessage(SessionMessage { … })`

When extending any of these, add the new input as a named field to the relevant struct. Don't grow the positional signature.

### Invariant: zero `#[allow(clippy::too_many_arguments)]` inside this crate

The workspace clippy gate runs with `-D warnings`. Any time you catch yourself reaching for `#[allow(clippy::too_many_arguments)]` instead of adding a context struct, stop — the rule above was introduced to replace exactly that reflex. `runner.rs`, `lifecycle.rs`, and `delegation/spawn.rs` used to have nine such attributes combined; they now have zero, and a new one showing up here will fail CI.

### When to simplify vs. test

If a function has low coverage AND high cognitive complexity, simplify first (split, extract, introduce context structs) and add tests for the smaller resulting pieces. Don't test a god method as-is — you pin the wrong behaviour in place.

If a function has low coverage but IS small and single-purpose, just test it.

### Extract shared helpers over inline duplication

When two methods do the same thing with the same closure captures, the copy will eventually diverge in a way you can't see from either site. Prefer a module-private free function (or `pub(crate)` method) with a focused signature.

Established examples:
- `attach_mid_session_recall_hook(executor, memory_recall, agent_id, ward_id)` — replaced identical ~55-line `executor.set_recall_hook(...)` invocations in `create_executor` and `invoke_continuation`.
- `handle_tool_call_start` / `handle_tool_result` — extracted from the per-event match inside `spawn_execution_task`. The closure body used to be a 130-line switch; now it's a flat dispatcher. Both handlers take `&mut EventAccumulator` + `&EventHandlerDeps<'_>` so new events or new mutables don't grow the signature.
- `run_intent_analysis` + `emit_intent_fallback_complete` — replaced a 220-line nested-5-deep block inside `create_executor` with a single `Option<IntentOutcome>` return value. The caller is now one `if let Some(out) = ...` line.
- `prepend_continuation_recall` + `build_continuation_message` — extracted from `invoke_continuation`; both are async module-private helpers with clearly-documented preconditions for their no-op branches.

If the two sites have *similar* but not *identical* behaviour, resist the pressure to unify under an `Option`-flag argument — that often hides a behaviour change as a refactor. Check whether the divergence is intentional (e.g. the continuation task skips working-memory updates on purpose) before reaching for a shared helper.
