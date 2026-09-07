# Spec: Junkyard audit + CI laws + Wave 2 revision (Wave 7)

- **Status:** Implementing
- **Branch:** `op_clean_crap`

## Wave 2 revision — architectural decision

The original plan called for merging gateway-execution into agent-runtime
("3 homes → 1 crate"). After waves 0, 1, 3, 4, and 5, this is REVISED:

**The crate boundary between agent-runtime (generic engine) and
gateway-execution (product orchestration) is architecturally sound.**
Merging would expand agent-runtime's dependency surface to 21+ crates,
creating a god-crate — the exact anti-pattern this consolidation removes.

What WAS broken (context duplication, god files, stringly errors,
non-extensible hooks) is fixed. The crate count stays at 2 for execution:
runtime = engine, gateway-execution = product. The boundary is the
`build_execution_engine` function — one construction path, already typed.

## Wave 7 — audit + CI laws

### Audit checklist

- [x] Audit result: 15 files remain over 800 lines (largest: distillation
      3,056 → being decomposed in wave 6; spawn 2,655; invoke_bootstrap 2,623).
      The >4,000-line god files are ALL eliminated (executor.rs 4,442→52,
      intent_analysis.rs 2,950→deleted). Remaining large files are tracked
      for future decomposition; the 800-line CI law ratchets from here.
- [x] No struct exceeds 8 fields (ExecCtx is the one exception — it IS
      the context; documented).
- [x] No `Result<_, String>` in non-test, non-trait-impl production code.
- [x] Zero references to deleted types (ContinuationArgs, RunnerContinuationInvoker,
      RunnerDelegationInvoker, AgentExecutor, select_engine, RecallHook,
      BeforeToolCallHook, AfterToolCallHook, TransformContextHook, ToolCallDecision).
- [x] Golden traces replay green (behavior oracle holds).

### CI enforcement (clippy lints)

Add to gateway-execution's Cargo.toml `[lints]` or workspace lints:
- `clippy::too_many_arguments` = deny (with allow for ExecCtx construction)
- Ensure existing `-D warnings` in CI catches regressions

### Engram verification

- Re-index gateway tree; verify no function exceeds 50 lines in the
  execution path (spot-check top-centrality nodes).

### Final receipts

Record the net line-count delta for the entire branch op_clean_crap
vs. its merge-base with the previous shipped branch.
