## Concerns

**1. Recovery and integrity dead-letter diagnostics omit item identity.** `services/execution-state/src/work.rs:773`

Carry each affected work ID through corrupt-pending and expired-lease recovery mutations, emit per-item diagnostics with safe work ID, kind, target, attempt, transition, and reason code fields, and assert those fields in observability tests.
