# Plan: Fluid Ward Context Integration

- **Spec:** [`spec.md`](spec.md)
- **Status:** Implemented

## Approach

Carry one generic normalized template packet from ward activation through every
context boundary, then remove the old model-authored structure and fixed-path
skills. Verify with incompatible templates rather than role-specific mocks.

## Constraints

- Consume the shared loader/linter; do not parse YAML independently.
- No mandatory artifact names, UI, repair/approval, or compatibility code.

## Declined additions

- Tempted to define a typed role registry; declining because arbitrary template
  keys are the feature.
- Tempted to keep ward-designer as a fallback; declining because it would
  restore a second layout authority.

## Tasks

### T1: Intent and session state carry a generic ward template packet

**Depends on:** spec:ward-layout-configuration/T4

**Touches:** `gateway/gateway-execution/src/middleware/intent_analysis.rs`, `gateway/gateway-execution/src/runner/invoke_bootstrap.rs`, `gateway/gateway-execution/src/session_ctx/**`

**Tests:**

- Unit/integration tests prove `structure` is absent and the normalized generic
  typed-rule projection plus digest survives session persistence/reuse, while
  instruction-like unknown metadata, secrets, provider values, bodies, and host
  paths are excluded (AC 1–2, 4).

**Done when:** no runtime state invites a model to design ward directories.

### T2: Planning, delegation, and continuation consume only the injected packet

**Depends on:** T1

**Touches:** `gateway/gateway-execution/src/**`, `gateway/templates/skills/{spec-builder,plan-composer}/**`, `gateway/templates/agents/**`, `gateway/templates/shards/**`

**Tests:**

- Prompt/template tests use default and no-spec/no-plan templates, propagate the
  same digest through planning/delegation/continuation, re-resolve after drift,
  and return the shared bounded fail-closed nudge when re-resolution fails
  (AC 2–5).
- Absent-role tests prove spec creation writes nothing and returns
  `role_not_declared`, while planning updates only the ephemeral session plan.

**Done when:** both templates work without a concrete path or mandatory role in
active prompts/skills.

### T3: Legacy layout designers are removed and fluid-template E2E is green

**Depends on:** T2

**Touches:** `gateway/templates/skills/ward-designer/**`, `gateway/gateway-execution/src/delegation/**`, `runtime/agent-runtime/src/tools/delegate.rs`, `e2e/**`

**Tests:**

- Absence checks remove `ward-designer`, `ward_hygiene`, `memory-bank/*`, and
  fixed spec/plan paths; E2E completes ward use with a renamed minimal template
  (AC 3–6).

**Done when:** E2E, workspace gates, and absence checks pass.

## Rollout

- Ships with the configuration/linter spec as one clean break.

## Risks

- Prompt consumers may silently retain defaults; incompatible-template tests
  are the authoritative gate.

## Changelog

- 2026-07-19: Removed mandatory role and sandbox/approval scope; narrowed to
  generic template context and fixed-layout skill retirement.
