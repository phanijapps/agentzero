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

## multimodal-provider-file-dialects

- **Multimodal Analyze URL Fetch AC2:** Give `multimodal_analyze` real
  document/file analysis by encoding per-provider file dialects (OpenAI
  `input_file`, GLM `file_url`) behind an encoder layer. Blocked because the
  OpenAI-compatible chat/completions surface has no file content part — the
  current shape (`{"type":"file","file":{"url":…}}`) is rejected by every
  provider (Ollama: 400 "invalid message format"), so file inputs fast-fail
  with shell-extraction guidance instead. Unblocked by a provider-dialect
  encoder selected from the multimodal config, with captured-request tests
  per dialect.

## main-path-multimodal-dialect

- **Multimodal Analyze URL Fetch (deferred):** Align the main LLM path's
  multimodal serialization with the OpenAI wire dialect — `ChatMessage`
  currently serializes image parts as `{"type":"image","source":…}` instead
  of `{"type":"image_url","image_url":…}`. Latent: no caller flows multimodal
  parts through the main LLM call today (vision goes through
  `multimodal_analyze`), but any future native-vision agent message would be
  rejected by providers. Unblocked by an encoder pass over message content
  before `build_request_body`, with serialization tests pinning the dialect.

## ward-slim-p3-plan-attention-warm-scope

- **Ward Slim P3 deferred:** `<plan_attention>` tells root to "re-delegate
  to planner-agent to regenerate" an unavailable session plan — correct for
  cold/planned work, contradicting the warm route's "Do NOT delegate to
  planner-agent". Blocked on the orchestrator-context high-stakes rule
  (plan/goal delivery changes need their own live multi-step verification);
  unblocked by scoping the line to graph-planned work and re-running both
  live flows.

## ward-slim-p5-gate-vocabulary

- **Ward Slim P5:** Placeholder-specs gate (`app:has_placeholder_specs`,
  delegate.rs) adopts the shared redirect-envelope helper from P2's
  `guards.rs`. State keys stay separate (invocation-local vs
  ward-persistent) — only the message format unifies. Blocked on nothing;
  smallest of the three. Unblocked by picking it up.

## a2a-external-conformance

- **A2A Federation and Discovery AC17:** Run the official A2A CLI/TCK and a
  documented two-daemon pair/delegate/get/cancel/restart journey, then record
  the commands and results in the verification artifact. Blocked because the
  repository integration suite does not provide an external authenticated A2A
  client or isolated process harness; unblocked by supplying that harness
  without weakening peer authentication and passing the advertised HTTP+JSON
  surface checks.

## p4-react-router-830

- **P4 CI and E2E Debt Cleanup AC7:** Remove the narrow
  `GHSA-qwww-vcr4-c8h2` audit exception and upgrade `react-router-dom` once npm
  publishes the advisory's patched `8.3.0` or newer release. Blocked because
  the registry's latest release is `7.18.2` and `8.3.0` returns `E404`;
  downgrading to `7.11.0` is not acceptable because its dependency graph has
  multiple other high-severity advisories. Unblocked when the patched release
  is installable and passes install, audit, lint, build, unit, and E2E gates.

## engram-fresh-db-manual-smoke

- **engram-memory-engine-cutover AC19:** The 2026-08-06 isolated fresh-vault
  smoke recorded provider startup, a root-agent turn, memory write/recall,
  on-demand consolidation, restart/session reload, and Memory/Graph/Observatory
  route rendering. Final acceptance is blocked on exercising knowledge-graph
  activity and an AgentZero-owned sleep-cycle cleanup, and on removing
  system-instruction fields from persisted traces; unblocked by recording the
  missing fresh-vault checks and landing/validating the
  `llm-instruction-log-redaction` follow-up. The separate CLI deep-mode
  pre-start failure remains tracked below.

## fresh-cli-deep-invocation

- **engram-memory-engine-cutover AC19 smoke:** Fresh-vault CLI one-shot forces
  deep mode and receives the normalized pre-start failure, while the fast
  WebSocket route completed a root-agent turn. Blocked on diagnosing the
  durable Research startup path; unblocked by a targeted fix and fresh-vault
  CLI evidence.

## llm-instruction-log-redaction

- **engram-memory-engine-cutover AC19 smoke:** Fresh-vault tracing exposed LLM
  system-instruction fields at info level. Blocked on safe trace serialization;
  unblocked by redacting instruction/prompt fields before persistence and
  validating that observability remains useful without private configuration.

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

## engram-host-application-portability-contract

- **Engram upstream follow-up:** Engram needs a backend-neutral host-application
  contract before semantic persistence can move fully out of zbot. The contract
  must cover a provider facade, backend selection and lifecycle, capability
  discovery, facts/graph/episodes/evidence/ontology/taxonomy/belief APIs,
  atomic batches, embeddings, unified recall, maintenance, observability,
  stable identifiers and metadata, errors, conformance testing, and
  migration/export. Blocked on Engram API and conformance-suite design;
  unblocked when a host can change SQLite to another supported backend through
  Engram configuration and dependencies, without owning database connections,
  schema/migration internals, or backend-specific semantic queries. Detailed
  requirements are maintained in
  `~/Documents/engram-host-application-requirements.md`.
  The adapter-facing gap analysis and retirement order are documented in
  [`architecture/future-state/engram-adapter-capability-gaps.md`](architecture/future-state/engram-adapter-capability-gaps.md).

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

## nested-builder-mcp-handoff-defect

- **Defect:** A nested ward → `builder-agent` delegation can accept an explicit
  `mcps: ["blender-mcp"]` assignment yet start the child without a registered
  Blender MCP tool. In session `sess-ee8c4526-38a0-47eb-a0e8-c8aaa192e8d5`,
  the builder's tool probe listed only shell/file tools and it used the
  prohibited `bpy` fallback, while a later direct root → ward retry mounted
  the same MCP successfully. The capability-resolution audit entry that should
  distinguish a dropped assignment from a rejected/unavailable MCP was not
  persisted for that child. Blocked on a focused delegation-runtime fix;
  unblocked by making nested explicit MCP assignments fail closed before model
  execution when no tool registers, and by persisting the effective IDs plus
  rejection reason for every child execution.

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

## personal-finance-agui-modules

- **Agent-driven-surfaces follow-up:** Add read-only personal-finance surfaces:
  watchlist, allocation chart, research brief, risk flags, and market calendar.
  Blocked on a chosen market-data provider and an explicit personal-data
  retention policy; unblocked by a focused provider/configuration spec. Never
  add broker credentials, trade execution, or personalized buy/sell directives.
