# Follow-up implementation review — round 1

**1. Delayed-update acceptance criterion contradicts server ordering.** `docs/specs/session-plan-monitoring/spec.md:74`; `docs/specs/session-plan-monitoring/plan.md:91`; `services/execution-state/src/repository.rs:371`. The spec says a delayed update is rejected, while the design deliberately accepts updates in trusted server-acceptance order. Fix: replace "delayed" with the explicit repository-stale condition, or add a trusted pre-acceptance sequence.
