# Spec: Runtime Context Control

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** RFC-0001: Unified Compaction and Memory Policy; [`openai-prompt-cache-contract`](../openai-prompt-cache-contract/spec.md)

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make zbot's live conversation compaction deterministic, inspectable, and owned
by the runtime middleware path. Long tool-heavy sessions should stay under the
model context limit by clearing stale tool results first, preserving a pinned
plan anchor, and using LLM summarization only as a last resort. Durable memory
continues to live in `knowledge.db` and remains queryable through the existing
`MemoryFactStore` / `MemoryRecall` paths; this spec does not replace or mirror
durable memory storage.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off
before proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Route all live conversation forgetting through the runtime middleware path;
  no independent executor-side compaction policy may remain active.
- Preserve OpenAI-compatible prompt-cache invariants: byte-stable request
  construction, preserved message/tool order, no `cache_control`, and cache
  telemetry parsing.
- Preserve valid `tool_calls` / `tool` pairing whenever old tool results are
  cleared, summarized, or trimmed.
- Keep the plan block after the static system prefix, current on every turn,
  and protected from clearing or summarization.
- Treat `knowledge.db` as the durable memory source of truth for facts,
  episodes, procedures, patterns, wiki articles, and KG data.

### Ask first

- Changing the durable memory schema, `MemoryFactStore` trait, or
  `MemoryRecall::recall_unified` behavior.
- Adding a new provider-specific context-cache API, including Anthropic
  explicit `cache_control` support.
- Changing default trigger thresholds by more than the existing chat/deep mode
  percentages.
- Dropping existing public exports for `SummarizationMiddleware`,
  `ContextEditingMiddleware`, or `MiddlewarePipeline`.

### Never do

- Never replace `knowledge.db` durable memory with markdown files, `memory.json`,
  or `conversations.db`.
- Never use sleep workers to mutate or prune the active conversation tape.
- Never summarize stale tool result bodies when the result is deterministically
  re-fetchable; clear the body and keep a placeholder instead.
- Never summarize the stable system prefix, the injected plan block, or a message
  already marked `is_summary`.
- Never orphan tool-result messages or split an assistant tool call from its
  corresponding tool response.

## Testing Strategy

- Runtime controller invariants: **TDD**. Middleware ordering, threshold
  behavior, tool-pair preservation, plan-block protection, and last-resort
  summarization are compact logic contracts that should be pinned with unit
  tests.
- Executor integration: **goal-based check**. A targeted `cargo test -p
  agent-runtime` and `cargo test -p gateway-execution` subset should prove the
  executor no longer calls a separate `compact_messages()` live-forgetting path
  and still sends valid model messages.
- Prompt-cache compatibility: **goal-based check**. The existing
  `cargo test -p agent-runtime openai` gate remains the required regression
  check for Layer 0.
- Durable memory boundary: **goal-based check**. Grep and targeted tests should
  prove this work does not add a new durable memory store or route durable
  memory writes away from `MemoryFactStore` / `knowledge.db`.

## Acceptance Criteria

- [x] The root executor no longer invokes `compact_messages()` as an independent
  live context policy; live context forgetting happens through the middleware
  controller path.
- [x] Runtime middleware order is explicit and test-covered as
  `ContextEditingMiddleware` before `PlanBlockMiddleware` before any enabled
  last-resort summarization middleware.
- [x] `ContextEditingMiddleware` is responsible only for deterministic tool
  result clearing and related tool-call stubbing; prose summarization is not
  mixed into this layer.
- [x] Last-resort summarization, when enabled, fires only after tool-result
  clearing cannot bring the post-clear token estimate under the configured
  threshold.
- [x] Last-resort summarization excludes system messages, the plan block,
  tool-call messages, tool-result messages, and all messages with
  `is_summary = true`.
- [x] Tool-call/tool-result pairing remains valid after context control runs,
  including conversations with multiple tool calls in one assistant turn.
- [x] The injected plan block remains present, current, and unsummarized after
  context control runs on a long tool-heavy conversation.
- [x] `cargo test -p agent-runtime openai` continues to pass, proving prompt
  cache request-shape invariants are preserved.
- [x] Durable memory remains backed by `knowledge.db`; this spec adds no
  markdown memory surface, no `conversations.db` memory writes, and no
  replacement for `MemoryFactStore`.

## Assumptions

- Technical: durable facts live in `knowledge.db`, not `conversations.db`
  (source: `docs/architecture/architecture.md`; `stores/zbot-stores-sqlite/src/schema.rs`).
- Technical: `knowledge.db` contains facts, KG, wiki, procedures, episodes,
  embeddings, and vec0 indexes, while `conversations.db` contains sessions,
  messages, logs, recall metadata, and distillation run metadata (source:
  `docs/architecture/architecture.md`).
- Technical: current gateway executor wiring already installs
  `ContextEditingMiddleware` before `PlanBlockMiddleware` (source:
  `gateway/gateway-execution/src/invoke/executor.rs`).
- Technical: the legacy executor-side `compact_messages()` helper is test-only;
  production live forgetting now runs through middleware (source:
  `runtime/agent-runtime/src/context_management.rs`;
  `runtime/agent-runtime/src/executor.rs`).
- Technical: OpenAI-compatible prompt-cache policy is implemented and tested in
  `runtime/agent-runtime/src/llm/openai.rs` (source: `cargo test -p
  agent-runtime openai`, 18 tests passed on 2026-05-31).
- Process: local spec precedent is `docs/specs/openai-prompt-cache-contract/`;
  no `docs/CONVENTIONS.md` or `docs/CHARTER.md` exists in this workspace
  (source: repository read 2026-05-31).
- Product: the user confirmed the architecture direction before requesting the
  spec and plan (source: user confirmation 2026-05-31).
