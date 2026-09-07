# Spec: Engine hook framework

- **Status:** Implementing
- **Branch:** `op_clean_crap`
- **Shape:** refactor (behavior-preserving) + framework extension point

## Objective

Replace the four single-slot closure hook aliases (`RecallHook`, `BeforeToolCallHook`, `AfterToolCallHook`, `TransformContextHook`) with **one** extensible [`EngineHook`] trait and an ordered multi-slot [`HookSet`]. Adding a hook point in the future must be a defaulted trait method — no call-site churn. Boy-scout rules: no compatibility aliases, old names deleted, every consumer migrated in the same change.

## Why

The current hooks are `Option<Arc<dyn Fn>>` singletons scattered across `ExecutorConfig` and a `(hook, u32, HashSet)` tuple on `PreparedExecution`. You cannot register two gates, cannot order them, cannot name a composite, and adding a new hook point means touching every construction site. The framework must be crisp enough to extend hooks, the agent builder, and the engine without archaeology.

## Contract

### New framework (already written in `runtime/agent-runtime/src/engine/hooks.rs`)

```rust
#[async_trait]
pub trait EngineHook: Send + Sync {
    async fn before_tool(&self, _name: &str, _args: &Value) -> ToolDecision { Allow }
    async fn after_tool(&self, _name, _args, _result, _ok) -> Option<String> { None }
    async fn transform_context(&self, _messages: &mut Vec<ChatMessage>) {}
    async fn recall(&self, _query, _injected) -> Result<RecallPacket, HookError> { Ok(default) }
}

pub struct HookSet { /* Vec<Arc<dyn EngineHook>> */ }
```

Fan-out semantics (unit-tested in hooks.rs):
- `before_tool`: first `Block` wins, else `Allow`
- `after_tool`: replacements chain, last `Some` wins
- `transform_context`: applied in registration order
- `recall`: first non-empty `RecallPacket` wins

Support types: `ToolDecision { Allow, Block { reason } }`, `RecallPacket { system_message, fact_keys }`, `HookError { message }`.

## Acceptance Criteria

- [ ] AC1: `EngineHook` is the only hook abstraction; `HookSet` composes multiple hooks with the documented ordering semantics; unit tests in `hooks.rs` pass.
- [ ] AC2: Zero references remain to `RecallHook`, `BeforeToolCallHook`, `AfterToolCallHook`, `TransformContextHook`, `ToolCallDecision`, `RecallHookResult` (crate-wide grep, tests included).
- [ ] AC3: `ExecutorConfig` carries `pub hooks: HookSet` (no per-hook `Option` fields); the engine, factory, and tool dispatch consume the set — not individual hooks.
- [ ] AC4: Mid-session recall schedule becomes an explicit `RecallSchedule { every_n_turns: u32, injected_keys: HashSet<String> }` on `PreparedExecution`; the recall *behavior* lives in a hook (registered by the gateway), the *schedule* is policy state.
- [ ] AC5: Existing behaviors preserved verbatim: gateway tool audit + result-context rewrite (executor.rs closures → one `EngineHook` impl), mid-session unified recall (boxed closure → one `EngineHook` impl), context transform in snapshot/live tests.
- [ ] AC6: Gates: `cargo test -p agent-runtime --locked`, `cargo test -p gateway-execution --features test-stubs --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo fmt --all` — all green.

## Boundaries

- No behavior change: same audit lines logged, same recall cadence, same transform results.
- No new crates; framework lives in `runtime/agent-runtime/src/engine/hooks.rs`.
- `ExecutorError` and error taxonomy untouched (HookError converts where recall errors previously returned `String`).
- Do not rebuild the builder or context structs here — hooks only. ExecCtx/consolidation waves are separate specs on this branch.

## Testing Strategy

TDD where semantics exist (hook ordering — already tested); migration verified by compilation plus the existing suites (snapshot/live/result tests in `rig_adapter/factory/`, gateway tool tests). No new e2e required.
