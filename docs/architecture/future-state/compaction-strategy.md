# Compaction Strategy for AgentZero

**Status**: future-state proposal, not yet implemented
**Last updated**: 2026-04-23
**Audience**: maintainers and engineers implementing against this plan

---

## 1. Why this document exists

AgentZero today runs two independent compaction systems that do not know about each other:

1. **Runtime conversation compaction** — `runtime/agent-runtime/src/middleware/summarization.rs`. Token-threshold-triggered summarization of the in-flight LLM conversation. Uses a 4-char-per-token heuristic. Off by default; agents opt in. No cost tracking, no retry, no real tokenizer.
2. **Sleep-cycle memory compaction** — `gateway/gateway-execution/src/sleep/*.rs`. A 60-minute background cycle that merges knowledge-graph entities (cosine ≥ 0.92), synthesizes cross-session strategy facts (LLM at temp 0.0, budget 10 calls/cycle), decays orphan entities (30 days, limit 100), and prunes/archives. All thresholds are hardcoded Rust constants. No transactional envelope. The `PairwiseVerifier` trait is defined but unwired.

These systems share no vocabulary, no budget, and no policy. The live conversation never tells the knowledge graph "this turned out to matter"; the knowledge graph never tells the runtime "you already summarized away something I'm now synthesizing as a strategy." This is the root cause of the ward-routing-fragmentation defect recorded in `docs/architecture/defects/`.

Two researchers — one on Anthropic's documented stack, one on the industry/academic state of the art — converged independently on the same recipe. This document captures that recipe, maps it to AgentZero's concrete code, and defines the order of operations.

---

## 2. Current state, bluntly

### 2.1 What the runtime compactor gets right
- Middleware pipeline shape is correct (`MiddlewarePipeline::process_messages`, `pipeline.rs:75`) — a clean extension point.
- Tool-safe message splitting walks backward to avoid cutting across `tool_use`/`tool_result` pairs.
- System messages are always preserved and re-prepended after summarization.

### 2.2 What the runtime compactor gets wrong
- **4-char-per-token heuristic** (`token_counter.rs:17-24`) under-counts Claude by ~25% and over-counts Llama by ~20%. Every eviction fires at the wrong threshold, and the error is larger for code and non-English text.
- **No cost tracking or circuit-breaker** — every firing is an extra LLM call with no budget.
- **No retry/fallback** if the summarizer LLM fails mid-turn. The main inference path silently depends on a second, untracked round-trip.
- **Temperature hardcoded at 0.3**; no customization point per agent.
- **Tests cover only a trivial 3-message case** — no long-conversation, no multi-pass, no tool-heavy scenario.

### 2.3 What the sleep compactor gets right
- Four-stage decomposition (compact → synthesize → decay → prune) is architecturally sound.
- `kg_compactions` audit table is under-used but structurally right.
- Soft-delete via `compressed_into = winner_id` / `__pruned__` sentinel preserves referential integrity and enables recovery from bad merges.

### 2.4 What the sleep compactor gets wrong
- **No transactional envelope across stages** — a panic in stage 3 leaves stages 1-2 committed.
- **`PairwiseVerifier` trait defined but never wired** (`compactor.rs:34-37`). Merge decisions are cosine-only → false merges (`maritime-tracking` vs `maritime-vessel-tracking`).
- **Decay is orphan-only** — rarely-referenced but connected junk entities live forever.
- **LLM budget is call-count, not cost** — 10 calls × large prompts is not a budget.
- **All thresholds hardcoded in Rust**; no YAML/DB config surface; no per-agent tuning.
- **Minimal tests**; no end-to-end cycle tests against realistic KG state.

### 2.5 The cross-cutting failure
There is no shared **forgetting policy**. Each system decides independently what is allowed to be forgotten from live conversations versus durable memory. Every future tuning knob has to be added in two places with two different semantics.

---

## 3. Research convergence

The two independent researches agreed strongly. Summarized findings below; full citations in section 10.

