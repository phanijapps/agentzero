# z-Bot Curator — Design (SUPERSEDED)

> **Superseded by `2026-05-23-ward-curator-spec.md`.** This first draft framed the curator around z-Bot **skills**. The user pointed out — correctly — that Hermes's "skill" is structurally a z-Bot **ward** (folder with metadata + sub-resources, agent-authorable, lifecycle-managed), not a z-Bot skill (progressive-disclosure prompt module). The reframed spec is the authoritative one. This doc is retained for the analysis-of-Hermes-mechanism content only.

**Date:** 2026-05-22
**Goal:** Close gap #1 from the Hermes comparison (`2026-05-22-hermes-comparison-gaps.md`) — give z-Bot an autonomous skill self-improvement loop equivalent to Hermes's Curator.
**Status:** Design — not yet implemented. Awaiting decision on the scope question in §6.

---

## 1. How Hermes does it (one-paragraph per pillar)

**Trigger.** The Hermes gateway runs a 60-second cron ticker (`gateway/run.py:17653`). Every 60th tick (≈hourly) it calls `agent.curator.maybe_run_curator(idle_for_seconds=inf)`, which then applies a two-stage gate: `should_run_now()` checks enabled + not paused + `now - last_run_at ≥ interval_hours` (default **7 days**), then `idle ≥ min_idle_hours * 3600` (default 2h, bypassed from the gateway). First-ever observation seeds `last_run_at=now` and defers — no boot run. Manual: `hermes curator run [--dry-run]`. There is **no cron-string scheduling**; the curator is a sleep-time stage of its own.

**Signals.** Read entirely from `~/.hermes/skills/.usage.json` (sidecar, atomic-written, file-locked) — per-skill record: `use_count`, `view_count`, `patch_count`, `last_used_at`, `last_viewed_at`, `last_patched_at`, `created_at`, `state` (`active|stale|archived`), `pinned`, `archived_at`, `created_by`. Counters are bumped at tool callsites (`bump_view`/`bump_use` in `skills_tool.py`, `bump_patch` in `skill_manager_tool.py`). **Telemetry lives outside SKILL.md so authored content stays untouched** — explicit design choice. `created_by="agent"` is the **sole signal** that opts a skill into curator management; it can only be set inside a forked curator-agent's `ContextVar` ("skill_write_origin = background_review"). Skills listed in `.bundled_manifest` or under `.hub/lock.json` are excluded.

**Decision logic — two layers.**

*Layer 1 (pure Python, no LLM).* `apply_automatic_transitions` walks each agent-created, non-pinned skill and computes `latest_activity_at = max(last_used_at, last_viewed_at, last_patched_at, created_at)`. Then: anchor ≤ now − 90d → archive; ≤ now − 30d and state=active → STALE; > 30d and state=stale → reactivate. Pinned skips everything.

