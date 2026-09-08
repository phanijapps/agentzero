# Specs

Feature specs for AgentZero. Each spec is the canonical artifact for its scope, requirements, and lifecycle state.

## Active (Implementing / Draft)

| Spec | Status | Summary |
| --- | --- | --- |
| [Execution Consolidation Waves](exec-consolidation-waves/spec.md) | Drafting | Multi-wave execution-layer cleanup: ExecCtx, crate consolidation, ToolSpec table, turn-loop decomposition, typed errors. |
| [Bootstrap Decomposition](bootstrap-decomposition/spec.md) | Implementing | Decomposes `invoke_bootstrap.rs` god-methods into phase functions. |
| [ExecCtx](exec-ctx/spec.md) | Implementing | One context type (`ExecCtx`) replaces ContinuationArgs, RunnerContinuationInvoker, and the builder context restatements. |
| [Golden Traces](golden-traces/spec.md) | Implementing | Recorded StreamEvent sequences for the three canonical flows, proving behavior preservation across rewrites. |
| [Session-stop Cancellation](session-stop-cancellation/spec.md) | Implementing | Cancels queued, active, and delegated request work without affecting another session. |
| [Spawn Decomposition](spawn-decomposition/spec.md) | Implementing | Decomposes `spawn_delegated_agent` (734-line god-function) into phases. |
| [Agent-driven Surfaces](agent-driven-surfaces/spec.md) | Draft | Surfaces that agents can present to the user during execution. |
| [Intent Ward Archetype Selection](intent-ward-archetype-selection/spec.md) | Draft | Ward archetype selection driven by intent analysis. |

## Recently Shipped (2026 Q3)

| Spec | Summary |
| --- | --- |
| [Rig-only Execution](rig-only-execution/spec.md) | Complete Rig cutover for root/subagent execution, MCP and skills; removes the legacy executor and all engine-selection fallback paths. 11 waves, all acceptance criteria met. |
| [Engine Hook Framework](engine-hook-framework/spec.md) | One `EngineHook` trait + ordered `HookSet` replaces 4 single-slot closure aliases. Extensible by design. |
| [Execution Errors](execution-errors/spec.md) | `ExecutionError` enum replaces 155 `Result<_, String>` sites across gateway-execution. |
| [Turn Loop](turn-loop/spec.md) | Engine loop decomposed: `TurnSignal` enum + pure event mapping. Golden traces prove zero behavior change. |
| [Junkyard Audit](junkyard-audit/spec.md) | Final branch receipts for the `op_clean_crap` consolidation. |

## Historical Record

All other shipped and archived specs are preserved as-is. They document the
requirements, decisions, and acceptance criteria for completed work. See the
individual `spec.md` files for details.

**Distillation extraction plans** (distillation-cleanup, distillation-extraction,
distillation-store-extraction) contain `plan.md` only — the work was executed
under `op_clean_crap` and the results are documented in the junkyard-audit spec.
