# Plan: Runtime Context Control

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially
> (a different approach, not just a re-ordering), note why in the changelog
> at the bottom.

## Approach

Consolidate live context forgetting into one runtime-controlled pipeline:
tool-result clearing, plan-block injection, then optional last-resort
summarization. The first change is to make the middleware order observable and
testable. Then split deterministic tool-result clearing away from prose
compression, retire the executor's independent `compact_messages()` call, and
rewrite summarization so it only handles eligible prose messages after clearing
fails to free enough budget. Durable memory and prompt-cache request shaping are
guardrails, not implementation targets.

## Constraints

- RFC-0001: runtime owns live conversation forgetting; sleep workers own durable
  memory/KG hygiene only.
- [`openai-prompt-cache-contract`](../openai-prompt-cache-contract/spec.md):
  OpenAI-compatible request bodies must remain byte-stable, ordered, and free
  of Anthropic `cache_control`.
- `knowledge.db` remains the durable memory source of truth; do not add a new
  primary memory store.
- Keep public runtime middleware exports source-compatible unless the spec is
  updated and the user approves the break.

## Construction tests

**Integration tests:**
- `cargo test -p agent-runtime context`
- `cargo test -p agent-runtime summarization`
- `cargo test -p agent-runtime openai`
- `cargo test -p gateway-execution executor`

**Manual verification:** none for this spec; behavior is backend/runtime logic.

## Tasks

### T1: Middleware order is named and test-covered

**Depends on:** none

**Touches:** `gateway/gateway-execution/src/invoke/executor.rs`,
`runtime/agent-runtime/src/middleware/pipeline.rs`

**Tests:**
- TDD: add or extend a unit test proving the gateway executor builds the root
  middleware pipeline in the order `context_editing`, `plan_block`, and then
  future summarization if enabled.
- Goal-based: `cargo test -p agent-runtime middleware::pipeline` stays green.

**Approach:**
- Add a read-only inspection surface on `MiddlewarePipeline`, such as
  `pre_processor_names()`, gated to avoid exposing internals beyond names.
- Add a focused builder/helper test around the gateway pipeline construction.
- Keep the production order unchanged in this task.

**Done when:** a failing test would catch `PlanBlockMiddleware` being moved
before `ContextEditingMiddleware`.

### T2: Tool-result clearing no longer compresses prose

**Depends on:** T1

**Touches:** `runtime/agent-runtime/src/middleware/context_editing.rs`,
`runtime/agent-runtime/src/context_management.rs`

**Tests:**
- TDD: a context-editing test with old assistant prose verifies the prose text is
  unchanged when only tool results are cleared.
- TDD: existing skill-aware unload and cascade tests still prove tool results and
  skill resources are replaced with placeholders.
- TDD: a multi-tool-call turn remains pair-valid after selected tool results are
  cleared.

**Approach:**
- Remove the `compress_old_assistant_messages()` call from
  `ContextEditingMiddleware::process`.
- Keep `clear_tool_results`, `clear_tool_call_inputs`, skill-aware placeholders,
  and cascade unload behavior in this layer.
- Move any tests that were really asserting assistant prose compression to the
  summarization task or delete them if they only covered legacy coupling.

**Done when:** context editing clears stale tool bodies but never rewrites
assistant prose.

### T3: Executor live compaction is retired

**Depends on:** T2

**Touches:** `runtime/agent-runtime/src/executor.rs`,
`runtime/agent-runtime/src/context_management.rs`,
`runtime/agent-runtime/src/progress.rs`

**Tests:**
- TDD: executor/token-threshold tests prove the pre-compaction memory flush nudge
  can still be emitted without calling `compact_messages()`.
- Goal-based: `rg -n "compact_messages\\(" runtime/agent-runtime/src/executor.rs`
  returns no call sites.
- Goal-based: `cargo test -p agent-runtime context_management progress` passes
  after any helper deprecation.

**Approach:**
- Replace the direct `current_messages = compact_messages(current_messages)`
  branch with middleware-owned behavior.
- Keep the memory flush warning if it remains useful, but classify it as a nudge
  before middleware pruning, not a compaction trigger.
- Decide whether `compact_messages()` remains test-only/deprecated helper or is
  removed after downstream tests migrate.

**Done when:** the executor no longer has a second live forgetting policy.

### T4: Last-resort summarization has a narrow eligibility contract

**Depends on:** T2, T3

**Touches:** `runtime/agent-runtime/src/middleware/summarization.rs`,
`runtime/agent-runtime/src/middleware/config.rs`,
`runtime/agent-runtime/src/types/messages.rs`,
`gateway/gateway-execution/src/invoke/executor.rs`

**Tests:**
- TDD: summarization excludes all system messages, plan-block messages,
  assistant messages with tool calls, tool messages, and messages already marked
  `is_summary`.
- TDD: summarization preserves the plan block and recent turns when rewriting a
  long mixed conversation.
