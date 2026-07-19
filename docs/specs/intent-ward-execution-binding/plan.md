# Plan: Intent Ward Execution Binding

- **Spec:** [`spec.md`](spec.md)
- **Status:** Complete

## Approach

Keep ward selection single-sourced in bootstrap. Convert a `use_existing`
recommendation to a canonical ID only by exact, safe inventory validation.
Atomically claim that ID if the session ward is empty; use the returned
effective persisted ward for the root executor and publish `WardChanged` only
when this invocation made the claim. Delegation and artifact persistence then
inherit the binding through their existing session reads. Do not make an intent
recommendation itself a session-state fallback.

## Constraints

- Retain explicit `create_new` and model-driven ward creation behavior.
- Add a narrow `StateService` compare-and-set ward claim rather than using the
  unconditional `update_session_ward` write for bootstrap routing.
- Reuse the existing `GatewayEvent::WardChanged` payload and lifecycle crash
  helper; add no endpoint or event variant.
- Preserve the existing artifact confinement and manifest-only serving model.
- Do not touch unrelated artifact hardening or the known Engram adapter build
  failure.
- Initial automatic recall stays unscoped because it precedes intent analysis;
  it is outside this filesystem/tool-context correction.

## Construction tests

**Integration tests:** atomic state-service claim race; bootstrap binding test
with a temporary runtime DB; downstream artifact/delegation ward resolution;
session-state builder test against intent metadata and an empty `ward_id`.

**Manual verification:** after the daemon is running, submit a simple
file-producing Research request that intent routes to an existing ward; inspect
the Research ward indicator, generated file location, and persisted artifact.

## Design (LLD)

### State & control flow

`run_intent_analysis` returns a canonical existing-ward candidate plus its
intent/prompt snapshot payload; it never puts model-authored ward text directly
into a filesystem or state sink. `create_executor` owns the transition: it
atomically claims the candidate only when the session ward is empty, publishes
`WardChanged` only if it claimed, then passes the effective persisted ward into
`ExecutorBuilder`. A pre-bound session wins. The `create_new` path returns no
automatic binding, so its established ward-tool flow remains responsible for
creation. Intent/prompt facts are written only after the effective ward is
known; an unbound `create_new` execution intentionally skips that speculative
duplicate. Its execution log and prompt history remain available until its
explicit ward entry, rather than storing facts under a model-proposed scope.
Traces to AC1, AC2, AC4, and AC5.

`SessionStateBuilder` treats `sessions.ward_id` as the active-workspace source
of truth. It no longer treats an intent recommendation as a ward fallback.
Traces to AC3.

### Failure, edge cases & resilience

The claim operation is a single `UPDATE ... WHERE ward_id IS NULL` followed by
a read of the effective persisted value in the same repository operation. It
rejects all non-canonical model names before directory inspection or a state
write. Validation checks the `wards/` root itself with `symlink_metadata`,
canonicalizes it, rejects a symlinked candidate, and confirms the child's
canonical path remains below that root immediately before use. The binding is
fail-closed: after setup has started, a claim failure
uses the existing crash lifecycle helper with session/execution correlation,
removes the registered handle, and returns a generic error. Event publication
only follows a successful claim. Historical sessions with no persisted ward
remain wardless; this change does not relocate scratch files or backfill
artifact manifests. Traces to AC1, AC2, AC3, and AC6.

## Tasks

### T1: Atomically claim a canonical existing ward

**Depends on:** none

**Touches:** `services/execution-state/src/{repository.rs,service.rs}`, focused
execution-state tests

**Tests:**

- TDD: competing `financial-analysis` and `travel-planning` claims on a
  wardless session yield one persisted effective ward with no overwrite.
  Covers AC1 and AC4.
- TDD: an already-bound session returns its existing ward without mutation.
  Covers AC4.

**Approach:**

- Add the smallest state-service claim result that distinguishes a successful
  claim from an existing effective ward.
- Use a compare-and-set write instead of `update_session_ward`; preserve that
  method for explicit ward-tool changes.

**Done when:** every caller can obtain one stable effective ward without a
read-then-write race.

### T2: Bind safe intent output through bootstrap and downstream execution

**Depends on:** T1

