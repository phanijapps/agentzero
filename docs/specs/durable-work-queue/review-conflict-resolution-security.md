# Conflict-resolution security review

## Concerns

**1. [reason] Oversized persisted payloads were parsed before the queue re-applied its payload bound.** `services/execution-state/src/work.rs:1277`

Add a schema constraint and reject the stored
byte length before JSON parsing; verify tampered rows are dead-lettered without
dispatch.
