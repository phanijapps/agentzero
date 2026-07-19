# Hermes-Agent vs z-Bot — Gap Analysis

**Date:** 2026-05-22
**Subject:** Nous Research's hermes-agent ([repo](https://github.com/nousresearch/hermes-agent), [docs](https://hermes-agent.nousresearch.com/docs/)) compared against current z-Bot.
**Scope:** Messaging connectors (Telegram, Discord, Slack, WeChat, etc.) and IDE/ACP adapters are **excluded by request**. Everything else is in scope.
**Confidence:** Single research pass over README + docs site, not a deep code review — finer-grained claims (role-enforcement strictness, plugin hook surface) could be slightly off. Top-tier gaps are clearly present in their README.

---

## Top tier — real capability gaps

### 1. Remote / serverless execution backends

Hermes ships seven terminal backends: **local, Docker, SSH, Singularity, Modal, Daytona, Vercel Sandbox** — Modal and Daytona offer serverless hibernation. z-Bot is local-shell only. Real gap for "agent that lives on a $5 VPS to a GPU cluster" deployments.

### 2. Smart per-task model routing — **IN PROGRESS (2026-05-24)**

Implemented for **curator** + **intent_analysis** (the two highest-value missing slots). Per-task `CuratorConfig` / `IntentAnalysisConfig` in `ExecutionSettings`, three-tier resolution (task → orchestrator → provider default), Settings > Advanced UI cards mirroring the Distillation pattern. Sleep-time stages (verifier, contradiction_detector, pattern_extractor, etc.) and other `MemoryLlmFactory`-routed calls remain on the orchestrator — covered in a separate follow-up since they need a small factory refactor.

Hermes routes auxiliary work — Curator, vision, embeddings, title generation, session search — to **different** models with per-task `reasoning_effort`. z-Bot has per-ward LLM config (shipped via PR #191) but no per-side-task routing.

### 3. Durable cross-session work queue (Kanban)

Hermes's Kanban plugin is a SQLite-backed work board with a dispatcher daemon (~60s tick), multi-profile / multi-worker collaboration, board isolation, and failure-limit auto-block. Hermes explicitly says `delegate_task` is **not durable** — long-running work goes here or to cron. z-Bot has cron + procedures but no durable kanban-style queue spanning sessions/agents.

---

## Mid tier — meaningful, smaller scope

### 4. Real subagent role gating

Hermes's `leaf` role *cannot* re-delegate, call `clarify`, send messages, or execute code; only `orchestrator` can spawn workers (capped by `max_spawn_depth`, default 2). z-Bot has `SubagentRole::Executor` / `Reviewer` but enforcement is largely prompt-text, not tool-call restrictions.

### 5. First-class `clarify` toolset

A dedicated tool for mid-task user questioning. z-Bot agents can prompt the user implicitly, but there's no canonical clarify mechanism.

### 6. Plugin lifecycle hooks

Hermes exposes `pre_tool_call` / `post_tool_call` / `pre_llm_call` / `post_llm_call` / `on_session_start` / `on_session_end` hooks. z-Bot has a `plugins/` directory but the hook surface looks thinner — worth a deeper read before committing to the gap.

---

## Lower tier — niche / strategic

### 7. Skills standard interop — **DROPPED (2026-05-24)**, not a real gap

z-Bot's SKILL.md format already matches the agentskills.io shape (name/description/license/metadata.{author,version,tags} + directory layout). The "gap" was a speculative read of the Hermes README. Do not re-propose.

Hermes is compatible with the `agentskills.io` open skill format; z-Bot skills are bespoke. Less portability across ecosystems.

### 8. TTS + image-generation tools

z-Bot has multimodal *analysis* (vision via `multimodal_analyze` per `component_multimodal_llm.md`) but no TTS or image-gen tool out of the box.

### 9. Pluggable memory provider abstraction — **DROPPED (2026-05-24)**, anti-goal

z-Bot's native memory stack (KG, episodic, bi-temporal, hierarchical, procedures-with-replay) is the differentiator; plugging into mem0/supermemory/etc. would degrade it. Do not re-propose.

Hermes plugs into mem0, supermemory, honcho, byterover, hindsight, holographic, openviking, retaindb. z-Bot is monolithic. **Arguably fine** given z-Bot's native memory is more sophisticated — see "ahead" section.

---

## Where z-Bot is ahead

For an honest comparison:

- **Native knowledge graph, episodic memory, procedures-with-replay, bi-temporal memory, hierarchical memory** (HiRAG/LeanRAG). Hermes's architecture docs do not include any of those as native concepts; everything memory-shaped lives in pluggable provider backends, `MEMORY.md` / `USER.md` prompt injection, or FTS5 over session text.
- **Ward-as-agent architecture** — domain-scoped delegatable agents with doctrine + procedures + per-ward LLM config + filesystem-authoritative graduation gate. Hermes is flatter (leaf / orchestrator roles only, no domain abstraction).
- **Pattern 3 — steerable running subagents** (`wait_agent` / `kill_agent`, `SteeringRegistry`). Hermes's `delegate_task` is synchronous-only by design.

---

## Sources

- https://github.com/NousResearch/hermes-agent (README, AGENTS.md root files)
- https://hermes-agent.nousresearch.com/docs/developer-guide/architecture
- https://hermes-agent.nousresearch.com/docs/ (user/developer guides index)
- Research pass: 2026-05-22.
