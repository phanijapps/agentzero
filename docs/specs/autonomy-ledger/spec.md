# Spec: Autonomy Ledger

- **Status:** Shipped
- **Owner:** zbot maintainers
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Give zbot a durable, evidence-backed decision-thread record that users can
approve, explicitly resume, inspect, and close from any session. A ledger item
represents an explicit commitment or approved research follow-up, not a memory
fact. It preserves the current decision, bounded next action, source evidence,
and lifecycle without changing Engram semantic memory or creating a second
executor.

## Boundaries

### Always do

- Store operational items in zbot-owned conversation/runtime persistence; keep
  Engram responsible for semantic memory and knowledge graph data.
- Require explicit user approval before an item becomes runnable or scheduled.
- Treat semantic similarity as a candidate only; never silently attach a new
  request to an existing item.
- Construct a resume packet on the server only after parameterized lookup of
  the user-selected, `approved` item. Its fixed fields and evidence-reference
  limits are the only ledger data visible to the resumed executor.
- Treat review and eligibility as read-only operations. A resume starts a new
  execution only after the ledger audit record and packet construction succeed.
- Preserve existing gateway, execution, tool, and UI contracts.

### Ask first

- Adding automatic external writes, filesystem edits, or mutating scheduled
  work.
- Adding an LLM extraction call beyond existing structured intent analysis.
- Adding a new background worker, event transport, or dependency.

### Never do

- Do not create active items from ordinary recalled facts or casual topic
  mentions.
- Do not let model output select an item id or bypass approval policy.
- Do not accept a ledger packet, item selection, or executable context from a
  model response, generic client metadata, or caller-supplied packet body.
- Do not store raw conversation replay as item context or create a second agent
  executor.
- Do not introduce a new top-level crate or persistence backend.
- Do not add a cron/worker integration, retry loop, automatic execution, or
  implicit external/filesystem write in this release.

## Testing Strategy

- Ledger lifecycle, approval, and candidate-resolution rules: TDD with
  deterministic unit and SQLite integration tests.
- HTTP request/response and Mission Control rendering: integration tests that
  assert user-visible decision-thread state and actions.
- Gateway/event wiring and context-packet injection: goal-based integration
  tests because the existing executor and event pipeline remain authoritative.
- Manual QA: create, approve, resume, and close an item from separate browser
  sessions using the running daemon.

## Acceptance Criteria

- [ ] Given an explicit user commitment or approved research follow-up, when it
  is saved, zbot persists a decision thread with title, objective, next action,
  state, policy, source-session evidence, and timestamps in `conversations.db`.
- [ ] Given a decision thread in `proposed`, `approved`, `blocked`, `complete`,
  or `stale` state, when a user acts on it, only valid lifecycle transitions are
  accepted and every transition retains an auditable outcome. Optional outcome
  text is trimmed, non-blank, and bounded to 512 UTF-8 bytes.
- [ ] Given a new request that is merely similar to an active item, normal
  execution receives no item id, packet, or attachment. The user must inspect
  and explicitly select a decision thread in Mission Control; model output
  cannot select an item or cause a resume.
- [ ] Given an explicit user selection of one item for resume, when the server
  loads that exact item and verifies `state == approved`, zbot records one
  audit attempt and starts a new execution with one immutable, bounded
  `LedgerResumePacket`. The packet contains only item id, title, objective,
  next action, and at most eight kind/reference evidence pairs—never labels,
  transcript content, or caller-supplied packet data. `review` is read-only;
  a missing, non-approved, ambiguous, oversized, or unreadable item fails
  closed without executor-state mutation or retry.
- [ ] Given an approved item, when it is viewed in Mission Control, the user can
  inspect state, evidence count/detail, and next action, then approve, block,
  complete, or explicitly resume it. Resume targets a new session for the
  source agent and changes no unrelated or source session.
- [ ] Given any ledger state and approval policy, when a caller asks for
  `timer` eligibility, zbot returns a pure eligibility projection. It neither
  invokes/enqueues an executor, creates a run, changes lifecycle/timestamps,
  retries, nor performs external or filesystem writes; schedules and workers
  are deliberately out of scope.
- [ ] Given the feature branch, when workspace lint, type checking, and focused
  tests run, the existing execution, memory, conversation, and UI contracts
  remain valid.

## Assumptions

- Technical: `zbot-conversation` owns schema and narrow transcript/checkpoint
  stores in `conversations.db` (source: `stores/zbot-conversation/src/schema.rs`).
- Technical: the gateway already has intent configuration, streamed execution
  events, cron hooks, and plan-block context middleware (source:
  `gateway/gateway-services/src/settings.rs`,
  `gateway/gateway-execution/src/invoke/stream_event_processor.rs`,
  `gateway/gateway-hooks/src/cron.rs`,
  `runtime/agent-runtime/src/middleware/plan_block.rs`).
- Product: first release is manual continuation plus approved time-trigger
  eligibility only; it has no scheduler, worker, or autonomous write action
  (source: user confirmation 2026-07-09).
- Product: only explicit commitments or approved research outcomes may create
  proposed items; similarity never creates or attaches an item automatically
  (source: user confirmation 2026-07-09).
- Product: Mission Control is the first ledger surface, and ambiguous cross-
  session references ask the user to choose (source: user confirmation
  2026-07-09).
