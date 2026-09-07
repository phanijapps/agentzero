# Plan: Engine hook framework

- **Spec:** [spec.md](spec.md)
- **Status:** Approved

## Current state on branch (context preservation — READ FIRST)

Branch `op_clean_crap`, work IN PROGRESS, uncommitted:

1. **DONE — `runtime/agent-runtime/src/engine/hooks.rs` fully rewritten**: `EngineHook` trait (async_trait, 4 defaulted methods), `HookSet` (ordered multi-slot, `add`/`with`, fan-out methods), `ToolDecision`, `RecallPacket`, `HookError`, plus unit tests. **All old type aliases are deleted** — everything referencing them is currently broken.
2. **DONE (mostly) — `runtime/agent-runtime/src/engine/config.rs`**: import line → `use super::hooks::{HookSet, ToolExecutionMode};`; struct fields `before_tool_call`/`after_tool_call`/`transform_context` replaced with `pub hooks: HookSet`; constructor sets `hooks: HookSet::new()`.
3. **KNOWN BROKEN — same file**: the `Debug` impl (~lines 195-205) still has `.field("before_tool_call", ...)` / `.field("after_tool_call", ...)` entries — compile error.

## Migration table (T1 — exact, verified by grep before the break)

| File | Change |
|---|---|
| `engine/config.rs` | Fix Debug impl: remove stale `.field` entries for removed hooks; add one `.field("hooks", &self.hooks.len())`. |
| `engine/prepared.rs` | Delete `recall: Option<(RecallHook, u32, HashSet<String>)>` + `set_recall_hook`. Add `pub recall_schedule: Option<RecallSchedule>` where `pub struct RecallSchedule { pub every_n_turns: u32, pub injected_keys: HashSet<String> }` (define in `engine/hooks.rs` or prepared.rs — pick one, export it). Keep a `set_recall_schedule` setter. The hook itself now lives in `config.hooks`. |
| `rig_adapter/context_inputs.rs` | `ContextInputs::new(recall_tuple, steering, transform_opt)` → `ContextInputs::new(hooks: Arc<HookSet>, schedule: Option<RecallSchedule>, steering)`. `apply()` calls `hooks.recall(...)` / `hooks.transform_context(...).await`. Update imports. |
| `rig_adapter/factory.rs` | `build_engine` currently passes `cfg.before_tool_call, cfg.after_tool_call` (~line 78) and `ContextInputs::new(prepared.recall, ..., cfg.transform_context)` — pass `Arc::new(cfg.hooks.clone())` (engine owns the Arc) + `prepared.recall_schedule`. |
| `rig_adapter/engine.rs` | Delete `before`/`after` fields, `with_tool_hooks` constructor, and the `new()` hook params. Engine holds `hooks: Arc<HookSet>`; pass the same Arc to `RigExecutionHook` and `ContextInputs`. |
| `rig_adapter/tool_hook.rs` | `RigExecutionHook { before, after }` → `{ hooks: Arc<HookSet> }`; dispatch becomes `hooks.before_tool(name, args).await` / `hooks.after_tool(...).await`; `ToolCallDecision` → `ToolDecision`. |
| `lib.rs` (agent-runtime) | Exports: remove `RecallHook`, `RecallHookResult`, `ToolCallDecision`, `BeforeToolCallHook`, `AfterToolCallHook`, `TransformContextHook` (check exact list in the `pub use engine::{...}` block); add `EngineHook`, `HookSet`, `ToolDecision`, `RecallPacket`, `HookError`, `RecallSchedule`. |

## Migration table (T2 — gateway)

| File | Change |
|---|---|
| `gateway-execution/src/invoke/executor.rs` ~1379-1420 | The `executor_config.before_tool_call = Some(closure)` (tool audit) and `after_tool_call = Some(closure)` (audit/result-context) become ONE struct, e.g. `ToolAuditHook` implementing `EngineHook` (defined beside the call site or in `invoke/`), registered via `executor_config.hooks.add(Arc::new(ToolAuditHook { ... }))`. Preserve the exact log lines and rewrite behavior. |
| `gateway-execution/src/runner/core.rs` `attach_mid_session_recall_hook` | The ~55-line boxed closure becomes `MidSessionRecallHook` implementing `EngineHook::recall` (holds the unified-recall machinery); the fn now does `executor.hooks.add(Arc::new(MidSessionRecallHook { ... })); executor.recall_schedule = Some(RecallSchedule { every_n_turns, injected_keys })`. Callers (`invoke_bootstrap.rs:1567`, `continuation_execution.rs:420`) keep the same signature. |
| tests in gateway if any reference old aliases | migrate to trait impls / HookSet. |

## Migration table (T3 — runtime tests)

| File | Change |
|---|---|
| `rig_adapter/factory/snapshot_tests.rs` ~149, ~228 | `prepared.config.transform_context = Some(closure)` → small struct or a generic closure-adapter impl (define `struct FnTransform(Fn...)` if convenient, but NOT exported as an alias type) added via `config.hooks.add`. |
| `rig_adapter/factory/live_context_tests.rs` ~136, ~176, ~326, ~341, ~453 | `prepared.set_recall_hook(...)` + transform closures → `MidRecallTestHook` impls returning `RecallPacket`; schedule via `set_recall_schedule`. `Ok(crate::RecallHookResult{..})` → `Ok(RecallPacket{..})`. |
| `rig_adapter/factory/result_tests.rs` ~206, ~239, ~248, ~273, ~352 | before/after closures → `EngineHook` impls; `ToolCallDecision` → `ToolDecision`. |
| any `engine/config.rs` tests touching removed fields | update. |

## Tasks

- **T1: Runtime migration** (table above) — compile `cargo check -p agent-runtime` clean, then `cargo test -p agent-runtime --locked` green.
- **T2: Gateway migration** — `cargo check -p gateway-execution`, then full gateway suite green.
- **T3: Workspace gates** — clippy `-D warnings`, fmt, `cargo test --workspace --locked` (tracked pre-existing exceptions only: saved-surfaces envelope, peer-diagnostics flake, adk-eval).
- **T4: Audit** — crate-wide grep proves AC2 (zero old names); commit as one: `refactor(engine): one EngineHook trait, ordered HookSet — no compat aliases`.

## Verification

Compile + suites as listed; behavior preservation argued from the migration table (same closures, same log lines, now behind trait impls). No e2e delta expected; run `full-mode/simple-qa` once on the rebuilt daemon as a smoke.

## Boy-scout rules

No `type FooHook = ...` aliases survive. No deprecated attributes. Old names deleted, not hidden. Anything discovered dead along the way (unused hook plumbing) gets deleted in the same commit.
