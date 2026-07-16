# Implementation review — round 1

**1. Oversized model input is cloned and deserialized before a bound is enforced.** `gateway/gateway-execution/src/invoke/stream_event_processor.rs:98-104`; `services/execution-state/src/types.rs:740-782`. A very large plan array can cause substantial allocation or CPU before validation rejects it. Fix: add a bounded borrowed preflight check before cloning, or enforce the cap at the event decoder.

**2. Research plan surface identity is execution-scoped instead of session-scoped.** `gateway/gateway-execution/src/invoke/stream_event_processor.rs:58`; `gateway/gateway-execution/src/invoke/stream_context.rs:52`. Root and subagent plan updates create distinct card identifiers. Fix: key the projected plan surface at session scope and prove root/subagent updates produce one current Plan surface.

**3. Required durable ordering coverage is incomplete.** `docs/specs/session-plan-monitoring/plan.md:160`; `services/execution-state/src/repository.rs:1984`. Equal-timestamp, concurrent-allocation, and lower-sequence boundary coverage are absent. Fix: add repository tests for those invariants.

**4. Required stream-path tests are incomplete.** `docs/specs/session-plan-monitoring/plan.md:184`; `gateway/gateway-execution/src/invoke/stream_event_processor.rs:392`. Only pure descriptor construction is tested. Fix: add processor-path tests for accepted persistence-before-surface and rejected-no-surface behavior.
