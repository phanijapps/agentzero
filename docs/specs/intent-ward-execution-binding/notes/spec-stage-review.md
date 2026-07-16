# Spec-stage review — 2026-07-15

## Blockers

**1. Canonical model ward validation.** `gateway/gateway-execution/src/runner/invoke_bootstrap.rs:1177`. Model-derived ward names can reach path construction before canonical inventory validation. Fix: exact-match a regular, non-symlinked ward below the canonical wards root and reject invalid, missing, and symlinked names before state, event, or tool use.

**2. Atomic ward claim.** `services/execution-state/src/repository.rs:569`. The current ward update can overwrite a concurrent bootstrap selection. Fix: add one compare-and-set claim that returns the effective persisted ward, and emit `WardChanged` only for the claimant.

**3. Setup failure cleanup.** `gateway/gateway-execution/src/runner/core.rs:862`. A failed binding can leave a started execution, subscribed client, and registered handle live. Fix: crash the root lifecycle, remove the handle, and emit a safe correlated error before returning.

**4. Delayed intent context facts.** `gateway/gateway-execution/src/runner/invoke_bootstrap.rs:1229`. Intent snapshots are written under the recommendation before the effective ward is known. Fix: carry the payload out of intent analysis and write it only after binding; leave unbound `create_new` sessions without a ward-scoped snapshot.

**5. Active-ward provenance.** `gateway/gateway-execution/src/session_state.rs:369`. Intent metadata can be rendered as an active ward even when the session has none. Fix: use `sessions.ward_id` as the sole current active-workspace source and test both recommendation and generic intent ward fields.

## Concerns

**6. Initial recall scope.** `gateway/gateway-execution/src/runner/invoke_bootstrap.rs:606`. First-turn recall is built before intent analysis, so it cannot use this binding. Fix: make that ordering an explicit non-goal here and add downstream artifact/delegation propagation coverage instead.
