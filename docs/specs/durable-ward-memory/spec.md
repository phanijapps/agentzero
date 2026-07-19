# Spec: Durable Ward Memory

- **Status:** Closed
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** RFC-0001: Unified Compaction and Memory Policy; RFC-0002: Memory Hygiene; [`runtime-context-control`](../runtime-context-control/spec.md); [`memory-hygiene`](../memory-hygiene/spec.md)

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make Layer 4 durable memory route through the existing ward-agent model without
flattening wards into summaries or replacing them with database rows.
`knowledge.db` remains the first-level searchable index for facts, wiki
articles, procedures, episodes, goals, KG data, and ward/resource metadata.
Wards remain the durable source corpus and executable agent workspaces:
`AGENTS.md`, `memory-bank/`, specs, reports, code, data files, and artifacts.
When recall, intent analysis, handoff, or compaction references durable work,
the system should preserve enough ward/file/artifact pointers for a future
agent to enter the right ward and inspect the underlying source.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off
before proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Treat existing `ward:{name}` warm-path delegation as production behavior, not
  as future work.
- Keep `knowledge.db` as the first-level durable search/index substrate for
  memory facts, ward wiki articles, procedures, episodes, goals, KG data,
  embeddings, and compaction audit.
- Treat ward directories as durable source workspaces whose files may be messy
  but still carry source value.
- Preserve `ward_id` on every Layer 4 route hint, and preserve file/artifact
  paths whenever a recalled item or handoff is backed by a ward file.
- Prefer additive metadata and adapters over schema churn when existing tables
  already carry `ward_id`, `session_id`, `execution_id`, or artifact paths.
- Keep runtime compaction focused on live context, while preserving pointers to
  durable DB rows and ward files that outlive the session.

### Ask first

- Adding new `knowledge.db` tables or changing uniqueness constraints.
- Changing the `MemoryFactStore`, `WikiStore`, `ProcedureStore`, or
  `EpisodeStore` trait contracts in a way that breaks existing implementors.
- Automatically editing, deleting, archiving, or restructuring files inside a
  ward's `memory-bank/`, `specs/`, reports, code, or wiki content.
- Changing the intent-analysis rule that `use_existing` delegates to
  `ward:{name}`.
- Changing warm-path behavior to use planner-agent or root orchestration before
  a ward-agent gets the task.

### Never do

- Never replace ward files with `knowledge.db` summaries as the only durable
  source.
- Never add a second ward invocation mechanism; existing `ward:{name}`
  delegation remains the execution path.
- Never make sleep-time hygiene silently rewrite ward files.
- Never summarize away the only pointer to a ward artifact, source file,
  delegated execution, or session episode.
- Never create a near-duplicate ward when intent analysis already identifies an
  existing ward that covers the task.

## Testing Strategy

- Warm-path contract: **TDD**. Existing `use_existing` intent analysis must
  keep producing a `delegate_to_agent(agent_id="ward:{name}",
  wait_for_result=true)` instruction, and ward-agent synthesis must keep routing
  execution into that ward.
- Route-hint adapters: **TDD**. Normalized recall/search items should expose
  stable `ward_id`, `source_kind`, and optional `source_path`, `session_id`,
  `execution_id`, and `artifact_id` fields without changing ranking behavior.
- Handoff and compaction pointer preservation: **TDD**. Long-session summaries
  and sleep handoffs should retain active ward, delegated ward-agent execution
  ids, artifact paths, and relevant durable memory ids.
- Persistence boundaries: **goal-based check**. Grep and targeted tests should
  prove this feature does not add a new durable memory store or route memory
  writes away from `knowledge.db`.
- End-to-end ward memory flow: **goal-based check**. A targeted gateway
  execution test should prove that intent `use_existing` creates a
  `ward:{name}` child execution and that the final recall/handoff surface still
  points back to the ward/files involved.

## Acceptance Criteria

- [ ] `use_existing` intent analysis continues to inject a required
  `delegate_to_agent(agent_id="ward:{name}", wait_for_result=true)` warm-path
  instruction.
- [ ] Delegating to `ward:{name}` continues to synthesize a ward-agent from
  `wards/{name}/AGENTS.md`, and the child execution's effective ward is
  `{name}`.
- [ ] Recall/search response items that come from facts, wiki articles,
  procedures, episodes, artifacts, or ward files expose a normalized route hint
  containing at least `ward_id` and `source_kind`.