*Layer 2 (LLM, the main job).* Spawns a forked `AIAgent` using an **auxiliary** LLM (config slot `auxiliary.curator.{provider,model}` — never the user's main session, to avoid blowing prompt cache), shows the model the **whole candidate table at once**, and asks it to cluster skills by name-prefix into 10–25 **umbrella** classes, then MERGE / CREATE / DEMOTE narrow siblings into umbrellas. The LLM emits a fenced YAML block (`consolidations:` + `prunings:`) and every `skill_manage delete` call carries an `absorbed_into=<umbrella>` argument. The big insight: **the curator's main value is consolidation, not just archival** — it tames the proliferation of narrow skills agents create.

**Actions.** Layer-1: direct `archive_skill()` (renames dir into `.archive/`). Layer-2: the forked agent uses `skill_manage` tool actions — patch / create / write_file / delete(absorbed_into=…). Pre-run safety: `curator_backup.snapshot_skills()` writes a tar.gz of the whole `skills/` tree (plus `.usage.json`, `.archive/`, `.curator_state`, cron jobs) to `.curator_backups/<utc-iso>/`, default keep=5, restorable via `hermes curator rollback`. Post-run side-effect: rewrites skill refs in `cron/jobs.json` so user cron jobs pointing at archived narrow skills get updated to the umbrella.

**State.** `.curator_state` (atomic JSON: `last_run_at`, `run_count`, `paused`, `last_report_path`, …) + `~/.hermes/logs/curator/<ts>/{run.json, REPORT.md}` per-run audit + `.curator_backups/<ts>/` tarballs.

**Skill schema.** Directory under `~/.hermes/skills/` with required `SKILL.md` (YAML frontmatter: `name`, `description`, `version`, `platforms`, `metadata.hermes.{tags, related_skills}`) and optional `references/` / `templates/` / `scripts/` / `assets/` siblings. Telemetry is **not** in frontmatter.

**What Hermes does NOT have:** no "patch if broken" counter heuristic — patching is left to LLM content judgement. No error-log or failure-counter signal — purely usage telemetry + skill source.

---

## 2. zbot mapping

| Hermes piece | z-Bot equivalent | Status |
| --- | --- | --- |
| `~/.hermes/skills/` | `~/Documents/zbot/skills/` | exists |
| `~/.hermes/skills/.usage.json` (sidecar) | `~/Documents/zbot/skills/.usage.json` | **new** |
| `.bundled_manifest` | enumeration from `gateway/templates/skills/` | **new** (compute at boot) |
| `.hub/lock.json` | n/a — no skill hub | — |
| `.curator_state` | `~/Documents/zbot/skills/.curator_state.json` | **new** |
| Per-run audit logs | `~/Documents/zbot/data/curator_logs/<ts>/` | **new** |
| `.curator_backups/<ts>/` | `~/Documents/zbot/skills/_curator_backups/<ts>.tar.gz` | **new** |
| Gateway 60s ticker → 1h tick → 7d gate | **sleep-time worker** (`gateway-memory/src/sleep/worker.rs`, 60-min cycle) + new gate | **extend** |
| `apply_automatic_transitions` (Layer 1) | `gateway/gateway-memory/src/sleep/skill_curator.rs` — pure Rust | **new** |
| Layer-2 forked AIAgent | new `curator-agent` invoked from the sleep stage | **new** |
| `auxiliary.curator.{provider,model}` | new auxiliary LLM config slot (overlaps with gap #3 — smart per-task routing) | **new** (or v1: reuse orchestrator) |
| `bump_use` / `bump_view` / `bump_patch` | extend `track_skill_load` (`runtime/agent-tools/src/tools/execution/skills.rs:49`) to also write the sidecar | **extend** |
| `skill_manage` tool (create/patch/delete) | new `SkillManageTool` runtime tool | **new** |
| `created_by="agent"` provenance | sidecar field; only settable when called from inside curator-agent context | **new** |
| `mark_agent_created` / ContextVar gate | `agent_runtime` task-local marker on the curator-agent's session | **new** |

---

## 3. Phased design

### Phase A — Persistent skill usage telemetry (prereq, no behavioral change)
Add `~/Documents/zbot/skills/.usage.json` mirroring Hermes's schema. Extend `track_skill_load` to bump `use_count` and `last_used_at` (atomic write, file lock via `fs2`). New crate-level helper `skill_usage` in `gateway-services` or `agent-tools`. **One file changed (`skills.rs`) + one new file.** Cost: small. Value: enables Phase B. No user-visible change yet.

### Phase B — Layer-1 Curator (heuristics only, no LLM)
New stage in `sleep/worker.rs::run_cycle` — `skill_curator`. Module: `gateway-memory/src/sleep/skill_curator.rs`. Pure Rust, no LLM call. Per cycle: 7-day interval gate (`.curator_state`), then for each agent-created non-pinned skill compute `latest_activity_at` and apply the same 30d/90d transitions Hermes uses. Pre-run: tar.gz snapshot to `_curator_backups/`. Per-run: `run.json` + `REPORT.md`. **Touches:** new module, `worker.rs` (add stage to `run_cycle`), `CycleStats` (add 3 fields). Cost: medium. Value: archival + state hygiene; only operates on agent-created skills (see scope question §6).

### Phase C — Layer-2 Curator (LLM umbrella-building, the main value)
New `curator-agent` agent in vault `agents/curator-agent/AGENTS.md` with a system prompt that mirrors Hermes's `CURATOR_REVIEW_PROMPT` (umbrella clustering, MERGE/CREATE/DEMOTE actions). New `SkillManageTool` runtime tool exposing patch/create/write_file/delete-with-absorbed-into. Sleep stage invokes the curator-agent with the candidate table; it runs to completion and reports consolidations + prunings. Cost: large. Value: the *consolidation* behavior — Hermes's marquee differentiator.

### Phase D (optional, separable) — Agent-driven skill auto-creation
Independently valuable: add a `create_skill` tool an agent can call after a complex task to materialise a new skill. Marks `created_by="agent"`. Without this, Phase B/C have nothing to act on (see §6).

---

## 4. Key design decisions (with recommended choices)

| Decision | Options | Recommended | Why |
| --- | --- | --- | --- |
| Telemetry persistence | A) JSON sidecar / B) SQLite table in `knowledge.db` | **A — sidecar** | Mirrors Hermes; portable; keeps `knowledge.db` schema focused on memory; atomic write + file lock is enough at this scale. |
| Telemetry location | Inside SKILL.md frontmatter / sidecar file | **Sidecar** | Hermes's deliberate choice — keeps authored content untouched, no git-merge churn. |
| Curator LLM | Reuse orchestrator / new auxiliary slot | **v1: reuse orchestrator** → migrate to auxiliary when gap #3 (smart per-task routing) ships | Avoids bundling two features; per-task routing is its own design item. |
| Curator state location | Vault / knowledge.db | **Vault** (`.curator_state.json`) | Matches sidecar pattern; user-inspectable. |
| Backup retention | Match Hermes (keep=5) / custom | **Keep 5** | Sensible default; configurable later. |
| Default thresholds | Match Hermes / custom | **Match Hermes** — stale=30d, archive=90d, interval=7d | No reason to differ on day one. |
| Scope of curated set | Agent-authored only / opt-in user skills too | See §6 | Hinges on Phase D. |

