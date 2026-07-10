# Spec: Autonomy Ledger

- **Status:** Implementing
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
approve, resume, inspect, and close from any session. A ledger item represents
an explicit commitment or approved research follow-up, not a memory fact. It
preserves the current decision, bounded next action, source evidence, and
lifecycle without changing the existing execution, Engram semantic-memory, or
gateway event contracts.

## Boundaries

### Always do

- Store operational items in zbot-owned conversation/runtime persistence; keep
  Engram responsible for semantic memory and knowledge graph data.
- Require explicit user approval before an item becomes runnable or scheduled.
- Treat semantic similarity as a candidate only; never silently attach a new
  request to an existing item.
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
- Do not store raw conversation replay as item context or create a second agent
  executor.
- Do not introduce a new top-level crate or persistence backend.

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
  accepted and every transition retains an auditable outcome.
- [ ] Given a new request that is merely similar to an active item, when intent
  analysis resolves the request, zbot returns it as related or ambiguous rather
  than silently resuming or attaching the old item.
- [ ] Given an explicit resume or review request with one high-confidence active
  match, when zbot begins execution, it injects one bounded decision packet
  containing the item objective, current state, next action, and linked
  evidence references rather than replaying its source conversation.
- [ ] Given an approved item, when it is viewed in Mission Control, the user can
  inspect state, evidence, and next action, then approve, block, complete, or
  resume it without changing any unrelated session.
- [ ] Given an item that is not approved, when a timer or future trigger sees
  it, zbot does not autonomously run it; first release exposes eligibility only
  and performs no external or filesystem write.
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
  eligibility only; no autonomous write actions (source: user confirmation
  2026-07-09).
- Product: only explicit commitments or approved research outcomes may create
  proposed items; similarity never creates or attaches an item automatically
  (source: user confirmation 2026-07-09).
- Product: Mission Control is the first ledger surface, and ambiguous cross-
  session references ask the user to choose (source: user confirmation
  2026-07-09).
