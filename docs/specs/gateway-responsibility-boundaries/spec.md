# Spec: Gateway responsibility boundaries

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [plan.md](plan.md)
- **Constrained by:** gateway/AGENTS.md; gateway/gateway-execution/src/runner/AGENTS.md
- **Contract:** none (existing public interfaces preserved)
- **Shape:** service

## Objective

Gateway maintainers can reason about execution control and capability inspection through focused components with explicit dependencies. ExecutionRunner delegates live execution control; AppState delegates capability catalog construction and resource enrichment. These are the first bounded extractions in the responsibility-first gateway cleanup, not a claim that every responsibility of either facade has been decomposed.

## Boundaries

### Always do

- Preserve public method signatures, persisted state transitions, actor filtering, tool inventory, error strings, and current cancellation behavior.
- Preserve the session-stop work committed at baseline 789fa55a and all unrelated user changes.
- Give extracted services explicit dependencies; keep facade methods thin and the shared handle registry singular.

### Ask first

- Changes to public transport contracts, execution semantics, or new crate/dependency boundaries.

### Never do

- Pass AppState or ExecutionRunner into the extracted component as a service locator, or use Deref to disguise broad dependency access.
- Change database schemas, tool authorization, startup order, recovery, or terminal-answer persistence in this refactor.

## Testing Strategy

Goal-based behavior-preserving extraction: retain existing real-store and catalog tests, add focused behavior tests where the extracted component is otherwise untested, and run gateway/execution tests plus typecheck and clippy. No red stub is needed for preserved behavior. Review verifies dependency ownership rather than treating file length as the success criterion. Existing integration tests exercise the public facades; no UI or prompt change is included.

## Acceptance Criteria

- [x] AC1: Execution control has a focused owner with explicit handle-registry, delegation-registry and state-service dependencies; stop, iteration extension, pause, live resume, cancel, exact cancel, end-session and handle lookup delegate through it.
- [x] AC2: Persisted-subagent recovery remains in the execution orchestrator; shared handles, database-before-signal ordering for pause/live-resume/cancellation, existing stop-before-complete ordering for end-session, recursive exact cancellation and unrelated-execution isolation are preserved.
- [x] AC3: Capability catalog construction and resource metadata mapping live outside AppState; production runner preference, minimal-state fallback, actor filtering, resource deduplication and unavailable-provider reporting are preserved.
- [x] AC4: Public facade compatibility remains intact; extracted components have no parent-facade dependency or broad implicit access and no new dependency is introduced.
- [x] AC5: Focused tests, affected-crate typecheck/clippy and independent review pass; any environment limitations are recorded honestly.
- [x] AC6: Resource enrichment uses only list/summary metadata interfaces, never connector queries, capability/tool execution, or source mutation.

## Assumptions

- Technical: Rust/Tokio and existing shared Arc-backed registries are retained (gateway/Cargo.toml and runner/core.rs).
- Product: the user requests responsibility-first cleanup of god classes; this slice preserves behavior (conversation authorization, 2026-09-06).
- Process: full work-loop applies because this introduces internal component boundaries. Direct implementation authorization supplies scope approval; there is no queued spec for this new request (workspace.toml).
