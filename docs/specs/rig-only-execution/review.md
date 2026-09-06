# Rig-only execution verification

## Current implementation round

The user's explicit branch authority covers cutover implementation, dependency
selection and reviewed plan adjustments. The amendment restored an Ask first
boundary only for external publication/deployment and unrelated product changes;
both adversarial and secure-design re-reviews returned Clean. The code-mode
baseline was sealed under run `fb320581-90fd-4785-8474-bd587f006a21`.

T1 repaired the provider-task cancellation leak and removed the duplicate
170-line SSE client. These bounded repairs independently reviewed Clean.
Native Rig/rmcp lifecycle probes and dependency checks are recorded in
[parity-matrix.md](parity-matrix.md). Production routing remains unchanged until
the required behavior is ported; no cutover AC is marked complete.

The remaining sections are the historical planning record, not restrictions on
the subsequent delegated implementation authorization.

## Implementation handoff — subsequent user approval

The user subsequently authorized incremental implementation and the small
tracking repairs needed to begin. The statements below and in the approved
plan about what "this planning turn" authorized describe that earlier turn,
not a restriction on the later implementation approval. The approved task
strategy remains sealed and unchanged.

The current spec is Implementing, the plan is Approved, and the code-mode run
is at T1. Two new pinned-Rig probes and 34 existing adapter tests pass; the
focused Clippy, formatting and diff checks pass. See [parity-matrix.md](parity-matrix.md)
for exact evidence and open gaps. No full T1, parity or cutover completion is
claimed, and production routing has not changed.

## Scope and baseline

- Branch: `feat/rig-only-execution`, created from `fix/surface-time-window-attribution` at `789fa55a`.
- Existing uncommitted gateway responsibility extraction, watch-script setting, workspace and doc edits were preserved. This turn changes only the new planning artifacts and spec index; it does not implement Rig cutover.
- Base freshness check: passed against `origin/main` before authoring.
- User clarified Rig (not Zig) and explicitly requires no legacy executor for MCP/skills/root/subagent execution. Scope and strategy remain Draft for approval.
- `project-knowledge not requested`; Draft authoring/review performs no knowledge capture.

## Mechanical checks

- Spec metadata lint against `789fa55a`: passed, with unrelated historical warn-only links.
- Task-structure check: 11 tasks, valid backward dependency edges, explicit verification modes, Tests before Approach, Done when on every task; initial 12 ACs expanded to 13 after security review.
- Daemon/CLI Cargo package names and the existing Mode Full harness paths verified. Runtime feasibility probes and full code/E2E gates are future T1/T11 work, not claimed as executed here.
- `git diff --check`: passed.
- Work-loop spec-plan state initialized; pending plan review is the expected pre-review condition.

## Independent review

- Spec/plan adversarial review: added distinct streamable-http transport matrix, connector_resource/connector_invoke negative tests, linked inherited behavior contracts, and active README retirement coverage. All four first-pass findings applied; re-review returned Clean — ready to commit. Archived/historical contracts are explicitly scoped so obsolete storage/engine decisions are not reinstated.
- Spec-stage security review: required explicit effectful-tool confinement AC. Applied AC13 and T4/T6 negative tests; re-review returned Clean — ready to commit. This preserves existing controls and does not promise a new remote-MCP sandbox.

## Disposition

No migration code or runtime/deployment side effect is authorized by this turn. Hard-cutover feasibility is checked before implementation; Draft status does not claim the pinned Rig version already supplies every required hook. No substantive reviewer finding may be left unresolved while this draft is described as review-clean.

All first-pass findings are resolved and both reviewers are clean. The reviewed Draft spec/plan awaits human scope and strategy approval; no approval, implementation, commit, push, PR or deployment is claimed.
