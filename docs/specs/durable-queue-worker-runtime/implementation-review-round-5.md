# Implementation Review Round 5

## Blockers

**1. Shutdown can return before an already-started claim settles.** `gateway/gateway-bus/src/worker.rs:599`

Dropping the `spawn_blocking` future did not cancel the underlying claim, so a
new lease could appear after shutdown returned.

Fix: settle an already-started recovery or claim before the worker returns,
never dispatch the settled claim, and verify the observed queue status cannot
change after shutdown completion.

## Disposition

Findings remain. The in-flight store-call settlement and shutdown-race
regression were implemented in the next pass.
