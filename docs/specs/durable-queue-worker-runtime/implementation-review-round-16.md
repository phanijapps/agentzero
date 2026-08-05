# Implementation Review Round 16

## Concerns

**1. The cancellable claim contract is optional for `WorkStore` wrappers.** `services/execution-state/src/work.rs:612`

The default implementation checked cancellation only once and delegated to a
non-cancellable claim, so a wrapper could compile while silently dropping the
SQLite interrupt and commit-settlement fence required by worker shutdown.

Fix: make `claim_next_cancellable` required and make every implementation or
wrapper preserve the cancellation-aware call explicitly.

**2. Public cancellation helper bypasses deadline-bounded settlement.** `services/execution-state/src/work.rs:720`

The exported `cancel()` helper combined signal and blocking settlement without
a deadline, recreating the unbounded runtime path removed from worker shutdown.

Fix: remove the helper; runtime callers use immediate `signal()` and place
blocking `settle()` behind their own deadline.

## Disposition

Findings remain. The trait and cancellation surface are closed in the next
pass.
