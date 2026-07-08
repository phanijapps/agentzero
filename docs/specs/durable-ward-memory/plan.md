# Plan: Durable Ward Memory

- **Spec:** [`spec.md`](spec.md)
- **Status:** Closed

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially
> (a different approach, not just a re-ordering), note why in the changelog
> at the bottom.

## Approach

Keep the existing warm-path ward-agent flow intact and harden the durable memory
chain around it. The work starts by naming a normalized route-hint shape in the
domain/recall layer, then adapting facts, wiki articles, procedures, episodes,
artifacts, and ward files into that shape. Next, recall/search and ward-content
surfaces expose the hints without changing ranking. Finally, runtime handoff and
context compaction preserve these pointers so long sessions can be summarized
without losing the path back to the real ward source. Schema changes are a last
resort; prefer deriving hints from existing fields first.

## Constraints

- RFC-0001: runtime owns live context forgetting; sleep workers own durable
  memory/KG hygiene only.
- RFC-0002 and [`memory-hygiene`](../memory-hygiene/spec.md): durable memory
  remains in `knowledge.db`; hygiene is observable and must not replace the
  store boundary.
- [`runtime-context-control`](../runtime-context-control/spec.md): live
  compaction must preserve the plan block and durable memory boundary.
- Existing `ward:{name}` delegation is production behavior. This plan must not
  add a competing invocation path.
- Wards are source workspaces. Do not silently edit or normalize ward files as
  part of pointer preservation.

## Construction tests

**Integration tests:**
- `cargo test -p gateway-execution intent_analysis`
- `cargo test -p gateway-execution ward_agent`
- `cargo test -p gateway-memory recall`
- `cargo test -p agent-runtime context`

**Manual verification:** inspect one existing conversation path in
`/home/videogamer/Documents/zbot/data/conversations.db` showing intent
`use_existing` -> `delegate_to_agent(agent_id="ward:{name}")` -> completed
`ward:{name}` child execution.

## Tasks

### T1: Warm-path ward-agent contract is pinned by tests

**Depends on:** none

**Touches:** `gateway/gateway-execution/src/middleware/intent_analysis.rs`,
`gateway/gateway-execution/src/invoke/setup.rs`,
`gateway/gateway-execution/src/delegation/spawn.rs`,
`gateway/gateway-execution/tests/*ward*`,
`gateway/gateway-execution/tests/*intent*`

**Tests:**
- TDD: `format_intent_injection` keeps emitting
  `delegate_to_agent(agent_id="ward:{name}", wait_for_result=true)` for
  `use_existing`, regardless of `execution_strategy.approach`.
- TDD: `load_or_create_specialist("ward:{name}")` synthesizes a ward agent
  from `wards/{name}/AGENTS.md`.
- TDD: `effective_ward_id("ward:{name}", None)` returns `{name}`.

**Approach:**
- Keep current warm-path behavior unchanged.
- Strengthen existing tests with assertions that the root is instructed not to
  call `ward(action="use")` or planner-agent before the ward-agent.
- Add a regression fixture that names the user-visible contract from this spec.

**Done when:** a change that routes an existing ward through root/planner
or breaks `ward:{name}` synthesis fails a focused gateway-execution test.

### T2: RouteHint domain shape exists without schema churn

**Depends on:** T1

**Touches:** `stores/zbot-stores-domain/src/*`,
`gateway/gateway-memory/src/recall/*`,
`gateway/src/http/memory_search.rs`,
`gateway/src/http/ward_content.rs`

**Tests:**
- TDD: constructing route hints for fact, wiki, procedure, episode, artifact,
  and ward-file examples yields stable `ward_id` and `source_kind`.
- TDD: route hints omit optional fields instead of fabricating paths when a DB
  row has no file backing.
- Goal-based: public JSON serialization uses stable snake_case or the existing
  local convention; tests pin the chosen field names.

