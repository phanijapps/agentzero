# Implementation Review Round 7

## Blockers

**1. The claim commit fence can consume the shutdown deadline before timeout starts.** `gateway/gateway-bus/src/worker.rs:707`

The synchronous cancellation call waited on a write gate before the bounded
join timeout began.

Fix: use rusqlite 0.32.1's cross-thread `InterruptHandle` to interrupt the
active SQLite operation, hold the commit gate only around the final
cancellation check and commit, and run cancellation itself inside the same
absolute shutdown deadline. The real `SqliteWorkStore` path now delays past the
drain and proves both bounded return and no later lease.

## Disposition

Findings remain. The interrupt-backed bounded cancellation path was implemented
in the next pass.
