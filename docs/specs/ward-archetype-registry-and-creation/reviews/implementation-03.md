## Blockers

**1. Spec status still shows pre-implementation state.** `docs/specs/ward-archetype-registry-and-creation/spec.md:3`. The implementation is being readied to ship, but the spec remains `Approved` instead of moving to `Shipped` per the spec metadata contract. Fix: update the status to `Shipped` if this PR completes the spec, or `Implementing` if it is intentionally still in progress.

**2. Acceptance Criteria remain unchecked.** `docs/specs/ward-archetype-registry-and-creation/spec.md:79`. Every Acceptance Criterion is still `- [ ]` with no deferral marker, so the shipping contract silently leaves all outcomes open. Fix: mark each met criterion `- [x]` or add an inline `(deferred: <docs/backlog.md anchor>)` for any intentionally unshipped item.

## Concerns

**3. User-visible archetype behavior is missing from the product changelog.** `docs/product/changelog.md:27`. The PR adds user-visible Ward archetype creation behavior and documentation, but `Unreleased` still has no entry for it despite the convention requiring `docs/product/changelog.md` updates in the same PR as user-visible behavior changes. Fix: add an `Unreleased` changelog entry describing the new Ward archetype registry and explicit coding/generic creation behavior.
