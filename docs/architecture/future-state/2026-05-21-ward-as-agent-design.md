# Ward-as-Agent — Design & Lessons Learned

**Date:** 2026-05-21
**Branch:** `ward-as-agent`
**Status:** Design — not yet implemented
**Supersedes:** the `ward-as-skill` approach on the parked `feat/ward-as-skill` branch

## 1. Premise

A ward is **already an agent** — it has a persona file (`AGENTS.md`), a
memory-bank, accumulated procedures and recall, and a persistent directory.
Two things are missing, and ward-as-agent closes exactly those:

- **Gap A — a ward is never executed as an agent.** Today you "enter" a ward
  (`ward(action=use)`) and the *root* does the work. There is no path where a
  ward runs as its own subagent in its own context.
- **Gap B — `AGENTS.md` is not a good subagent system prompt.** It is a loose
  readme, not generic doctrine shaped to drive a plan+execute agent.

Everything else (directory, memory-bank, recall scoping, procedures) exists.

## 2. Lessons learned from ward-as-skill

`feat/ward-as-skill` tried to make wards skill-like by pinning the ward's
`AGENTS.md` into the root's context every turn. Why that was the wrong shape:

1. **It inverted the skill model.** A real skill is progressive disclosure —
   name+description resident, body loaded on demand. ward-as-skill pinned the
   entire ~8 KB `AGENTS.md` body into a system message *every turn*, flagged
   `is_summary=true` so compaction could never reclaim it. A permanent
   preamble, not a skill.
2. **The 8 KB truncation fought the roadmap.** The goal was to *enrich*
   `AGENTS.md`; ward-as-skill silently truncated it at 8000 bytes.
3. **It treated the symptom, not the disease.** The real pain — the agent
   serially reading multiple spec files — was untouched; `AGENTS.md` just
   *points at* those files, so the agent still read them.
4. **It was an unmergeable bundle.** 38 commits / 5777 lines mixing
   ward-as-skill, the procedures subsystem, KG cleanup, intent routing, and an
   `AGENTS.md`→`ZBOT.md` rename — five features in one branch.
5. **The `ZBOT.md` rename was gratuitous churn.** `AGENTS.md` is the
   convention.

**Carry-over still good:** the procedures subsystem (`run_procedure`, dedup,
3-tier recommendation) — already merged via PR #185. Phase 3a
`synthesize_ward_agent` is a useful sketch, not a finished design.

**Explicitly dropped:** the per-turn `AGENTS.md` pin
(`inject_ward_skill_block` / `app:active_ward_skill`), the 8 KB cap, the
`ZBOT.md` rename, and any inlining of specs into a prompt.

## 3. The model — wards are agents, reached via `delegate_to_agent`

No `run_ward` tool. A ward appears in the `delegate_to_agent` roster next to
`planner`; the root picks one the way it picks any agent.

**Warm path** — ward exists AND is exposed as a capable agent:
`delegate_to_agent("ward:<name>", task, wait_for_result=true)` → the ward-agent
runs a full **plan + execute** loop internally and returns a finished result.
One call from the root; the root never orchestrates the ward's internal steps.

**Cold path** — no ward, or the ward lacks the capability for this task:
root → `planner` → `{builder | writer | researcher | …}` loop → runs to
completion. The existing generic orchestration. It runs inside a
`ward(create)`'d directory; distillation populates the ward's `AGENTS.md`
doctrine + memory-bank + procedures. The ward **graduates** into an exposed
agent once it has accumulated real capability.

One mechanism, two roles: the generic planner loop is both the fallback **and**
how wards get built. A ward *earns* agent-hood — never synthesized empty.

```
task
 │
 ├─ exposed ward covers this capability?
 │     YES → delegate_to_agent("ward:X", wait_for_result=true)   [warm: one call]
 │            └─ ward returns capability_missing? → fall through ↓
 │     NO  ↓
 ▼
 cold: ward(create) → planner → {builder|writer|researcher|…} loop, till done
        └─ distillation populates the ward → it graduates → exposed next time
```

### 3.1 Sequence — existing ward (warm path)

```
User → Root      request
Root → list_agents → sees ward:X with its scope blurb
Root → delegate_to_agent("ward:X", task, wait_for_result=true)
        ▼ delegate.rs → DelegationDispatcher → spawn_delegated_agent
        ▼ load_or_create_specialist("ward:X") → synthesize_ward_agent
             compose system prompt L1–L5 ; cwd = wards/X/ ; steering handle
        ▼ WARD-AGENT (one delegation, internal loop):
             ward(use) → recall → capability survey → PLAN → EXECUTE → respond
        ▼ AgentResultBus.resolve {done | out_of_scope | capability_missing}
Root ← result   (delegate_to_agent was blocked on wait_for_result)
```

### 3.2 Sequence — new ward (cold path)

