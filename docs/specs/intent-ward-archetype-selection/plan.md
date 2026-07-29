# Plan: Intent Ward Archetype Selection

- **Spec:** [`spec.md`](spec.md)
- **Status:** Drafting

## Approach

First change the structured intent contract and fixtures from free-form
structure to a closed optional archetype. Then add a small resolver for action,
override, recommendation, and fallback precedence. Bind its typed result into
existing root tool state and consume it once in the `ward` tool creation path.
Verify reuse and clean-start behavior across the middleware/tool/service seam.

## Constraints

- Depends on the registry and create contract in
  [`ward-archetype-registry-and-creation`](../ward-archetype-registry-and-creation/spec.md).
- Follow RFC-0017's exact selection matrix and ADR-0002's snapshot boundary.
- Model output is untrusted; no free-form structure survives as authority.

## Construction tests

**Integration tests:** prompt/schema decoding through resolver and tool adapter;
existing-ward conflict test; clean-start automatic selection E2E.

**Manual verification:** inspect one formatted intent injection and one tool
call trace to confirm only a closed archetype and selection source appear.

## Design (LLD)

### Interfaces & contracts

`WardRecommendation.archetype: Option<WardArchetypeId>` replaces the
model-facing `structure`. A pure resolver combines `WardAction`, explicit
override, decoded recommendation, and availability into
`ResolvedWardArchetype { id, source }`. Traces to: AC1, AC4-AC8.

### Dependencies & integration

Intent middleware produces the recommendation; runner/bootstrap writes trusted
root state; `WardTool` consumes it only on new creation; `GatewayWardLayoutAccess`
passes it to the Slice 1 service. Traces to: AC6-AC10.

### Failure, edge cases & resilience

Classifier absence/invalidity falls back to generic. Explicit invalid input is
an error. `UseExisting` discards selection. A creation attempt resolves once
so retries cannot drift. Traces to: AC4-AC9.

## Tasks

### T1: Intent schema expresses the closed selection matrix

**Depends on:** spec:ward-archetype-registry-and-creation/T1

**Touches:** `gateway/gateway-execution/src/middleware/intent_analysis.rs`, `gateway/templates/intent_analysis_prompt.md`

**Verification mode:** TDD.

**Tests:**

- TDD serialization/schema tests accept every enum and reject invented values
  (AC1).
- Table fixtures cover positive, negative, and ambiguous asks for all rows
  (AC2, AC3).
- Upgrade fixtures prove an unchanged legacy seeded prompt migrates to the new
  default and a custom prompt retains unrelated prose while legacy
  `structure` contract lines are removed from the model-visible prompt and the
  closed generated contract is appended (AC11).
- Stub handoff:
  `gateway/gateway-execution/src/middleware/intent_analysis.rs`
  `tests::ward_recommendation_schema_is_closed`,
  `tests::selection_matrix_falls_back_when_ambiguous`, and
  `tests::legacy_prompt_override_cannot_request_structure`; `stub: deferred to
  work-loop PLAN` because `new-spec` step 4 forbids committing stubs during
  spec authoring.

**Approach:**

- Import/use `WardArchetypeId` at the integration boundary.
- Replace the requested `structure` field in the prompt and structured
  contract with optional `archetype`; retain read compatibility only if
  existing stored logs require it, with no runtime consumer.
- Version the seeded default by its known digest. Replace only an unchanged
  old seed on disk; for custom prompts, compile a model-visible prompt that
  preserves unrelated prose, strips legacy structured-contract lines, and
  appends the code-owned closed response contract.

**Done when:** schema and classifier fixtures enforce the selection matrix and
free-form structure is not requested or consumed.

### T2: One resolver enforces action, override, and fallback precedence

**Depends on:** T1

**Touches:** `gateway/gateway-execution/src/middleware/intent_analysis.rs`, `gateway/gateway-execution/src/runner/invoke_bootstrap.rs`

