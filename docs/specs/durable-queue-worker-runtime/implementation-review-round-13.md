# Implementation Review Round 13

## Blockers

**1. Claim cancellation can still commit after shutdown intent.** `services/execution-state/src/work.rs:696`; `services/execution-state/src/work.rs:729`; `services/execution-state/src/work.rs:943`

Cancellation published its atomic flag outside the commit gate, while a claim
checked the flag only once after acquiring shared commit authority. A signal
could therefore publish between that check and SQLite commit.

Fix: publish cancellation under the gate's exclusive authority and execute the
final SQLite commit under the corresponding shared authority. Add a regression
that pauses inside commit authority and proves signal cannot linearize midway.

**2. Gateway same-store wiring is unverified.** `gateway/src/server.rs:559`

The lifecycle test proved only handle ownership; it would pass if the worker
used a different queue store from `AppState`.

Fix: enqueue and wake through the gateway state's runtime store and transport,
then prove the empty-registry worker dead-letters that exact row with
`handler_unavailable` before shutdown.

## Disposition

Findings remain. Commit/cancellation linearization and same-store lifecycle
coverage are implemented in the next pass.
