# Dimension 2 — Where the roster belongs (current-state map)

## The smoking gun

`build_planner_capability_catalog` (`invoke_bootstrap.rs:304-358`) assembles the planner's ONLY capability surface:

```json
{ "skills": [...], "mcps": [...], "intent_guidance": [...] }
```

**There is no `agents` key.** The catalog the planner can search contains zero agents — yet the planner's own instructions (`~/Documents/zbot/agents/planner-agent/AGENTS.md`) command: *"Assign only agents/capabilities present in the live agent catalog. Every step must name one recommended agent."*

So the planner is ordered to delegate by agent name, given a search tool whose corpus **cannot contain agents**, and told not to guess. Result (forensics): 12+ semantic re-queries — "developer", "executor builder implementer", "data analyst", limit 50/25 — each returning skill/MCP hits, never the roster it needs. Enumeration-via-search of a corpus that doesn't hold the answer.

## Adjacent wiring that exists but doesn't close the loop

- `available_agents` state **is** collected per-execution (`collect_agents_summary`, invoke_bootstrap.rs:1117) and injected (`builder.rs:548-553`) — cached "for list_agents tool". **No such tool exists** (grep: only `list_session_agents` — running agents — and `list_zbots`). The data is written; nothing reads it.
- The intent contract (`middleware/intent/contract.rs:26`) says agents come "from list_agents tool output" — a tool that was deleted/never shipped.
- `WardAudience::Subagent` ward description (`ward.rs:1166`) exists, so description-audience switching is an established mechanism.

## Roster reality (what a manifest would cost)

`ls ~/Documents/zbot/agents/`: **7 agents** (adversarial-reviewer, builder-agent, general-purpose, planner-agent, research-agent, reviewer-agent, writing-agent) + ward-as-agent virtual entries. Skills: ~15. A manifest of `{id, one-line role}` for 7 agents ≈ **~120 tokens**; with skills ≈ 300–400 tokens — cheaper than a single lookup_capabilities round-trip after the model prompt is counted.

## Candidate placements

1. **Planner catalog gains `agents` + system-prompt roster line** — extend `build_planner_capability_catalog` with `collect_agents_summary` output AND inject a compact roster into the planner's prompt assembly (builder.rs planner path). Kills the search entirely for the fixed set; lookup remains for long-tail skill/MCP discovery. Cost ~150 tokens/session.
2. **Capability preamble for all actors at session start** (root + planner): one block, `lookup_capabilities` becomes the long-tail fallback. Cost ~300 tokens.
3. **Fix only the catalog (agents key)**, keep search: planner still burns 1–3 queries per session (search for a 7-item list).

Harness consensus (Claude Code, SWE-agent, pi): fixed rosters and core tools are **inline in the system prompt**; search is reserved for the long tail. zbot inverted this — search for the fixed set, nothing inline.