### 3.1 Anthropic's published three-layer stack
1. **Prompt caching** on the stable prefix (system prompt + tool schemas). 5-min TTL as of 2026 — invalidation is frequent; plan accordingly.
2. **Compaction** (`/compact`) on the conversation tail. Client-side single-model summarization. Preserves user requests and key snippets; summarizes older tool outputs. Same model; no cheaper-model switch.
3. **Memory tool** (`/memories/*.md` file tree). Durable cross-session storage. Complementary to compaction, not an alternative: "Compaction keeps active context manageable; memory persists what must survive summary."
4. **Context editing API** (beta `context-management-2025-06-27`, `clear_tool_uses_20250919`). The newer primitive: surgically *deletes* stale tool results rather than summarizing them. 84% token reduction in Anthropic's internal eval. The model still retrieves the cleared info via memory tool if needed.

### 3.2 The industry's convergent production stack
The systems mature enough to talk about — Claude Code, Cursor, Cognition/Devin, Replit Agent, Letta, Windsurf — all converge on (some subset of) the same four patterns, prioritized:

1. **Structured scratchpad / plan state** (Manus `todo.md`, Anthropic `claude-progress.txt`, Devin). A compact model-rewritten plan doc that survives turn rotation. The single most durable anchor for tool-heavy agents.
2. **Tool-result clearing, not summarizing** (Anthropic beta, Cursor). Replace old `tool_result` bodies with placeholders; keep the `tool_use` envelope for API validity. Re-fetchable output (file reads, shell, search) should always be cleared, never summarized.
3. **Targeted compaction** of prose reasoning — never recursive self-summarization. Anthropic's own probe showed 0 of 3 quantitative facts survived their summarization pass.
4. **Cross-session memory file** for fact distillation and re-injection. File surface (markdown, diff-visible, user-editable), not opaque KV.

### 3.3 Research results that load-bear on this decision
- **Chroma — Context Rot (2025)**: 18 frontier models degrade with length *and* with semantic-distractor density, even well below the window cap. Argues for aggressively pruning context even when you "have room."
- **StreamingLLM / attention sinks (ICLR 2024)**: "first few tokens + rolling window" is architecturally safe. Explains why the prefix-plus-recent pattern works even though middle drop looks lossy.
- **Anthropic's summarization probe**: 0/3 quantitative facts survived a summarization pass — compaction drift is real; never let the model summarize itself unsupervised without a bounded schema.

### 3.4 Universal anti-patterns
The research unanimously warns against:
- Char-per-token heuristics driving eviction decisions (breaks 2-4× on code and Unicode).
- Unsupervised self-summarization (drift compounds; obscure-but-load-bearing facts disappear).
- Two uncoordinated compactors with different policies (contradictory recall, which is exactly what AgentZero has today).
- Cosine-only entity merging without type/name guards (fragmentation or false merges).
- Orphaning `tool_use`/`tool_result` pairs (Anthropic API returns HTTP 400).
- Compaction that rewrites the stable prefix (invalidates prompt cache).
- Semantic-similarity-only retrieval of history (distractor interference degrades coherence).

---

## 4. The recipe

Five layers, bottom-up in per-turn order. Each layer names the Anthropic or industry primitive it corresponds to and cites the research that makes it non-negotiable.

### Layer 0 — Stable prefix with cache breakpoints
- **What it is**: the system prompt, tool schemas, and agent persona. Never rewritten after session start.
- **Mechanism**: one `cache_control: {type: "ephemeral"}` breakpoint at the tail of the stable prefix, and another on the plan block (Layer 1).
- **Preserves**: deterministic prefix bytes.
- **Discards**: nothing.
- **Corresponds to**: Anthropic prompt caching.
- **Why non-negotiable**: without this, every efficiency gain of layers 2-4 is swamped by re-billing the prefix on every call. Cache TTL dropped to 5 min in early 2026, so this is more load-bearing than it was.

