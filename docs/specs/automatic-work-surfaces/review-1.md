## Blockers

**1. Surface markers bypass the presentation capability.** `runtime/agent-runtime/src/executor.rs:1199`. Both engines parse every successful tool result for surface markers, so another tool can publish marker-shaped JSON. Fix: honor work-surface markers only from `present_surface` in both engines and add a regression test.

**2. AC12 is not verified at the execution boundary.** `gateway/gateway-execution/src/invoke/executor.rs:2545`. The direct tool test does not prove loop continuation or terminal events. Fix: add an executor-level rejected-surface-then-respond test and compare it with a respond-only run.

**3. Shipping metadata is still open.** `docs/specs/automatic-work-surfaces/spec.md:3`. The status and acceptance checkboxes have not reached the shipped state. Fix: mark the spec shipped and check met criteria after implementation review is clean.