**Verification mode:** TDD.

**Tests:**

- TDD matrix covers explicit valid/invalid, intent valid/invalid/missing,
  `CreateNew`, `UseExisting`, and classifier failure (AC4-AC6).
- Adversarial explicit overrides such as traversal, absolute-path, YAML-shaped,
  and oversized values return one stable bounded error code/envelope without
  echoing raw input, paths, YAML, schema internals, or absolute paths (AC4).
- Retry test proves one resolved value remains stable for an attempt (AC8).
- Stub handoff:
  `gateway/gateway-execution/src/middleware/intent_analysis.rs`
  `tests::resolve_ward_archetype_enforces_precedence` and
  `tests::use_existing_discards_archetype`; `stub: deferred to work-loop PLAN`
  because `new-spec` step 4 forbids committing stubs during spec authoring.

**Approach:**

- Add a pure `resolve_ward_archetype`-equivalent function returning typed ID
  and source.
- Store the resolved pair in namespaced trusted root state only for create.

**Done when:** the complete precedence matrix passes without a model string
reaching a path API.

### T3: Ward creation consumes trusted selection without delegation authority

**Depends on:** T2, spec:ward-archetype-registry-and-creation/T3

**Touches:** `runtime/agent-tools/src/tools/ward.rs`, `gateway/gateway-execution/src/invoke/ward_layout_adapter.rs`, `gateway/gateway-execution/src/invoke/executor.rs`, `gateway/gateway-execution/src/runner/invoke_bootstrap.rs`

**Verification mode:** goal-based integration with adversarial fixtures.

**Tests:**

- Integration test traces the typed value and source from root state to one
  `WardLayoutAccess::create` call (AC7, AC8).
- Prompt-injection/delegation fixtures attempting paths, YAML, or another
  identifier cannot replace trusted state (AC7, AC10).
- Existing-ward test compares bytes, digest, tree, and provenance across a
  conflicting ask (AC9).

**Approach:**

- Read the trusted selection only in the create branch and clear it after
  consumption.
- Ensure use/list/info and existing destination paths never resolve a new
  archetype.

**Done when:** integration tests prove the value is typed, single-use, and
ignored for existing wards.

### T4: Complete-bundle readiness gates clean-start automatic selection

**Depends on:** T3, spec:ward-archetype-registry-and-creation/T4, spec:bundled-ward-archetypes/T5

**Touches:** `e2e/playwright/full-mode/ward-archetypes.full.spec.ts`, `e2e/playwright/lib/harness-full.ts`, `gateway/gateway-execution/src/middleware/intent_analysis.rs`

**Verification mode:** goal-based E2E.

**Tests:**

- Fresh database/vault E2E covers coding selection, ambiguous generic
  fallback, and conflicting reuse (AC12).
- Release/readiness test refuses to enable automatic selection unless the
  shipped registry contains all seven valid bundles from the all-archetype
  construction gate; explicit corrupt/missing selection still fails (AC13).
- Run `cargo test -p gateway-execution` and `cargo test -p agent-tools`.

**Approach:**

- Extend the clean-start fixture from Slice 1 through normal intent analysis
  and ward tool invocation.

**Done when:** all three clean-start journeys and the complete-bundle readiness
gate pass with no fixture-preseeded registry or ward state.

## Rollout

Implement after Slice 1, but enable automatic binding only after the complete
Slice 3 bundle gate. The schema change retains deserialization compatibility
for old logs only where needed, but no old `structure` value is consumed.
Rollback disables automatic binding; explicit generic creation remains
available.

## Risks

- Intent fixtures can test prompt/schema discipline but not guarantee model
  accuracy; ambiguity deliberately favors generic.
- Trusted state lifetime must be scoped to one root creation attempt.
- Backward-compatible fields can accidentally regain authority; tests must
  prove no consumer reads legacy structure.

## Changelog

- 2026-07-26: initial plan.
