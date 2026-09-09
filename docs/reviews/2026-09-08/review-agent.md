# Agent-Capability Review — zbot on `op_clean_crap`

**Scope:** What would make zbot a *better agent* (capability), not code quality.
**Method:** Read the actual retrieval, tool, context, planning, learning, delegation, and recovery code.

## What's already strong (don't rebuild)

- Hybrid retrieval: FTS + embeddings, RRF merge, MMR diversity rerank, Self-RAG query gate, category priority (corrections > strategies > preferences) — `gateway/gateway-memory/src/recall/mod.rs`
- Procedures with **concrete tool args** (`ProcedureStep.args`) — distillation produces executable learned sequences
- Strategy emergence: failed episodes cluster into strategy facts (`try_cluster_failures`, threshold 0.6)
- Steering: mid-run injection with User/System/Parent/Peer sources + priorities (`runtime/agent-runtime/src/steering.rs`)
- Checkpoints: versioned, latest-wins, O(1) restore (`stores/zbot-conversation/src/checkpoints.rs`)
- Stuck detection, turn budgets by complexity, context-full "save facts" warning (`rig_adapter/progress_policy.rs`)
- Context offload of large tool results to files (`agent-tools/src/tools/mod.rs` ToolSettings)
- Episode chains: prior successful sessions injected at start (`recall/previous_episodes.rs`)

---

## Improvements, ranked by impact/effort

### 1. Inject *failed* episodes as an avoid-list at session start — S / High

**What:** `PreviousEpisodesAdapter::fetch` calls `fetch_recent_successful_by_ward` only. Add a sibling fetch of the 3 most recent *failed* episodes in the ward and inject them as items tagged `[FAILED]` with `key_learnings`.

**Why:** The agent restarts cold into approaches that already failed in that ward. Distillation already clusters failures into strategies (`try_cluster_failures`), but nothing surfaces raw failure context at session start. `SessionEpisode` has `outcome: failed|crashed` and `key_learnings` fields sitting unused at recall time.

**How:** New `fetch_recent_failed_by_ward(3)` on `EpisodeStore` trait (default: empty). In `previous_episodes.rs`, emit items with content `[FAILED, date] summary
Learnings: ...` and a `Provenance` marking them as avoid-hints. RRF handles ranking. ~120 lines + trait default.

### 2. Recency decay + usage reinforcement in fact scoring — S / Med-High

**What:** Multiply recall score by `exp(-λ * age)` from `updated_at`; increment an `access_count` (and stamp `last_accessed`) whenever a fact is returned; add a small boost proportional to log(access_count).

**Why:** Facts are stored with `updated_at` (`stores/zbot-engram-adapter/src/stores/memory_facts.rs:343`) but the hybrid search (`SearchRequest` → sidecar) never uses it. A 6-month-old stale fact outranks a fresh correction whenever its text matches slightly better. No forgetting, no reinforcement — memory quality degrades monotonically with size.

**How:** In the sidecar `SearchRequest` scoring path: add `recency_factor = (-age_days / 30.0).exp().max(0.25)` (floor so old-but-relevant still surfaces) and boost `1 + (access_count as f64 / 10).min(0.5)`. Write-back of access_count goes in `recall_facts*` after rows are selected. Two crates: engram-adapter sidecar + trait for the write-back. ~150 lines.

### 3. `WriteSkillTool` — the agent writes its own skills — S/M / High

**What:** A tool that persists a skill markdown file to the agent's skills dir after validation. Skilled agent sessions become reusable `load_skill` targets without the user copying files by hand.

**Why:** zbot can *load* skills (`LoadSkillTool`) but cannot *create* them — the only write path is the HTTP API (`gateway/src/http/skills.rs:76`). This is the single biggest missing learning loop: successful sessions distill facts/procedures, but the highest-leverage artifact (a skill teaching future sessions the whole approach) requires out-of-band action. Compare: procedures capture tool-call sequences, skills capture domain methodology — they're complementary and procedures already exist.

**How:** In `runtime/agent-tools/src/tools/execution/skills.rs` alongside `LoadSkillTool` (which already knows `skills_dirs()` resolution): validate name (kebab-case, no traversal), require `description:` frontmatter, cap size, write `<name>.md`. Register in the same place `LoadSkillTool` is registered, gated on a permission. ~200 lines. (Optionally later: distillation proposes skill candidates in its LLM schema — a `candidate_skill` field — and the user approves via UI.)

### 4. Structured failure feedback on repeated tool errors — S / High

**What:** When a tool call fails twice with the same `(name, args_hash)`, inject a system message listing the exact prior errors and demanding a changed approach; on the third, block the identical call via `before_tool`.

**Why:** `ProgressTracker.recent_tool_calls` already hashes args for repeat detection, and `recent_errors` stores messages — but they only feed the coarse `is_clearly_stuck()` nudge ("step back and try a different approach"), which fires once. The model gets no *content*: which errors, from which calls, what to change. Identical failing calls keep burning turns until the score hits −12 and `MaxIterationsNeedsIntervention` kills the run.