---

## 5. Anti-goals (explicit, to bound scope)

- **No auto-patching of user-authored skills.** Even in Phase C, the curator only acts on `created_by="agent"` items.
- **No failure-log mining for "patch if broken."** Hermes doesn't have it, we don't either — keep the signal surface small.
- **No skill hub / marketplace.** Hermes's `.hub/` concept is out of scope.
- **No new memory backend.** Telemetry is sidecar JSON, not a new SQLite table — unless review surfaces a reason.

---

## 6. Open question — the scope fork (needs decision before coding)

The Curator's value in Hermes is consolidating skills **the agent created**. z-Bot has **no agent-side skill-creation today** — only `LoadSkillTool`. Three viable scopes:

1. **Phase A + B only (cheapest, low value now).** Add telemetry + Layer-1 transitions. Operates only on a small set of "agent-authored" skills marked by hand or by a future tool. Mostly dormant until skill auto-creation lands. Cost: small, payoff: small now.

2. **Phase A + B + C (full Curator, no auto-creation).** Add the LLM umbrella-building too. Still operates only on agent-created skills, so still dormant without Phase D. Cost: large, payoff: same as path 1 until D lands.

3. **Phase A + B + D, defer C (telemetry + transitions + skill auto-creation).** Skip the LLM umbrella step for v1; ship the prereqs that *also* unlock auto-creation. Once agents start authoring skills, the heuristic curator runs against real data. Add Phase C later when there's signal to consolidate. Cost: medium, payoff: useful end-to-end loop earlier. **Recommended.**

There's also a path 4 — make Layer-1 operate on **opt-in user-authored skills** (`curate: true` in frontmatter) — but this risks touching the user's hand-crafted work and breaks the Hermes-aligned `created_by` model. Probably not.

**Decision needed:** which scope?

---

## 7. Sources

- `2026-05-22-hermes-comparison-gaps.md` (the parent gap analysis)
- Hermes deep-dive research pass, 2026-05-22 — files read: `agent/curator.py` (1781 LOC, full), `tools/skill_usage.py` (608 LOC, full), `tools/skill_provenance.py` (78 LOC, full), `agent/curator_backup.py`, `tools/skill_manager_tool.py` + `tools/skills_tool.py` (callsites only), `gateway/run.py:17653-17744` (curator wiring), `cron/{jobs,scheduler}.py` (confirmed one-way: curator rewrites cron, cron does not fire curator), `skills/dogfood/SKILL.md` (schema sample).
- z-Bot code touched in the survey: `gateway/gateway-memory/src/sleep/{worker.rs, mod.rs}`, `runtime/agent-tools/src/tools/execution/skills.rs`, `gateway/src/state/mod.rs:1507/1600` (`seed_default_cron`).