### Layer 1 — Pinned scratchpad (the plan block)
- **What it is**: a single model-rewritten block in a fixed slot right after the stable prefix and before the rolling tape.
- **Contents**: current goal, plan checklist, 3-5 line "what I just learned" delta, pointers (not contents) to key files touched.
- **Mechanism**: rewritten in place by the `update_plan` tool and by an implicit post-turn executor pass that asks the model to rewrite the block when the tape has moved materially.
- **Preserves**: intent, progress, decisions — survives every other layer's forgetting.
- **Discards**: prose reasoning, tool bodies.
- **Corresponds to**: Manus `todo.md`, Claude Code `claude-progress.txt`, Devin's progress file.
- **Why non-negotiable**: #1 finding in the survey. For tool-heavy agents this is the single most durable anchor. Compaction drift (Anthropic's 0/3 probe) is survivable only if an unsummarized plan block sits above the summary. Chroma's context-rot result argues the plan block is what keeps precision up as length grows.

### Layer 2 — Tool-result clearing (surgical deletion)
- **What it is**: replacement of old `tool_result` message bodies with placeholders, keeping the `tool_use` envelope intact for API validity.
- **Policy**: `keep_last_n = 3` by default. Exclude list for non-idempotent tools (anything with write side effects). Re-fetchable tool output — file reads, shell output, search results — is always cleared, never summarized.
- **Preserves**: tool-call graph structure, recent results.
- **Discards**: stale re-fetchable bodies.
- **Corresponds to**: Anthropic `clear_tool_uses_20250919`. Implementation already exists at `runtime/agent-runtime/src/middleware/context_editing.rs`.
- **Why non-negotiable**: 84% token reduction in Anthropic's eval at zero summarization risk. We already have this built but disabled by default.

### Layer 3 — Targeted tail summarization (last resort)
- **What it is**: single-pass LLM summarization of the oldest contiguous prose tail — assistant reasoning and user turns only, never tool results (Layer 2 handles those), never the plan block (Layer 1).
- **Trigger**: fires only after Layer 2 has cleared everything it can and post-Layer-2 token count ≥ 0.7 × real-context-window. Token count comes from the real tokenizer, not the 4-char heuristic.
- **Invariants**: one pass per trigger. Never re-summarize an existing summary (enforce via message flag, not string prefix sniff).
- **Preserves**: narrative continuity.
- **Discards**: verbose reasoning.
- **Corresponds to**: Anthropic `/compact`.
- **Why non-negotiable**: when all else fails we still need tail compression, but rationed — Anthropic's own probe shows 0/3 quantitative facts survive summarization, so Layer 1 carries what Layer 3 drops.

### Layer 4 — Durable memory surface
- **What it is**: `knowledge.db`, the durable source of truth for facts, KG data, wiki articles, procedures, episodes, patterns, embeddings, and compaction audit data.
- **Mechanism**:
  - **Read**: recall paths query `knowledge.db` through `MemoryFactStore`, `MemoryRecall`, KG, episode, procedure, and related store traits.
  - **Write**: tools, distillation, ingestion, and sleep-cycle hygiene write through the existing store/repository abstractions backed by `knowledge.db`.
- **Preserves**: facts that must survive every other layer's forgetting.
- **Discards**: nothing by default; sleep-cycle pruning/compaction must be audited and verifier-gated where entity merges are possible.
- **Corresponds to**: AgentZero's existing database-backed memory layer, not a markdown mirror or replacement.
- **Why non-negotiable**: durable memory has one authoritative store. Adding a second markdown source of truth would split recall, compaction, and audit semantics.

---

## 5. Mapping to AgentZero code

### Layer 0 — cache breakpoints
- **Extends**: `runtime/agent-runtime/src/llm/client.rs` and provider impls (`openai.rs`, `non_streaming.rs`).
- **Work**: request builder emits `cache_control: {type: "ephemeral"}` on the last stable-prefix block and on the plan block.
- **New modules**: none.