**How:** In `rig_adapter/progress_policy.rs` prep turn-policy: match `recent_errors` against last-N calls by args hash; on 2nd occurrence push `[SYSTEM: {tool}({args}) failed twice — errors: {e1}; {e2}. Change the arguments or use a different tool.]`; in `HookSet.before_tool` add a guard that returns `ToolDecision::Block` on the 3rd identical failure. ~100 lines, all in files that already exist.

### 5. Web search tool — M / High

**What:** A `web_search` tool backed by a pluggable provider (SearxNG local / Tavily / Brave key), plus optional `fetch_url` (respecting robots, size caps, markdown conversion).

**Why:** zbot has *no internet access* — tools are: memory/recall/read/write/edit/glob/shell/load_skill/update_plan/delegate/respond/run_procedure/graph_query/ingest/goal/ward/multimodal/connectors. `connectors` requires per-source manual configuration; there is no zero-config path to fresh information. Every comparable agent (ChatGPT, Claude, Gemini agents) treats web search as table stakes; for a research-oriented agent (wards, research briefs are "ALWAYS graph" in the intent prompt) this is the largest raw capability gap.

**How:** New `runtime/agent-tools/src/tools/web.rs`: `WebSearchTool` + `FetchUrlTool` behind a `ToolSettings.web_tools` flag (mirroring `file_tools`); provider trait + SearxNG impl (self-hosted, no key) and Tavily impl (key). Reuse the offload-large-results mechanism for big pages. ~600 lines + config UI. Wire into the intent prompt's resource list so the agent knows it can search.

### 6. Mid-run re-planning on plan-step failure — M / Med-High

**What:** Track failures per active plan step (from `update_plan` state); when the step's tool calls accumulate ≥2 errors, inject a nudge: `update_plan` to mark the step blocked and revise remaining steps.

**Why:** `PlanBlockMiddleware` pins the plan in context and `ProgressTracker` tracks `plan_items_completed` — but the two never meet. A failed step stays `in_progress` forever while the agent flails; the plan block keeps asserting a stale strategy. The intent agent's `solution_path` is a first draft; nothing revises it when reality disagrees.

**How:** `ProgressTracker` gains `step_errors: HashMap<String, u32>`; the turn-policy pushes the revise nudge at threshold; `PlanBlockMiddleware` renders blocked steps from `update_plan` state (extend its state schema with `blocked`). ~200 lines across `progress.rs`, `progress_policy.rs`, `middleware/plan_block.rs`.

### 7. Parallel delegated children — M / Med

**What:** Fan out independent subagents concurrently (`FuturesUnordered`), reusing the existing per-session semaphore for capacity.

**Why:** `DelegationContext.parallel: bool` exists but `spawn.rs:2079` and `spawn.rs:2282` hardcode `parallel: false`. Research-shaped work (the graph path) is inherently parallelizable across sources; today every child serializes. The infrastructure (registry, result bus, permits) is present — the flag is simply never true.

**How:** Accept `parallel` from the delegation request (it's already plumbed through `DelegationContext`); in the spawn path, when the parent's message contains multiple `delegate_to_agent` calls in one turn, the `ToolExecutionMode::Parallel` default in `engine/hooks.rs` already executes tools concurrently — verify child spawn is safe under it (semaphore + `agent_result_bus` are already concurrency-safe per their docs). Expose a `parallel: true` option on the delegate tool schema. ~150 lines, mostly verification + schema.

### 8. Critic pass before `respond` — L / Med

**What:** For L/XL tasks, spawn a reviewer agent over the final artifact + plan before the root responds; surface its verdict; one revision round.

**Why:** There is no self-eval anywhere: the intent agent plans, workers produce, the root `respond`s. Distillation judges quality only post-session (`outcome` label). The steering queue and delegation machinery could host a critic without new primitives, but it needs prompt design, budget accounting, and UI surfacing to be worth it.

**How:** Hook on the *first* `respond` call for L/XL complexity: block, spawn critic child with the artifact + original intent, inject its critique as steering, allow the second `respond`. ~400 lines in gateway-execution delegation + a critic prompt. Do after 1–6.

### 9. Hierarchical summarization — L / Med

**What:** Summary-of-summaries: when a session's summary itself exceeds budget, summarize prior summaries into a level-2 digest, keeping level-1 windows.

**Why:** `SummarizationMiddleware` is a single whole-history pass with keep-last-N (20 chat / 30 deep messages). Very long sessions progressively lose early detail with no structure; the first summary eats everything older each trigger. Fine at current scale, degrades for day-scale sessions.

**How:** Track `is_summary` messages (already flagged — the plan block uses `is_summary = true` to stay out of passes); on trigger, if ≥2 summary messages exist, summarize those into one level-2 message. ~250 lines in `middleware/summarization.rs`.

---

## Top 5 (do these first)

1. **Failed-episode avoid-list** (S/High) — instant payoff, code half-exists
2. **Repeated-failure structured feedback + block** (S/High) — stops the biggest token burner
3. **Recency + usage scoring** (S/Med-High) — memory gets better as it ages instead of worse
4. **`WriteSkillTool`** (S-M/High) — closes the strongest learning loop
5. **Web search** (M/High) — largest raw capability gap for a research agent

Items 1–4 are pure Rust in existing modules with existing patterns; no new crates, no schema migrations except `access_count` (nullable, default 0).