- TDD: summarization does not trigger when post-context-editing token count is
  below the configured threshold.
- TDD: summarization failure returns an error or proceeds according to an
  explicit fallback policy; it must not silently corrupt the message tape.

**Approach:**
- Rewrite `split_messages` around explicit eligibility predicates rather than
  broad system/non-system buckets.
- Mark generated summary messages with `is_summary = true`.
- Add config for "trigger only after previous middleware did not reclaim enough"
  if the current `TriggerCondition` cannot express it cleanly.
- Wire the gateway runtime pipeline to append enabled summarization after
  `PlanBlockMiddleware`, using the post-context-editing message tape and the
  same threshold.
- Keep summary model/provider config source-compatible unless an approved spec
  update says otherwise.

**Done when:** summarization is a prose-tail fallback and cannot touch tool
output, the stable prefix, the plan block, or prior summaries.

### T4b: Sleep compactor verifier fails closed

**Depends on:** none

**Touches:** `gateway/gateway-memory/src/sleep/compactor.rs`,
`gateway/gateway-memory/src/sleep/worker.rs`,
`docs/architecture/future-state/compaction-strategy.md`

**Tests:**
- TDD: a verifier-enabled compactor skips a candidate when either entity cannot
  be loaded for verifier adjudication.
- TDD: verifier rejection skips merge and increments verifier-skip stats.
- Goal-based: `cargo test -p gateway-memory compactor` passes.

**Approach:**
- Treat missing candidate entity data as verifier failure when a verifier is
  configured.
- Propagate `merges_skipped_by_verifier` from `CompactionStats` through
  `CycleStats` and sleep-cycle tracing.
- Update future-state text to say `knowledge.db` is the durable source and the
  normal service path wires the verifier; the remaining invariant is fail-closed
  behavior.

**Done when:** sleep compaction cannot merge a verifier-enabled candidate
without both entity records and an affirmative verifier result.

### T5: Runtime context controller contract is covered end-to-end

**Depends on:** T1-T4

**Touches:** `runtime/agent-runtime/src/middleware/*`,
`runtime/agent-runtime/src/executor.rs`,
`gateway/gateway-execution/src/invoke/executor.rs`

**Tests:**
- TDD/integration: a synthetic long, tool-heavy conversation runs through the
  full middleware pipeline and ends with valid tool pairs, a current plan block,
  cleared old tool bodies, and no summarized tool output.
- Goal-based: `cargo test -p agent-runtime context` passes.
- Goal-based: `cargo test -p gateway-execution executor` passes.

**Approach:**
- Add one fixture builder for long mixed conversations with system messages,
  `update_plan`, assistant tool calls, tool responses, and prose turns.
- Exercise the actual middleware pipeline rather than each middleware in
  isolation.
- Assert externally observable message properties instead of exact full message
  arrays where possible.

**Done when:** the target architecture is proven by one realistic pipeline test.

### T6: Durable memory and prompt-cache boundaries are protected

**Depends on:** T1-T5

**Touches:** `runtime/agent-runtime/src/llm/openai.rs`,
`gateway/src/state/mod.rs`,
`stores/zbot-stores-sqlite/src/*`,
`docs/specs/runtime-context-control/spec.md`

**Tests:**
- Goal-based: `cargo test -p agent-runtime openai` passes.
- Goal-based: `rg -n "cache_control" runtime gateway framework services apps stores`
  has no OpenAI-compatible request emission matches.
- Goal-based: `git diff --name-only` and `git diff -U0` show no new durable
  memory replacement path, no `memory_surface` module, and no writes that route
  durable facts to `conversations.db`.

**Approach:**
- Run the existing OpenAI cache gate after runtime changes.
- Audit the diff for accidental durable memory scope creep.
- If implementation discovers a legitimate durable-memory need, stop and update
  the spec before changing storage.

**Done when:** the runtime context work is demonstrably isolated from durable
memory replacement and prompt-cache regressions.

## Rollout

Ship as a runtime behavior change behind existing chat/deep mode thresholds. No
new UI or data migration is required. The rollback path is to revert the runtime
middleware/executor changes; `knowledge.db` and `conversations.db` schemas are
unchanged.

## Risks

- Removing executor-side compaction may expose edge cases where middleware does
  not run, especially tests or alternate executor construction paths that use an
  empty `MiddlewarePipeline`.
- Tightening summarization eligibility may reduce token savings for prose-heavy
  conversations until the fallback is tuned.
- Existing tests may depend on exact legacy placeholder text or one-line
  assistant compression, even when those details are not public behavior.
- Summary model failures need a clear fallback; a failed last-resort summary must
  not cause message corruption or invalid tool-pair payloads.

## Changelog

- 2026-05-31: initial plan.
- 2026-05-31: added T4b for sleep-compactor verifier fail-closed behavior and
  clarified gateway summarization wiring under T4.
