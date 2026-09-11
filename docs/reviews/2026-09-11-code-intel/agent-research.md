# Autonomous-Agent Research → zbot Improvement Plan

Date: 2026-09-10 · Grounded in the develop tree (post recall-quality arc,
gateway decompose, tool diet). Every zbot claim cites a file.

---

## 1. Planning & long-horizon execution

**Field.** Plan-then-execute (HuggingGPT/LLM+P lineage) beats reactive ReAct
loops on long-horizon tasks, but the consensus from SWE-agent and the agent
harness literature is *adaptive replanning*: plan as a living artifact,
verify each step's exit condition, and re-plan when the world invalidates
the plan — not upfront waterfall. Voyager's skill library shows plans
compounding across sessions when successful executions become reusable units.

**zbot has.** `update_plan` tool with planning-gate enforcement
(`runtime/agent-tools/src/tools/guards.rs:41` — `active_planning_gate`
blocks cold graph work pre-ward); complexity-budgeted iterations
(`progress_policy.rs:71` — S/M/L/XL budgets with soft/hard steer); plans
persist per session (`session_plan_counters`); procedures = cross-session
plan reuse (`run_procedure` with step interpolation,
`runtime/agent-runtime/src/tools/run_procedure.rs`); plan items feed the
progress score (`progress.rs` — completed steps add +2 each).

**Gap (the one).** **No step-level exit verification and no in-flight
replanning.** The plan is tracked (done/pending) but never *checked*: a step
can be marked complete with a wrong/empty output, and plan divergence (the
model wandering off-plan) only surfaces via the score, late. SWE-agent's
core win was exactly per-step verification before advancing.

**Effort.** M — an `EngineHook` after-tool check comparing the active plan
step's declared output against the artifact/answer produced (hooks framework
already exists: `runtime/agent-runtime/src/engine/hooks.rs`).

---

## 2. Self-correction loops

**Field.** Reflexion (Shinn et al.): verbal self-critique stored as episodic
memory measurably improves retry success; the effect compounds when the
critique is *retrieved at the next attempt*, not just emitted. SWE-agent's
"agent-computer interface" work shows error-message design itself is a
first-class lever (they recovered ~10% resolve rate by rewriting tool error
text).

