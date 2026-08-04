# Review Round 1

## Blockers

**1. Ship-state metadata is stale** `docs/specs/durable-work-queue/spec.md:3`

   Fix: transition it only after all reviewers are clean, as required by the work-loop.

**2. Provenance is caller-supplied** `services/execution-state/src/work.rs:145`

   Fix: have the host policy return source/provenance authorization context.

**3. Permanent failure is unverified** `services/execution-state/tests/durable_work_store.rs:262`

   Fix: add matching and stale permanent-failure tests.

**4. Corrupt pending rows can be healed by claim** `services/execution-state/src/work.rs:669`

   Fix: validate the selected row inside the claim transaction before mutation.

## Concerns

**5. Transition diagnostics omit planned fields** `services/execution-state/src/work.rs:761`

   Fix: emit and assert the bounded transition fields.

**6. Store tests copy the production schema** `services/execution-state/tests/durable_work_store.rs:13`

   Fix: initialize through the production runtime SQLite path.

**7. Retry math is not pinned** `services/execution-state/tests/durable_work_store.rs:397`

   Fix: assert the complete delay sequence.

**8. Wake absence is wall-clock dependent** `gateway/gateway-bus/tests/durable_work_queue.rs:86`

   Fix: use a deterministic counting transport for deduplication.

**9. Debug and serialization bypass redaction** `services/execution-state/src/work.rs:129`

   Fix: remove broad serialization and implement redacted debug output.

Scanner-owned CVE, license, and secret checks were not part of this source-review round.
