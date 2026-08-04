## Blockers

**1. Timestamp truncation breaks ordering and lease fences.** `services/execution-state/src/work.rs:1023`

Persist timestamps at nanosecond precision and cover same-millisecond ordering, lease renewal, and retry scheduling.

**2. Expired corrupt leases are requeued before validation.** `services/execution-state/src/work.rs:1033`

Validate expired leased rows before recovery, dead-letter malformed rows with `integrity_violation`, and cover the recovery path with a regression test.