```
User → Root      request
Root → list_agents → no ward covers this
Root → ward(create="X") → scaffold wards/X/ : AGENTS.md (seed) + docs/
Root → delegate_to_agent("planner", task) → plan
Root    loop steps → delegate_to_agent("builder"/"writer"/"researcher", …) → till done
Root → User      result
        ▼ session ends → DISTILLATION writes wards/X/ :
             AGENTS.md doctrine · procedures · memory-bank facts
        ▼ GRADUATION CHECK passes → ward:X exposed in list_agents next time
```

## 4. Gap A — making a ward execute (warm-path call graph)

The warm path reuses the existing delegation pipeline; it forks at one point.

```
delegate_to_agent("ward:X", task, wait_for_result=true)
  → delegate.rs:31           sets a DelegateAction in the action queue
  → handle_delegation()      [invoke/delegation_handler.rs]  child exec QUEUED
  → DelegationDispatcher     [runner/delegation_dispatcher.rs]
  → spawn_delegated_agent    [delegation/spawn.rs:48]
       ├─ load_or_create_specialist("ward:X")
       │     └─ "ward:" prefix → synthesize_ward_agent("X")  [invoke/setup.rs]
       │           · ward must already exist + be exposed (warm path only)
       │           · compose system prompt (§5)
       ├─ ExecutorBuilder    cwd = ward dir, full capability inventory (§7)
       ├─ register steering handle
       └─ prime ward-scoped memory recall
  → ward-agent loop:  ward(use) → recall → plan → execute → respond(handoff)
  → AgentResultBus.resolve / .reject   [agent_pool/result_bus.rs]
```

**To build:**
- `ward:` agent-id prefix handled in `load_or_create_specialist` →
  `synthesize_ward_agent`.
- Ward-agent runs with cwd = ward dir, full capability inventory.
- **No create-on-miss here.** `delegate_to_agent("ward:X")` only fires for an
  already-exposed ward. Creation is the cold path's job.
- Return states: `done`, `out_of_scope` (wrong domain), `capability_missing`
  (right ward, missing skill → cold path extends it). Ride the existing
  `.reject()` path.
- Ward delegations use `wait_for_result=true` so the single call blocks until
  plan+execute completes; the root stays steerable via the registered handle.

## 5. Gap B — `AGENTS.md` as a subagent system prompt

The ward-agent's system prompt is assembled in layers — a framework scaffold
(identical for every ward) plus the ward's own `AGENTS.md` (the only per-ward
file; no `ZBOT.md`).

| Layer | Origin | Content |
|---|---|---|
| L1 Identity & contract | framework | "You are the `<ward>` ward-agent. One task in — you plan *and* execute it to completion in this single run." |
| L2 Scope contract | `AGENTS.md ## Purpose` | Domain in/out of scope. Outside → return `out_of_scope`. |
| L3 First-turn protocol | framework | `ward(use)` → recall → capability survey → plan → execute → respond(handoff) |
| L4 Ward doctrine | `AGENTS.md` body | Folder map · standards · tools · failure modes · handoff. |
| L5 System-context shards | existing `append_system_context` | OS · memory-learning · session-ctx |

**`AGENTS.md` stays GENERIC.** It is domain-level doctrine, not operational
minutiae. A `financial-analysis` ward's scope is "equity & options analysis,
financial prediction" — not "VaR at 95% over 252 days" or "use yfinance."
Standards are generic principles ("cite the source," "state confidence"), not
formats or thresholds.

**The line — where each kind of knowledge lives:**
- **Generic → `AGENTS.md` (the system prompt):** domain scope, role,
  high-level principles, handoff schema. Stable, ~1 KB, same shape every ward.
- **Specific → memory-bank + procedures (recalled, never in the prompt):**
  data sources, formulas, thresholds, conventions, the actual methods.
  Learned, grows, pulled in by recall at plan time.

This permanently kills the `AGENTS.md`-growth problem: it stays small *by
design* because it only holds generic doctrine; all enrichment flows to
docs/procedures, which are recalled, never pinned. A ward's
intelligence = **generic prompt + accumulated memory**. A fresh ward behaves
like a generic domain agent; a mature one behaves like an expert — same
prompt, the difference is entirely recalled memory.

Canonical `AGENTS.md` (generic example):

```markdown
# financial-analysis

## Purpose / Scope
IN  — equity & options analysis, financial prediction and forecasting,
      portfolio and risk assessment.
OUT — accounting, tax, bookkeeping, M&A legal, personal budgeting.
      If a task falls outside IN → return out_of_scope.

## Folder map
- specs/  per-task specs   - core/  reusable analysis functions
- data/   cached data      - docs/  ward.md · structure.md · core_docs.md
Read on demand — recall first, open a file only when recall points at it.

## Standards
- Cite the data source and as-of date for every number.
- State assumptions and confidence; never present a prediction as fact.

## Tools & delegation
- Recall the ward's procedures before planning from scratch.
- Delegate net-new code modules to `builder`; reuse core/ first.

## Failure modes & hard don'ts
- Don't fabricate data — if a source is unavailable, say so.
- Don't expand beyond IN scope; surface it as out_of_scope.

## Handoff
Return: { status, summary, findings, confidence, artifacts:[paths] }
```

