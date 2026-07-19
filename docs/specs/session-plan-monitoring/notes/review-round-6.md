# Implementation review — round 6

**1. Plan test contract contradicts rejected-event behavior.** `docs/specs/session-plan-monitoring/plan.md:186`. The plan says a rejected update preserves the raw gateway event, while the accepted spec and implementation suppress it. Fix: align the T2 test contract with the raw-event suppression rule.

**2. Foreign execution ownership lacks regression coverage.** `docs/specs/session-plan-monitoring/plan.md:158`; `services/execution-state/src/repository.rs:356`; `services/execution-state/src/repository.rs:2034`. Ownership is enforced, but no test proves a plan submitted through an execution from another session is rejected without replacing the existing snapshot. Fix: add a two-session repository test that verifies the foreign execution rejection and snapshot preservation.
