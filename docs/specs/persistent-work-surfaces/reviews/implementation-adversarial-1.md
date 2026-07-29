# Implementation adversarial review 1

## Blockers

**1. Non-persistable updates left stale saved descriptors behind.** `gateway/gateway-execution/src/invoke/stream_event_processor.rs:108`. A valid live
surface could replace a saved display-only surface with an `ApprovalGate`
under the same stable ID; persistence ignored the new descriptor but retained
the old one for later restoration.

## Resolution

`persist_gateway_surface` now evicts an existing saved row when a valid
create/update event is outside the durable display-only allowlist. The focused
regression updates the same stable ID to `ApprovalGate`, proves the saved row
is removed, and then verifies the ordinary persist/delete path remains intact.