- [ ] Route hints include `source_path` whenever the source is backed by a ward
  file or artifact path.
- [ ] Runtime handoff and compaction summaries preserve active ward,
  `ward:{name}` child execution ids, artifact paths, and referenced durable
  memory ids instead of reducing them to prose-only summaries.
- [ ] No implementation path silently rewrites, deletes, archives, or
  restructures ward files during sleep-time hygiene or runtime compaction.
- [ ] Intent analysis keeps using `knowledge.db`/memory-fact ward indexes as the
  first-level discovery mechanism before routing into a ward-agent.
- [ ] Tests cover both a DB-only hit with a ward route and a file-backed hit with
  a ward path.
- [ ] `cargo test -p gateway-execution intent_analysis` and
  `cargo test -p gateway-execution ward_agent` pass, or their current exact test
  filters are updated in the plan changelog.
- [ ] `cargo test -p gateway-memory recall` and `cargo test -p agent-runtime
  context` pass for the pointer-preservation behavior.

## Known Future Needs

These items harden the layer after the first route-hint implementation. They
are intentionally future needs, not blockers for the current Durable Ward Memory
slice.

- Add an end-to-end daemon regression for the full live path:
  `intent use_existing` -> `ward:{name}` child execution -> artifact -> handoff
  -> memory search `route_hint`.
- Expand file provenance so more writers populate exact `source_ref` or
  relative ward source paths when creating facts, wiki entries, procedures,
  episodes, artifacts, and reports.
- Add UI affordances that render route hints as direct actions such as open
  ward, open artifact, inspect source, and resume related execution.
- Add an anti-fragmentation guard before `create_new` ward creation so near
  duplicate wards are avoided when an existing ward is semantically close.
- Define stale-pointer behavior so missing ward files, deleted artifacts, or
  unavailable source paths degrade gracefully while preserving the durable DB
  row and ward id.
- Rank and cap pointer blocks by relevance and recency rather than collection
  order when multiple ward executions, artifacts, or memory ids are available.
- Keep tightening path hygiene so API responses and handoff blocks never leak
  absolute host paths when a ward-relative path is sufficient.

## Assumptions

- Technical: intent analysis tells the model to delegate to `ward:{name}` when
  `ward_recommendation.action == "use_existing"` (source:
  `gateway/gateway-execution/src/middleware/intent_analysis.rs`).
- Technical: `ward:{name}` delegation is implemented by synthesizing an agent
  from the ward directory, not by loading an `agents/<id>` folder (source:
  `gateway/gateway-execution/src/invoke/setup.rs`).
- Technical: delegated `ward:{name}` executions run in that ward even when the
  parent session has no active ward (source:
  `gateway/gateway-execution/src/delegation/spawn.rs`).
- Technical: conversation history confirms real warm-path executions for
  `ward:financial-analysis` and `ward:travel-planning` from intent
  `use_existing` to child execution completion (source: read-only SQLite query
  against `/home/videogamer/Documents/zbot/data/conversations.db`, 2026-06-01).
- Technical: `knowledge.db` contains ward-scoped facts, wiki, procedures,
  episodes, goals, KG data, and vector indexes; it is already the searchable
  durable substrate (source:
  `docs/architecture/components/memory-layer/overview.md`).
- Technical: wards are persistent directories with `AGENTS.md`,
  `memory-bank/`, artifacts, specs, and session-produced files; live local wards
  confirm this shape under `/home/videogamer/Documents/zbot/wards` (source:
  read-only filesystem probe, 2026-06-01).
- Process: no local `docs/CONVENTIONS.md` or `docs/CHARTER.md` exists; existing
  spec precedent is `docs/specs/runtime-context-control/` and
  `docs/specs/memory-hygiene/` (source: repository read, 2026-06-01).
- Product: Layer 4 should formalize `knowledge.db` as the first-level index and
  wards as the durable source corpus/workspace, with recall/compaction
  preserving pointers into ward files and artifacts (source: user confirmation
  2026-06-01).
- Product: this spec should not add a new ward invocation mechanism; warm-path
  ward-agent delegation is already in scope as existing behavior (source: user
  confirmation 2026-06-01).
- Product: the first implementation slice should focus on pointer preservation
  and recall route hints, not broad ward cleanup or restructuring (source: user
  confirmation 2026-06-01).
