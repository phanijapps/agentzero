## Blockers

**1. Integrated surface publication journey is not covered.** `runtime/agent-runtime/src/executor.rs:2793`. Tests do not walk the real presentation tool through runtime parsing and gateway conversion. Fix: add a focused real-tool integration test for create, update, and invalid input.

## Concerns

**2. Bound-data validation recurses without a depth cap.** `runtime/agent-surfaces/src/lib.rs:380`. Deep bound JSON can exhaust the validator stack. Fix: impose a depth limit with a redacted error and regression.

**3. Quick Chat keeps stale surfaces after clearing the session.** `apps/ui/src/features/chat-v2/useQuickChat.ts:290`. A successful clear does not reset surface state. Fix: clear surfaces with the fresh hydration and test it.

**4. Tool-level rejection coverage misses declared invalid cases.** `gateway/gateway-execution/src/invoke/executor.rs:2497`. Malformed pointer and over-budget cases are not exercised through the real tool. Fix: extend the rejection table and assert bounded errors.
