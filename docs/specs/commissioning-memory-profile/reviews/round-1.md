## Blockers

**1. Durable restart recovery is lost on reload.** `apps/ui/src/features/commissioning/CommissioningScreen.tsx:82`. The restart screen is entered only from the in-memory POST response, so reopening `/commission` shows the wizard instead of durable recovery. Fix: load commissioning status on mount, restore restart recovery when pending, navigate away when complete, and add reload tests.

**2. An orphan or invalid marker can falsely report restart-pending persistence.** `gateway/src/http/commissioning.rs:490`. Marker existence plus an effective `needs_attention` state is insufficient to prove exact V1 pending inputs and can produce an endless or impossible restart instruction after a later persistence failure. Fix: derive restart recovery from the direct persisted pending state and exact V1 inputs; return a finite conflict state for mismatches and test it.

**3. The integrated commissioning contract is not tested.** `gateway/src/http/commissioning.rs:543`. Helper-only tests do not prove safe-baseline preservation or the full provider/SOUL/settings/files/marker/response ordering together. Fix: add service-boundary tests for both profiles with a controllable verified provider and post-marker failure coverage.

## Concerns

**4. Governance fixtures are not fully pinned.** `gateway/gateway-memory/src/lib.rs:1859`. Tests check only kind and ID, allowing most ontology or taxonomy content to drift without breaking the exact-mimic contract. Fix: assert complete canonical bytes or stable approved digests for both governance fixtures.

## Nits

**5. The safe-baseline summary promises governance files it will not create.** `apps/ui/src/features/commissioning/CommissioningScreen.tsx:358`. Starter taxonomy and working ontology are listed unconditionally despite safe baseline creating neither. Fix: condition the items on the full profile or use profile-neutral wording.

**6. The bundled lifecycle repair leaves its public contract stale.** `gateway/gateway-execution/src/lifecycle.rs:36`. The updated test matches deferred reactivation, but the function documentation still promises eager reactivation. Fix: document that bootstrap reactivates only after durable message persistence.

**7. Completed tests retain red-phase stub terminology.** `gateway/src/http/commissioning.rs:1143`. Implemented tests and constructors still describe themselves as stubs. Fix: remove stale stub labels and helper names.

**8. Ship-state documentation is incomplete.** `docs/specs/commissioning-memory-profile/spec.md:3`. Criteria, statuses, and changelog still reflect execution rather than a reviewed implementation. Fix: update them after fixes and gates, then run spec-status lint.