**Touches:** `gateway/gateway-execution/src/runner/{invoke_bootstrap.rs,core.rs}`,
`gateway/gateway-execution/src/{delegation/spawn.rs,invoke/stream_event_processor.rs}`,
focused bootstrap/delegation/artifact tests

**Tests:**

- TDD: a canonical existing ward binds before root executor construction and
  produces one `WardChanged`; only the canonical ID reaches executor state.
  Covers AC1 and AC5.
- TDD: absolute, separator-containing, dot, missing, and symlinked candidates
  do not claim, emit, or construct a ward context. Covers AC2.
- TDD: a bound session's artifact declaration and delegated execution read the
  same persisted ward instead of `scratch`. Covers AC3.
- TDD: a claim failure crashes the already-started root execution/session,
  removes its handle, emits a generic correlated error, and emits no ward
  event. Covers AC5.

**Approach:**

- Return the canonical candidate and snapshot payload from intent analysis;
  delay session-context writes until the atomic claim yields an effective ward.
- Validate the model candidate as exactly one non-symlinked ward directory
  under a canonical non-symlinked wards root before determining graduation or
  claiming, including a symlinked-root regression test.
- Route claim failure through the existing crash helper at the
  `invoke_with_callback` setup seam, removing the handle before return.
- Use the effective persisted ward for the root executor; leave `create_new`
  unbound until the existing ward tool explicitly changes state.

**Done when:** root, delegation, and artifact flows agree on one safe,
persisted ward and setup never leaves a stranded running execution.

### T3: Stop treating recommendations as active session state

**Depends on:** T2

**Touches:** `gateway/gateway-execution/src/session_state.rs`,
`gateway/gateway-execution/tests/session_state_tests.rs`

**Tests:**

- TDD: intent metadata containing `ward_recommendation` and generic `ward`
  values with a null session ward yields no `SessionState.ward`. Covers AC3.
- TDD: a nonempty persisted session ward still appears in session state.
  Covers AC3 and AC4.

**Approach:**

- Make `sessions.ward_id` the only active-ward source for current session
  state. Do not infer an active workspace from intent metadata.

**Done when:** session snapshots cannot misrepresent a recommendation as a
bound workspace.

### T4: Verify the runtime correction and close the defect record

**Depends on:** T1-T3

**Touches:** `docs/specs/intent-ward-execution-binding/*`, `docs/backlog.md`,
`docs/specs/README.md`

**Tests:**

- Goal-based: run `cargo test -p gateway-execution` for the new focused
  bootstrap and session-state tests, `cargo fmt --check`, and `cargo check -p
  gateway-execution`.
- Goal-based: run `git diff --check` and the work-loop spec-status lint.

**Approach:**

- Mark checked acceptance criteria with recorded test evidence.
- Replace the active backlog entry with a short resolution reference after all
  criteria pass.

**Done when:** the defect has regression coverage, the spec is shipped, and
mechanical checks are clean within the known unrelated build limitation.

## Rollout

Delivery is an immediate behavior correction with no migration or external
dependency. The change is reversible by reverting the bootstrap binding;
historical sessions remain untouched. A manual daemon smoke verifies newly
created sessions only.

## Risks

- Binding a non-existent or untrusted ward would make file resolution unsafe;
  limit automatic binding to the bootstrap-validated `use_existing` path.
- Binding after executor construction would reproduce the root defect; tests
  must observe the effective ward before construction.
- Changing historical session fallback behavior can hide previously misleading
  ward chips, which is intentional data-correctness behavior.

## Changelog

- 2026-07-15: Initial defect-correction plan from
  `intent-recommended-ward-is-not-execution-bound-defect`.
- 2026-07-15: Expanded after security and adversarial review: canonical model
  output validation, atomic claims, delayed context facts, downstream
  propagation, and setup-failure lifecycle cleanup are mandatory.
- 2026-07-15: Shipped: bootstrap now binds only validated `use_existing`
  wards with an atomic claim, revalidates every effective ward before context
  and executor use, and uses the persisted result for root, delegation, and
  artifact paths. Added concurrent-claim, bind/event, root-context, artifact,
  session-state, and setup-failure cleanup regression coverage.