### Layer 1 — plan block middleware
- **New**: `runtime/agent-runtime/src/middleware/plan_block.rs` implementing `PreProcessMiddleware`.
- **Runs last** in the pipeline so it owns the final rendered message list.
- **Source of truth**: `app:plan` session state, already written by `runtime/agent-tools/src/tools/execution/update_plan.rs`.
- **Extensions**:
  - `update_plan.rs` grows a "notes" field and a "recent-deltas" appender.
  - `runtime/agent-runtime/src/executor.rs` grows a post-turn hook that asks the model to rewrite the block when the tape has moved materially.

### Layer 2 — tool-result clearing
- **Already exists** at `runtime/agent-runtime/src/middleware/context_editing.rs`.
- **Work**: flip defaults to `enabled: true`, derive `trigger_tokens` from real context window (depends on Layer 0's tokenizer work), set `keep_tool_results: 3`.
- This is wiring, not new code.

### Layer 3 — rebuilt summarization
- **Replaces**: `runtime/agent-runtime/src/middleware/summarization.rs` (keep the file, rewrite behavior).
- **New behavior**:
  - Fires only if post-Layer-2 token count ≥ 0.7 × context_window.
  - Summarizes only prose messages (role=user; role=assistant with `tool_calls.is_none()`).
  - Never touches Layer 1 block.
  - Never summarizes an already-summary message (anti-recursion flag on the message struct).
- **Rewrites**: `runtime/agent-runtime/src/middleware/token_counter.rs`.
  - Wraps `tiktoken-rs` for OpenAI-compatible providers.
  - Uses Anthropic's count-tokens endpoint (or the server-side token count if exposed) for Claude.
  - Real `tokenizers` crate model for Ollama/local.
  - 4-char heuristic goes.

### Layer 4 — durable memory surface
- **Existing source of truth**: `knowledge.db` via the zbot-stores repositories and traits.
- **Adapter**: `runtime/agent-tools/src/tools/memory.rs` remains a tool-facing API over the database-backed memory layer.
- **Session-close distillation**: lives in existing `gateway/gateway-execution/src/distillation.rs` and writes durable facts through `MemoryFactStore`.
- **Sleep compactor scope**: `gateway/gateway-memory/src/sleep/compactor.rs` is scoped to durable KG hygiene. It no longer represents a parallel forgetting policy against live conversation state.
- **Verifier status**: the normal `gateway-memory` service construction wires `LlmPairwiseVerifier`; verifier-enabled compaction must fail closed when either entity record is unavailable.

### Pipeline ordering
Enforce in `runtime/agent-runtime/src/middleware/pipeline.rs` setup (not by convention):
```
ContextEditing (Layer 2)  →  PlanBlock (Layer 1)  →  Summarization (Layer 3)
```
Layer 0 happens at request-build time, not in the middleware pipeline. Layer 4 happens at session-start (read) and session-close (write), not per-turn.

---

## 6. Phased rollout

Three phases, each independently shippable. Honest sizing in engineer-days.

### Phase 1 — Foundations (4-5 days)
**Ships**: Layers 0 and 2.

- Real tokenizer in `token_counter.rs` (tiktoken + Anthropic count endpoint + tokenizers crate).
- Prompt-cache breakpoints in `LlmClient` and each provider's request builder.
- Flip `ContextEditingMiddleware` to enabled-by-default with `keep_tool_results: 3` and real-tokenizer-derived trigger.

**If skipped**: every later layer fires at the wrong threshold, and every request re-bills the prefix. Phase 2 blocks on the real tokenizer.

**Acceptance**:
- `token_counter::count_tokens(msg, model)` returns within ±5% of the provider's own counter across OpenAI, Claude, and Llama.
- A synthetic conversation at 150% of context window drops below 70% after one middleware pass without a single LLM call (Layer 2 alone should handle it).
- Prompt-cache telemetry shows hit rate > 80% on the stable prefix across two back-to-back requests.

### Phase 2 — The anchor (5-7 days)
**Ships**: Layer 1 and a rebuilt Layer 3.

- New `plan_block.rs` middleware.
- Plan-block cache breakpoint wired.
- Post-turn rewrite hook in `executor.rs`.
- `summarization.rs` neutered to fire only as last-resort tail compactor; anti-recursion flag added to the message struct.

**If skipped**: compaction drift still compounds (Chroma's context-rot result argues the plan block is what keeps precision up as length grows), and the two compactors continue to fight over the same tape.

**Acceptance**:
- In a 100-turn tool-heavy synthetic session, the plan block is readable at turn 100 and still contains the original goal and all checkpoints added along the way.
- Summarization fires at most once per 50 turns under realistic load.
- No message in the tape is ever double-summarized (anti-recursion test).

### Phase 3 — Durability (7-10 days)
**Ships**: Layer 4 hardening and retirement of duplicate forgetting policies.

- `knowledge.db` remains the durable source of truth; no markdown memory mirror.
- `MemoryTool` remains an adapter over the database-backed memory layer.
- `distillation.rs` continues to write durable memories through existing store abstractions.
- Sleep compactor remains durable KG hygiene only; hardcoded constants can move to YAML or DB config later.
- `PairwiseVerifier` is wired in the normal service path; verifier-enabled compaction fails closed if entity records are unavailable.

**If skipped**: facts that Layer 3 drops are recoverable only when already saved to `knowledge.db`; unsafe sleep compaction can still create false durable KG merges.

**Acceptance**:
- Session N+1 can recall a named fact written by session N via `memory.recall` without re-running any tool calls from session N.
- No new markdown memory source of truth is added.
- Verifier-enabled sleep compaction skips a candidate when either entity record cannot be loaded.

---

## 7. What we retire

Things currently in the codebase that must be removed or deprecated when this lands.

1. **4-char-per-token heuristic** (`token_counter.rs:17-24`). Every eviction currently fires at the wrong threshold. Gone in Phase 1.
2. **`summarization.rs`'s 8k trigger contract**. It fires regardless of what `context_editing` did. Replaced in Phase 2 by "only if Layer 2 couldn't free enough."
3. **Recursive self-summarization**. No code path should ever summarize a message whose content starts with the summary prefix. Enforce with a message flag, not a string sniff.
4. **`sleep/compactor.rs`'s role as a forgetting policy over live state**. It stays, scoped to durable KG hygiene; runtime owns live-tape forgetting. This removes the "two uncoordinated compactors" failure mode and specifically the `defect_ward_routing_fragmentation` bug pattern.
5. **Cosine-only entity merging** (`compactor.rs`). The normal service path wires `PairwiseVerifier`; verifier-enabled compaction must skip merges when adjudication cannot run.
6. **Any replacement durable memory surface**. `knowledge.db` is the source of truth; markdown files may be documentation or export artifacts, not primary memory.
7. **Implicit "summarize before clearing" middleware order**. Enforced in `pipeline.rs` setup, not by convention: `ContextEditing → PlanBlock → Summarization`.

---

## 8. Anti-patterns we refuse

Codified so future contributors can point at the doc instead of rediscovering:

1. **No char-per-token heuristics for budget decisions.** Real tokenizer or no decision.
2. **No unsupervised self-summarization.** Summarization must have a bounded schema or be ruthlessly restricted to prose tails.
3. **No two forgetting policies over the same tape.** Runtime middleware owns live state; sleep cycle owns durable `knowledge.db` hygiene. The boundary is load-bearing.
4. **No cosine-only entity merging.** Add a type+name guard, require LLM-judge confirmation above a similarity threshold, or accept slightly more duplicate entities. False merges are more expensive than fragmentation.
5. **No orphaning of `tool_use` / `tool_result` pairs.** Eviction and summarization must preserve atomic groups.
6. **No compaction of the stable prefix.** Layer 0 is immutable for the session.
7. **No semantic-similarity-only retrieval of history.** Use recency + task-relevance signals together.
8. **No summarizing tool output that is deterministically re-fetchable.** Clear it (free) instead of summarizing (lossy API call).

---

## 9. The single load-bearing architectural decision

**Who owns the forgetting policy for the live conversation tape — runtime middleware or the sleep cycle?**

The research converges unanimously on runtime. Two independent compactors with different thresholds, different notions of "stale," and different transactional envelopes is exactly the anti-pattern both surveys flagged, and it's the root cause of the ward-routing-fragmentation defect we have today.

### The commitment
**Runtime middleware owns all forgetting over the live conversation tape. The sleep cycle owns hygiene over durable `knowledge.db` memory only, and never reaches into an active session.**

This is irreversible. Layer 1 (plan block) is what survives runtime forgetting. Layer 4 (`knowledge.db`) is what survives session boundaries. Sleep-cycle operations on live tape would violate both contracts. If we pick the other shape (sleep cycle as authoritative), Layer 1 has no meaning and Layer 4 has to fight the sleep worker for write ordering.

Every later engineering decision in this document assumes this commitment. Make the call before writing a line of code.

---

## 10. Research references

### Anthropic (authoritative)
- [Effective context engineering for AI agents](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)
- [Effective harnesses for long-running agents](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents)
- [Anthropic cookbook — tool-use context engineering](https://platform.claude.com/cookbook/tool-use-context-engineering-context-engineering-tools)
- [Memory tool docs](https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool)
- [Context editing docs](https://platform.claude.com/docs/en/build-with-claude/context-editing)
- [Prompt caching docs](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)
- [How Claude Code works — `/compact`](https://code.claude.com/docs/en/how-claude-code-works.md)

### Production systems
- [Cognition — Don't Build Multi-Agents](https://cognition.ai/blog/dont-build-multi-agents)
- [Cursor — Dynamic context discovery](https://cursor.com/blog/dynamic-context-discovery)
- [LangChain — Replit Agent case study](https://www.langchain.com/breakoutagents/replit)
- [Letta — Memory tiers (MemGPT)](https://docs.letta.com/guides/legacy/memgpt_agents_legacy)
- [Letta — Stateful agents memory guide](https://docs.letta.com/guides/agents/memory/)

### Research results
- [Chroma — Context Rot (2025)](https://research.trychroma.com/context-rot)
- [StreamingLLM / attention sinks — ICLR 2024](https://arxiv.org/abs/2309.17453)
- [LLMLingua-2 — ACL 2024 Findings](https://arxiv.org/abs/2403.12968)
- [Provence — ICLR 2025](https://proceedings.iclr.cc/paper_files/paper/2025/file/5e956fef0946dc1e39760f94b78045fe-Paper-Conference.pdf)
- [xRAG — NeurIPS 2024](https://proceedings.neurips.cc/paper_files/paper/2024/file/c5cf13bfd3762821ef7607e63ee90075-Paper-Conference.pdf)
- [OSCAR — ICLR 2025](https://openreview.net/pdf?id=ideKAUWvFE)

### Known failure modes
- [openclaw#7527 — orphaned tool_result blocks after compaction](https://github.com/openclaw/openclaw/issues/7527)
- [Claude Prompt Caching in 2026: The 5-Minute TTL Change](https://dev.to/whoffagents/claude-prompt-caching-in-2026-the-5-minute-ttl-change-thats-costing-you-money-4363)
- [memu.pro — Windsurf session-memory limitation](https://memu.pro/blog/windsurf-ide-ai-coding-agent-memory)

### Internal context (AgentZero)
- `docs/adr/decisions.md` — architectural decisions log
- `docs/architecture/defects/` — ward-routing-fragmentation defect (root cause of the dual-compactor failure)
- `docs/architecture/architecture.md` — system context for where this plan lands