**zbot has.** Structured failure feedback — repeated identical failing calls
get named-error nudges (`progress_policy.rs` — `top_failing_call` + once-per-
call re-nudge); failed-episode avoid-list at session start
(`previous_episodes.rs` — `[AVOID]` items with learnings); after-tool failure
guidance injected by `SubagentGuardHook` (`invoke/policy.rs:249` — "read the
error, fix the ROOT CAUSE"); runaway-builder guard on same-path rewrites
(`progress.rs` — `write_target_repeats`); strategy emergence from failure
clustering (distillation).

**Gap.** **The correction doesn't persist within the session as first-class
memory.** Failure feedback is transient (nudges) or cross-session (avoid-list);
there is no in-session reflexion store — the model that failed twice and
self-corrected at turn 5 gets no boost against re-failing at turn 25 on a
similar subtask. The avoid-list fix landed the cross-session half.

**Effort.** S — on `top_failing_call` discharge (a success after nudges),
write a `correction`-class fact via `memory_write`'s path, tagged to the
session; recall already ranks corrections highest (golden set: 5/5).

*Critic-pass note:* the backlog rejected pre-respond critic passes once
(L-effort, unclear payoff). Evidence since: your two live-session bugs were
both *call-shape* errors a critic would have caught — but both were also
fixed at the surface (contract in recall; schema validation). Hold the
rejection; revisit only if task-level evals (§7) show answer-shape failures.

---

## 3. Memory

**Field.** Generative Agents (Park et al.): retrieval = recency × importance
× relevance — all three terms matter; importance is LLM-scored at write
time. MemGPT/Letta: tiered memory (working/archival) with self-editing.
Mem0: write-time ADD/UPDATE/DELETE consolidation. ACT-R: activation from
frequency+recency of *accesses*.

**zbot has.** Relevance × recency × usage — engram weighted RRF with a
temporal lane (`stores/zbot-engram-adapter/src/stores/retrieval_composition.rs`),
per-category half-lives, `touch_facts` access reinforcement
(`recall/mod.rs` — ACT-R term), supersession/dedup at write (distillation),
procedures with success/failure counters surfaced in recall
(`adapters.rs:141` — Parameters + Track record). Measured: 76.7% precision@5,
corrections 5/5 (`gateway/gateway-memory/tests/golden_recall/BASELINE.md`).
Consolidation: engram port slots (`sleep/belief_engram.rs`,
`hierarchy_engram.rs`).

**Gap.** **Importance — the third Generative-Agents term — is absent**
(grep-verified: no importance scoring in gateway-memory or distillation).
Every fact enters at parity; only usage/recency differentiate afterward.
A one-shot user correction and a throwaway observation decay identically.

**Effort.** S — distillation already asks the LLM per-fact; add a 1–5
importance field to the extraction schema (`services/distillation/src/lib.rs`
`DistillationResponse`), persist on the fact, multiply into the temporal
lane. The golden set measures the win directly.

---

## 4. Context engineering

**Field.** The harness literature (Claude Code, Cline, SWE-agent ablations)
converges on: budget-aware assembly > truncation; structured offload beats
lossy compaction; tool-result size is the dominant context killer.

**zbot has.** Context-editing middleware
(`runtime/agent-runtime/src/middleware/context_editing.rs`), offload-large-
results, budget-reject on prompt growth (`progress_policy` input_budget +
context-full steer to save facts), context-packet lanes with resource
handles (`gateway-execution/src/recall/mod.rs` — budget + omitted-result
explanations), token hints per tool (`invoke/tool_catalog.rs`).

**Gap.** **No measured context telemetry loop** — budgets are configured,
not learned. The catalog carries token_hint estimates; nothing reconciles
them against actual usage to retune assembly. Secondary: no cross-session
working-memory carry (each session rebuilds context from durable memory
only).

**Effort.** M — the RecallTrace event already carries surfaced counts;
extend with actual prompt-token accounting per lane, log to the recall_log,
retune quarterly. Low glamour, compounding returns.

---

## 5. Tool use

**Field.** Tool-count vs depth: the function-calling literature and harness
practice say ~10–20 well-described tools is the sweet spot for small/local
models; error-feedback design is as important as tool design (SWE-agent's
ACI finding). Schema narrowness beats flexible blobs for reliability.

**zbot has.** The evidence-driven diet just landed (glob/memory/
graph_query/query_resource deleted on 0-usage; 18-call surface aligned to
usage histogram); args-validation with actionable errors
(`run_procedure` "requires args: user_id, journal_path" — observed self-
corrected in one turn); call contracts surfaced in recall (procedure
Parameters + track record).

**Gap.** **`lookup_capabilities` is a tool, not a property of context** —
25 calls in traces means the model asks "what can I do?" mid-task. The
catalog (tool_catalog) already knows; a static capability preamble (or
skill-index recall at bootstrap) would delete those turns entirely.

**Effort.** S — compose the catalog's per-actor inventory into the system
prompt assembly (`gateway-templates` prompt build), one place.

---

## 6. Harness patterns

**Field.** Subagent isolation with narrow tool grants is the established
pattern (Claude Code subagents, AutoGen roles); parallel fan-out with join
is table stakes for research-shaped tasks; steering/checkpointing define
the recovery envelope.

**zbot has.** Actor capability policy (`invoke/policy.rs` — per-actor tool
grants), delegated-executor shell-write block, versioned checkpoints
(`stores/zbot-conversation/src/checkpoints.rs`), steering queue, ward cwd
isolation, per-ward serialization locks (`delegation_dispatcher.rs:189`).

**Gap.** **Parallel children are declared but never read.**
`DelegationRequest.parallel` exists (`delegation/context.rs:175`), rides in
events (`agent-primitives/src/event.rs:253`), and `spawn.rs` never branches
on it (grep-verified: zero production reads). Live sessions fake it by
issuing sequential delegate calls and letting async dispatch overlap —
which works (sess-d6737135 ran two "parallel" streams) but the model has no
join primitive (`wait_agent` is hidden) and no true fan-out semantics.

**Effort.** M — branch on `parallel` in the dispatcher (concurrent spawn is
already safe under the per-ward locks; the join is the continuation logic
that exists for sequential children). Gate with task-level evals (§7).

---

## 7. Evaluation

**Field.** The serious agent teams evaluate the *loop*, not just components:
task-level golden runs (SWE-bench style: fixed task, verified end state),
regression harnesses on the harness itself.

**zbot has.** Golden recall set — 30 labeled cases, floors + precision@5,
measures the production path (`gateway/gateway-memory/tests/golden_recall/`).
Golden traces for execution (3-flow behavior oracle, replay-normalized).

**Gap.** **No task-level agent-loop evaluation.** Recall is measured; the
loop that *uses* recall is not. The two live-session bugs this week (ward
race, procedure contract) were found by reading traces, not by any harness
— a 10-task golden run (ward→plan→delegate→artifact, assert end state)
would have caught both.

**Effort.** M — 10 scripted tasks against the daemon (the durable-task
path exists), asserting artifacts + no-orphaned-executions + budget bounds.
This is the enabling gate for §6 parallel and §2 critic decisions.

---

## TOP-5 ranked (impact × 1/effort)

1. **Task-level golden runs** (§7, M) — enables every other bet; would have
   caught both live bugs this week. Shape: `e2e/golden_tasks/` — 10 YAML
   tasks, daemon-level, assert artifacts + lifecycle invariants.
2. **Importance scoring** (§3, S) — completes the Generative-Agents triple;
   one schema field + one multiplier into the temporal lane; the golden
   recall set measures it immediately.
3. **Capability preamble** (§5, S) — deletes the lookup_capabilities turns
   (25 in traces); compose tool_catalog's per-actor inventory into prompt
   assembly.
4. **In-session reflexion store** (§2, S) — discharge of a failing-call
   nudge writes a session-scoped correction fact; recall already ranks
   corrections top. Closes the turn-5→turn-25 self-correction gap.
5. **Parallel delegation wired** (§6, M) — read the existing flag in the
   dispatcher, add join semantics; gate on #1's harness. Fan-out is the
   biggest visible capability jump for research-shaped tasks.

**Explicitly not now**: pre-respond critic passes (rejection stands pending
#1's evidence), learned rerankers (76.7% headroom not yet worth a serving
path), cross-session working memory (§4 secondary).

**Uncertainty note**: literature claims cited from memory (Reflexion, SWE-
agent ACI, Generative Agents, MemGPT, Mem0, ACT-R, Voyager) — directions
are solid, specific numbers (e.g., "~10% resolve rate") should be
re-verified before being quoted externally.
