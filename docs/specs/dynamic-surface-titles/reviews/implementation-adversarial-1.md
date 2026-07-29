# Implementation adversarial review 1

## Blockers

**1. Raw type fallback still leaks when the component id is generic.** `apps/ui/src/features/surfaces/A2uiSurfaceRenderer.tsx:31`. `humanizedId` turns ids such as `callout` or `Callout` into the visible fallback `Callout`. Fix: detect id-derived labels equal to the component type and use `Component`.

**2. Spec metadata is stale for an implementation PR.** `docs/specs/dynamic-surface-titles/spec.md:3`. The spec remains `Approved` while implementation is active. Fix: advance it to `Implementing`.

**3. AC8 action availability is not verified.** `apps/ui/src/features/surfaces/A2uiSurfaceRenderer.test.tsx:124`. The existing assertion does not cover an enabled static-title ApprovalGate. Fix: add the static-title regression assertion.

**4. Spec index edit is outside the implementation plan.** `docs/specs/README.md:19`. The process-required index update is not explained in the plan. Fix: explicitly record it as new-spec lifecycle bookkeeping outside the product task mapping.
