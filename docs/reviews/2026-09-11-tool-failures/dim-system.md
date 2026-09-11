# Dimension 4 — Is search the right primitive?

## What lookup_capabilities is

`CapabilityCatalogTool` (capabilities.rs): case-insensitive substring match (`entry_matches`) over the host-provided catalog state (`app:planner_capability_catalog`), paginated (max 25/page), bounded descriptions. It is **retrieval** — probabilistic relevance by substring — over a corpus that (per Dimension 2) excludes agents.

## The set it should answer for

Roster reality: **7 agents, 14 skills** (+ ward-as-agent virtuals). This is a **fixed, small, enumerable manifest** — not a long tail. The planner's information need is deterministic: "what delegates exist and their one-line roles". Semantic search over 21 items is the wrong shape in three ways:

1. **Coverage**: substring matching answers "does X exist" only if the query lexically overlaps the entry — hence "developer"≠builder-agent, "data analyst"≠(absent), 12 re-phrasings.
2. **Completeness**: pagination + ranking hide the full set; the planner can never *see* the whole roster to choose from.
3. **Cost**: each query is a model turn (~1–3s latency + tokens). A manifest line is ~120 tokens once.

Delegation validation (spawn.rs `validate_dynamic_assignment_target`) requires an **exact agent id** — so the planner must ultimately produce exact ids that search only occasionally reveals.

## Design verdict: **hybrid — manifest primary, search for the long tail**

- **Manifest**: agents + skills as a compact block. Agents inline in the planner prompt (they're 7, stable, required by the planner's own contract). Skills inline as id+one-liner (14) or top-N + "use lookup_capabilities for the full skill catalog" if token budget matters.
- **Search stays** for: MCP tool discovery (open-ended), skill long-tail as it grows, and ward-scoped capability queries. Its catalog gains the missing `agents` key so it CAN answer roster queries as fallback.

### Manifest schema (one line per entry)

```
Agents: builder-agent (writes/reviews/fixes code) · research-agent (web research & synthesis) · writing-agent (content) · reviewer-agent (review) · adversarial-reviewer (red-team) · general-purpose (fallback executor) · planner-agent (planning; do not self-assign)
Skills: book-reader · clean-code · duckduckgo-search · eagle-eye · pdf · plan-composer · … (14 total; lookup_capabilities for details)
```

~150–350 tokens depending on skill inclusion.

## Minimal file-level change

1. `invoke_bootstrap.rs::build_planner_capability_catalog` — add `"agents": collect_agents_summary(...)` (async fn; signature change) → search fallback fixed.
2. Planner prompt assembly (builder.rs planner branch / planner AGENTS.md template) — inject the manifest line.
3. Optional: `builder.rs` currently writes `available_agents` state "for list_agents tool" — either register that tiny tool or delete the dead state write.

**When lookup remains justified**: MCP discovery, skill-detail lookup, ward-specific capability filtering — genuinely long-tail corpora. Not the 7-agent roster.
