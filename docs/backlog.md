# Backlog — open items by spec

Single index of **open** work across every spec in `docs/specs/`. Each item
names the spec, the Acceptance Criterion (where one applies), what's blocking
it, and how it gets unblocked. Closed/shipped work is **not** kept here — see
each spec's Changelog and [`product/changelog.md`](product/changelog.md).

This is the tactical **backlog**: per-instance, no pack-side source after first
install — it's yours to curate. It is distinct from the **product roadmap**
(strategy, not a work index) at [`product/roadmap.md`](product/roadmap.md).
"Roadmap" = direction; "backlog" = the work/deferral index.

Deferred acceptance criteria point here by **anchor**: a spec criterion written
`- [ ] <outcome> (deferred: <anchor>)` means `<anchor>` resolves to a heading in
this file (GitHub heading-slug rules — lowercase, spaces become hyphens). The
deferral lives here, version-controlled and greppable, not in a PR comment that
rots. See `CONVENTIONS.md` § 4 (Spec metadata contract).

## How this file is maintained

- Every spec records its own `Status:` field and `Acceptance Criteria`
  checkboxes. This file aggregates the **open** items so they're visible in one
  place — it is not the source of truth.
- When an AC closes or a spec ships, update the spec first, then **remove** the
  now-closed item here in the same change (closed work lives in the spec
  Changelog / `product/changelog.md`, not here).
- When a new spec lands with open ACs, add a section here.
- If an item here is no longer accurate against the underlying spec, trust the
  spec and fix this file.

---

## engram-pinned-source-before-release

- **engram-memory-engine-cutover AC5:** Release/publish still needs Engram
  pinned through the sanctioned dependency mechanism instead of mutable local
  path dependencies. Blocked on the final source mechanism; unblocked by
  replacing local Engram path dependencies with the accepted pin and recording
  metadata, lockfile, revision/provenance, and dirty-state evidence.

## engram-fresh-db-manual-smoke

- **engram-memory-engine-cutover AC19:** User-run fresh-DB daemon/UI or CLI
  smoke still needs to cover chat, memory/knowledge activity, AgentZero-owned
  sleep-cycle cleanup, reload, and Memory/Graph/Observatory tabs. Blocked on
  manual runtime validation; unblocked by running the smoke against a fresh
  zbot data directory.

## sqlite-store-crate-split

- **engram-memory-engine-cutover follow-up:** `stores/zbot-stores-sqlite`
  still owns the live `conversations.db` runtime contract through
  `DatabaseManager`, `ConversationRepository`, execution/log/state adapters,
  bridge outbox, and distillation run status, so it cannot be deleted yet.
  Split or rename the crate so conversation/execution/outbox persistence lives
  in a runtime SQLite crate, while old `KnowledgeDatabase`, sqlite-vec,
  `MemoryRepository`, `GraphStorage`, old belief/wiki/procedure repos, and
  current-SQLite parity fixtures move to a quarantined legacy memory SQLite
  crate. Blocked on a focused crate-boundary spec; unblocked by extracting the
  runtime DB surface first, then shrinking the legacy crate until only
  migration/reference tests depend on it.

## dynamic-ontology-local-definition-import

- **dynamic-ontology-skos-taxonomy AC3/AC4:** Governance definition paths are
  confined under `~/Documents/zbot/config`, validated, and fingerprinted in
  migration manifests, but local ontology/SKOS JSON files are not parsed into
  active Engram definitions yet. Runtime bootstrap currently loads the built-in
  `zbot.base:v1` ontology and `zbot.general:v1` SKOS scheme. Blocked on a
  focused local-definition import spec; unblocked by defining the JSON schema,
  parser validation, overlay merge semantics, version conflict behavior, and
  tests that prove local definitions bootstrap through Engram public
  ontology/taxonomy repositories.

## dynamic-ontology-proposed-term-queue