**Approach:**
- Add a small `RouteHint`/`MemoryRouteHint` domain struct with:
  `ward_id`, `source_kind`, `source_path`, `session_id`, `execution_id`,
  `artifact_id`, and `memory_id`.
- Prefer optional fields over empty strings.
- Add adapter helpers near recall adapters instead of changing every repository
  method at once.
- Do not add a new table in this task.

**Done when:** route hints can be serialized and unit-tested independently from
ranking/search.

### T3: Recall adapters expose route hints without changing ranking

**Depends on:** T2

**Touches:** `gateway/gateway-memory/src/recall/adapters.rs`,
`gateway/gateway-memory/src/recall/mod.rs`,
`gateway/gateway-memory/src/recall/scored_item.rs`

**Tests:**
- TDD: fact, wiki, procedure, and episode recall items retain their previous
  score/order while carrying route hints.
- TDD: a ward-scoped fact with no source path still routes to its ward.
- TDD: a file-backed or artifact-backed item includes `source_path`.

**Approach:**
- Extend `ScoredItem` or its metadata map with a route-hint field.
- Populate hints in the existing adapter functions.
- Keep RRF, score filtering, intent boost, and truncation logic unchanged.

**Done when:** recall callers can inspect where to go next without changing
which items rank.

### T4: HTTP search and ward-content surfaces return route hints

**Depends on:** T3

**Touches:** `gateway/src/http/memory_search.rs`,
`gateway/src/http/ward_content.rs`,
`apps/ui/src/*`

**Tests:**
- TDD: `/api/memory/search` wiki/fact/procedure/episode hits include route-hint
  JSON when the backing item has a ward.
- TDD: `/api/wards/:ward_id/content` includes route hints or equivalent source
  coordinates for each returned memory block.
- Goal-based: TypeScript build stays green if UI types need updates.

**Approach:**
- Add `route_hint` to the JSON conversion helpers for each memory type.
- Keep old fields in place for backward compatibility.
- Update UI/client types only if strict typing requires it; do not build new UI
  affordances in this task.

**Done when:** API consumers can see the ward/file route from search results.

### T5: Artifact and ward-file pointers are indexed as discoverable memory

**Depends on:** T2

**Touches:** `services/execution-state/src/repository.rs`,
`services/execution-state/src/types.rs`,
`gateway/gateway-execution/src/artifacts.rs`,
`gateway/src/http/ward_content.rs`,
`stores/zbot-stores-sqlite/src/*`

**Tests:**
- TDD: an artifact recorded with `ward_id` and relative path produces a route
  hint with `source_kind = "artifact"` and `source_path`.
- TDD: a ward file discovered by ward-content produces
  `source_kind = "ward_file"` and a relative path, not an absolute host path.
- Goal-based: no test or API response exposes private absolute filesystem paths
  unless an existing endpoint already does so intentionally.

**Approach:**
- Reuse existing `artifacts` table fields first; add only adapter metadata if
  the table already stores enough coordinates.
- For ward files, derive route hints from `ward_id` plus relative path under
  `wards/<ward_id>/`.
- Keep filesystem scanning read-only.

**Done when:** file-backed ward knowledge can be surfaced as a pointer without
copying file contents into a new memory store.

### T6: Handoff writer preserves ward and artifact coordinates

**Depends on:** T3, T5

**Touches:** `gateway/gateway-execution/src/sleep/handoff_writer.rs`,
`gateway/gateway-execution/src/session_ctx/writer.rs`,
`gateway/gateway-execution/src/session_ctx/preamble.rs`

**Tests:**
- TDD: handoff content for a session with a `ward:{name}` child execution
  includes the active ward and child execution id.
- TDD: handoff content includes artifact paths and relevant durable memory ids
  when available.
- TDD: handoff filtering still avoids loading handoffs from the wrong ward.

**Approach:**
- Extend handoff input collection to include child ward executions and artifacts
  already recorded for the session.
- Render a compact "Pointers" block with ward, paths, execution ids, and memory
  ids.