## 6. Reasoning & planning

The ward-agent is **ReAct (loop) + Plan-and-Execute (front) + self-sourced
few-shot** — three layers, not a single choice.

- **Loop = ReAct.** A tool-using agent: Thought → Action → Observation. Not a
  choice — it is what the execute phase is.
- **Front = Plan-and-Execute.** Before the ReAct loop, one explicit planning
  turn produces a written, steerable plan. Beats pure step-by-step ReAct: a
  whole procedure can drop in as the plan, and it stops ReAct "drift."
- **Few-shot = dynamic, from the ward's own procedures.** Never static
  in-prompt examples (token cost, staleness). The ward's promoted procedures —
  captured successful traces — are recalled at plan time and act as exemplars.

**Planning a new request** — first-turn protocol:

```
1. ward(use)            land in the ward dir
2. recall(task)         ward-scoped: procedures (3-tier), facts, episodes
3. capability survey    analyze_intent surfaces relevant skills; tool+MCP inventory known
4. PLAN ── graduated ──┐
     promoted procedure matches → plan = run_procedure(it)        [replay]
     partial match (advisory)   → adapt the procedure into steps  [scaffold]
     no match                   → CoT decompose into steps        [from scratch]
5. EXECUTE (ReAct per step)
6. respond(handoff)
```

The plan is an explicit artifact, emitted before execution (visible,
steerable), and **every step is bound to a concrete capability** (tool / skill
/ MCP tool / sub-delegation). On a miss, the successful run distills into a new
procedure — so the ward few-shots itself from its own history and gets faster
and more deterministic with use.

## 7. Capability inventory

The ward-agent gets the **complete capability surface** — not a configured
subset:

- **Tools** — the full built-in registry. Always available.
- **Skills** — *all* skills, discovered and loaded on demand via
  `analyze_intent(auto_load=true)` (progressive disclosure — no context bloat).
- **MCPs** — every configured MCP server; MCP tools surfaced when planning
  indicates need.

**A ward is bounded by its scope contract, not by a tool whitelist.**
Restricting tools would cause false `capability_missing` failures; the L2
scope contract handles the boundary instead. Capability is broad; *behaviour*
is bounded by doctrine. A mature ward recalls which tools/skills/MCPs it
actually uses (encoded in its procedures), so planning gets faster — but
access is always full.

## 8. Graduation gate — when a ward becomes exposed

Existence ≠ exposure. Proposed gate to appear in `list_agents` / be a valid
`delegate_to_agent` target:
- non-stub `AGENTS.md` (Purpose/scope written), **and**
- ≥ 1 promoted procedure (one proven, reusable capability).

`list_agents` must surface each exposed ward **with its scope blurb**, so the
root routes into an existing ward instead of minting a near-duplicate.

## 9. Open decisions

1. **Graduation gate threshold** — §8 is a proposal.
2. **Capability detection — pre-flight vs in-flight.** Pre-flight from the
   scope blurb in `list_agents` + `capability_missing` as the in-flight safety
   net. Recommend both.
3. **Ward-agent model/provider config** — inherit the root's by default;
   optional `wards/<ward>/config.yaml` override later.
4. **Anti-fragmentation** — semantic match against existing wards before the
   cold path creates a new one (known defect: `maritime-tracking` vs
   `maritime-vessel-tracking`).
5. **Concurrency** — one ward, multiple simultaneous delegations: who owns
   writes to its memory-bank.

## 10. Implementation phasing

- **P1 — execution path:** `ward:` prefix → `synthesize_ward_agent`;
  ward-agent runs with cwd = ward dir, full capability inventory;
  `wait_for_result` contract. (Gap A)
- **P2 — system prompt:** the L1–L5 assembly; canonical generic `AGENTS.md`
  template; the graduated first-turn protocol. (Gap B + §6)
- **P3 — routing:** `list_agents` exposes wards with scope blurbs; the
  graduation gate.
- **P4 — return states:** `out_of_scope` / `capability_missing` → cold-path
  fallback.
- **P5 — anti-fragmentation:** semantic ward match before cold-path create.

Procedures (`run_procedure`) already merged via PR #185 — no work needed.

## 11. Known limitations

**`capability_missing` resolution — built-in tools cannot be self-healed.**
When a ward returns `RESULT: CAPABILITY_MISSING`, the cold-path builder
augments the ward by writing skills, wiring MCP config, and registering
procedures — all file/shell work it can do. It *cannot* add a new built-in
daemon tool at runtime; that needs a Rust change and a rebuild. Built-in-tool
gaps are rare (most capability gaps are skills/MCPs/procedures), but a
`capability_missing` whose missing piece is a built-in tool cannot be
augmented away — it must surface to the user. Also note: an MCP wired during
augmentation only loads on the next daemon restart, so the augmented ward is
not immediately able to use it within the same session.