- **dynamic-ontology-skos-taxonomy AC9:** Unknown predicates and mismatches now
  produce sanitized advisory findings, but model/extraction-discovered ontology
  terms and taxonomy concepts do not yet have a durable proposed-change queue.
  Blocked on a governed merge-policy spec; unblocked by adding a read-only
  proposed-term store, operator review semantics, and an explicit activation
  path that cannot silently mutate active definitions.

## engram-governance-finding-port

- **dynamic-ontology-skos-taxonomy follow-up:** zbot persists governance
  validation findings in adapter sidecars because Engram does not yet expose a
  generic validation-finding write/read port with scope, code, severity,
  target, and sanitized payload guarantees. Blocked on an Engram upstream port;
  unblocked by adding generic finding persistence/query APIs to Engram and
  moving zbot's sidecar-backed finding read model behind that public port.

## context-capability-resource-catalog-completion

- **context-capability-registry AC2:** The shipped catalog is first-party
  tool/actor-policy backed. It still needs live MCP manager metadata, connector
  read-only resource metadata, and memory/graph/recall resource provider
  entries as first-class catalog resources. Blocked on a focused resource
  catalog provider slice; unblocked by adding catalog sources that do not
  change tool execution behavior.

## context-capability-broad-tool-split-completion

- **context-capability-registry AC11:** Broad tools now expose split-target
  metadata and `load_skill` returns bounded packets. The weak built-in `grep`
  tool has been retired in favor of shell `rg`/`grep`, but `memory`,
  `query_resource`, `graph_query`, `shell`, `ward`, and `load_skill`
  are still callable action/resource compatibility surfaces. Blocked on
  replacing each read-heavy use with resource/context packet lanes; unblocked by
  removing each broad wrapper from default model registration once its split
  target has parity.

## context-capability-evidence-intake-completion

- **context-capability-registry AC12:** The `ingest` tool records structured
  evidence with retention, ontology, taxonomy, and provenance. `memory`
  fact writes, tool-result distillation, and resource-read distillation still
  need to route through the same internal evidence intake boundary. Blocked on a
  focused distillation/write-lane slice; unblocked by adding shared evidence
  recording behind those paths without making read-only resource access persist
  by default.

## spec-driven-research-development-contract-defect

- **Defect:** The spec-driven research/development loop uses markdown prose as
  the execution contract, so root, planner, ward-designer, subagents, and the
  runtime infer different truths about plan location, next step, path scope,
  status, and completion. The path failures seen in
  `sess-c9847609-9ce6-47db-afcc-1b00c56b1a77` are a symptom: planner reported
  `wards/<ward>/specs/<domain>/plan.md`, root was ward-scoped and tried
  `financial-analysis/specs/...`, root then tried `~/...` with `read`, and only
  recovered by shell-searching for the file. The broader issue is that
  `plan.md`, `steps/*.md`, injected plan text, delegation callbacks, session DB
  state, and prompt shards all carry overlapping but non-authoritative state.
  This is especially weak for research/build hybrids where a plan marked
  `research` still needs code, data fetching, analysis, and final synthesis.
  Blocked on a dedicated spec-driven execution contract redesign; unblocked by
  introducing a machine-readable plan manifest with canonical ward-relative
  paths, explicit step ids, assigned agents, required skills/resources,
  artifact contracts, status transitions, and continuation pointers, then making
  markdown specs a rendered view instead of the source of runtime truth.

<!-- Add one section per spec with open work, e.g.:

## <spec-name>

- **AC<N> (deferred: <anchor>):** <what's open> — blocked on <X>; unblocked by <Y>.

-->

## conversation-store-revamp-summary-store

- **`SummaryStore` / `thread_summaries` (deferred: conversation-store-revamp-summary-store):**
  compaction-without-loss context-window assembly — a `SummaryStore` trait +
  `thread_summaries` table + a writer wired into
  `runtime/agent-runtime/src/context_management.rs` (where compaction already
  lives), so context = latest summary + message tail (`seq > as_of_seq`). Blocked
  on the conversation-store-revamp base (append-only `messages` + `checkpoints`)
  landing; unblocked by a follow-up spec that adds the summary writer +
  context-window assembly.