- Keep prose summary separate from pointers so summarization cannot erase them.

**Done when:** a future session can recover the ward and files from the handoff
without rereading the full transcript.

### T7: Runtime compaction protects route pointers

**Depends on:** T3, T6

**Touches:** `runtime/agent-runtime/src/context_management.rs`,
`runtime/agent-runtime/src/middleware/context_editing.rs`,
`runtime/agent-runtime/src/middleware/summarization.rs`,
`gateway/gateway-execution/src/sleep/handoff_writer.rs`

**Tests:**
- TDD: a long conversation containing a route-pointer block keeps that block
  after deterministic context editing.
- TDD: last-resort summarization excludes route-pointer blocks or re-emits them
  verbatim.
- TDD: tool-result clearing does not remove the only artifact path associated
  with a delegated ward execution.

**Approach:**
- Mark pointer blocks with an existing protected-message mechanism if one fits,
  or add a narrow metadata flag for route pointers.
- Teach summarization eligibility to exclude route-pointer messages.
- Avoid changing normal user/assistant prose handling beyond this protection.

**Done when:** live context can shrink without losing ward/file coordinates.

### T8: End-to-end durable ward memory regression passes

**Depends on:** T1-T7

**Touches:** `gateway/gateway-execution/tests/e2e_ward_pipeline_tests.rs`,
`gateway/gateway-memory/src/recall/*`,
`gateway/src/http/memory_search.rs`

**Tests:**
- Goal-based: a synthetic `use_existing` intent produces a
  `ward:{name}` child execution and a final memory/search/handoff surface with
  the matching route hint.
- Goal-based: `cargo test -p gateway-execution e2e_ward_pipeline` passes.
- Goal-based: `cargo test -p gateway-memory recall` passes.

**Approach:**
- Build on existing e2e ward pipeline tests rather than adding a separate fake
  harness.
- Assert durable coordinates, not full generated prose.
- Include one DB-only memory hit and one file/artifact-backed hit.

**Done when:** the whole path "DB finds -> ward-agent executes -> pointers
survive" is covered by one regression.

## Rollout

Ship as additive metadata and response fields. Existing ward-agent delegation
continues to run as it does today. If a schema migration becomes necessary, land
it behind a separate plan update and keep old response fields source-compatible.

## Risks

- Route hints can expose host-specific absolute paths if adapters are careless;
  use ward-relative paths in API responses.
- Adding route metadata to scored recall items could accidentally perturb
  ranking if implementation touches sort/fusion logic; keep ranking tests in
  place.
- Handoff pointer blocks could become noisy if every artifact is listed; cap or
  rank pointers by relevance.
- Existing ward files are not cleanly structured; route hints must tolerate
  missing files, stale paths, and sparse `AGENTS.md`/`memory-bank` content.

## Future Hardening Backlog

- End-to-end daemon test: cover the full live route from intent
  `use_existing`, through `ward:{name}` child execution, artifact creation,
  handoff generation, and memory search `route_hint` output.
- Richer file provenance: make more memory writers persist relative source
  paths in `source_ref` or equivalent fields when the source is a ward file,
  artifact, report, spec, or generated wiki entry.
- UI route actions: expose route hints as open ward, open artifact, inspect
  source, and resume execution actions instead of raw JSON metadata.
- Ward anti-fragmentation: add a semantic duplicate guard before `create_new`
  ward creation and make the fallback recommendation observable.
- Stale pointer handling: add tests and API behavior for missing files, deleted
  artifacts, unavailable executions, and sparse ward directories.
- Pointer caps/ranking: rank pointer blocks by relevance and recency, then cap
  them to keep handoffs compact.
- Path hygiene: keep all externally visible pointers ward-relative unless an
  endpoint has an explicit local-filesystem contract.

## Changelog

- 2026-06-01: initial plan.
- 2026-06-01: captured known future hardening needs after the first
  route-hint implementation slice.
