# Spec: Execution context consolidation (Wave 1)

- **Status:** Implementing
- **Branch:** `op_clean_crap`
- **Shape:** refactor (behavior-preserving), pure deletion

## Objective

One `ExecCtx` — the set of services an execution needs — built once, owned once, shared by `Arc`. Deletes the four restatements of that set: `ContinuationArgs` (30 fields), `RunnerContinuationInvoker` (25 field clones), `RunnerDelegationInvoker`, and the `bootstrap` field-copy dance. `invoke_continuation` becomes `invoke_continuation(ctx: &ExecCtx, session_id, root_agent_id)`.

## Acceptance Criteria

- [ ] AC1: `ExecCtx` is the single shared-services struct; `ExecutionRunner` holds `Arc<ExecCtx>` plus runner-only state (handles map, semaphore, bootstrap, control).
- [ ] AC2: Zero references to `ContinuationArgs`, `RunnerContinuationInvoker`, `RunnerDelegationInvoker` (crate-wide, tests included).
- [ ] AC3: The `ContinuationSpawner`/`DelegationSpawner` traits survive only if they still earn their keep as the watcher/dispatcher test seams; `SessionSpawner` (unused) is deleted.
- [ ] AC4: Late-binding behavior preserved: `set_model_registry` / integrations installed after construction remain visible to pre-captured contexts (ArcSwap stays in ExecCtx).
- [ ] AC5: Gates: gateway-execution suite, workspace check/clippy/fmt green (tracked pre-existing exceptions only).

## Boundaries

No behavior change. No new crates. The spawner traits' `StubSessionInvoker` (test-stubs) may simplify to stub the narrower trait only.
