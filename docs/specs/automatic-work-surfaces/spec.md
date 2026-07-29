# Spec: Automatic work surfaces

- **Status:** Shipped
- **Owner:** zbot maintainers
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** [`contracts/asyncapi/agent-surfaces.yaml`](../../../contracts/asyncapi/agent-surfaces.yaml)
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Let zbot automatically add a validated native work surface when structured
presentation materially improves an answer. In Quick Chat and other
A2UI-capable web sessions, root and ward agents can choose a display-only
metric, status, record, timeline, table, or chart surface without the user
requesting a component explicitly. Simple or primarily prose answers remain
text-only. Every surface supplements—never replaces—the canonical response,
and an agent can update its own surface during the same session using a stable
identifier.

## Boundaries

### Always do

- Expose one gateway-owned, capability-gated presentation tool whose input is
  validated against `zbot/work-surface/v1` before it can emit an event.
- Tell the model to present only when structure improves comprehension and to
  send the canonical response regardless of presentation support.
- Keep automatic surfaces display-only and available only to root and ward
  agents producing user-facing session results.

### Ask first

- Adding automatic presentation to delegated executor or reviewer agents.
- Adding interactive actions, navigation, external side effects, or a new
  component/catalog version.
- Adding a new transport, persistence layer, top-level crate, or runtime
  dependency.

### Never do

- Never accept agent-authored HTML, CSS, URLs, code, formatter functions, event
  handlers, tool names, shell commands, or `ApprovalGate` through the automatic
  presentation tool.
- Never infer or execute a surface from ordinary assistant text; only a
  successful, validated tool result may produce a surface event.
- Never omit, delay, or alter the canonical response because a surface was
  produced, rejected, unsupported, or not displayed.
- Never publish secrets, system/developer prompts, hidden reasoning/context, or
  unrelated connector/tool data; surface data must already be appropriate for
  the canonical user-facing answer.
- Never add a new top-level crate/package, persistence backend, transport,
  renderer path, or runtime dependency for automatic presentation.

## Testing Strategy

- Tool schema, actor allowlist, display-only component filter, descriptor
  validation, and create/update marker selection: **TDD**, because each is a
  closed deterministic contract.
- Tool-result marker → runtime event → gateway validation → Quick Chat
  replacement: **goal-based integration tests**, because the behavior spans
  existing execution and client boundaries.
- Automatic selection guidance: **contract test plus recorded model smoke
  check**, because tool descriptions are deterministic but model choice is
  probabilistic.
- Non-capable/headless behavior: **goal-based regression tests**, because
  canonical response delivery must be identical with or without surface
  subscribers.

## Acceptance Criteria

- [x] Given a fixed model, prompt, tool set, and run configuration, when one
  eligible prompt contains chart-shaped, comparative, metric, status, record,
  or timeline data, the recorded smoke transcript contains a
  `present_surface` call; when a paired simple factual prompt runs under the
  same configuration, its transcript contains no `present_surface` call.
- [x] Given a valid display-only descriptor, when the agent calls the
  presentation tool in create mode, the tool returns a `__work_surface` marker
  using the server-owned `zbot/work-surface/v1` catalog identifier.
- [x] Given the same stable `surface_id`, when the agent calls the tool in
  update mode, Quick Chat receives `surface.updated` and replaces the prior
  surface without duplicating it or reloading the page.
- [x] Given the same create/update events in Research or another existing
  A2UI-capable web session, its shared surface state replaces the stable
  `surface_id` without duplication.
- [x] Given an unknown component, `ApprovalGate`, unsupported property,
  malformed JSON pointer, oversized payload, or data exceeding catalog
  budgets, when the tool is called, it returns an error and emits no surface
  marker.
- [x] Given a delegated executor or reviewer, when its model-visible tool list
  is built, the automatic presentation tool is absent.
- [x] Given a simple factual or primarily prose answer, the tool guidance tells
  the model not to create a surface, and the canonical response remains the
  only required result.
- [x] Given a non-A2UI or headless client, when an execution also produces a
  surface, the canonical response completes unchanged and no renderer is
  required.
- [x] Given any presentation-tool input, no URL, HTML, code, action, external
  side effect, filesystem operation, or network operation can be invoked.
- [x] Given model context contains secrets, system/developer instructions,
  hidden reasoning/context, or unrelated connector/tool data, the tool guidance
  prohibits publishing it and limits surfaces to data already suitable for the
  canonical user-facing answer.
- [x] Given malformed, actionable, or oversized tool input, the returned error
  is bounded and identifies only the generic field/reason without echoing
  component properties, bound data values, or raw JSON into tool output or
  logs.
- [x] Given a rejected `present_surface` call followed by `respond`, the same
  canonical final response and terminal session state are produced as a run
  without a presentation call.

## Assumptions

- Technical: the runtime already converts validated `__work_surface` and
  `__work_surface_updated` tool results into surface events (source:
  `runtime/agent-runtime/src/executor.rs`).
- Technical: no current tool produces work-surface markers, so a bounded
  agent-facing presentation tool is the missing trigger (source:
  `rg "__work_surface" runtime gateway apps` on 2026-07-28).
- Technical: the gateway tool registry is the established capability-gated
  integration point (source:
  `gateway/gateway-execution/src/invoke/executor.rs`).
- Technical: the existing AsyncAPI create/update events and
  `zbot/work-surface/v1` contract need no new transport shape (source:
  `contracts/asyncapi/agent-surfaces.yaml`).
- Process: surfaces remain supplementary to canonical text and cannot add
  executable content or unapproved actions (source:
  `docs/specs/agent-driven-surfaces/spec.md`).
- Product: "as needed" means the model presents only when structure materially
  improves the answer, not on every response (source: user confirmation
  2026-07-28).
- Product: automatic selection applies to Quick Chat and other A2UI-capable web
  sessions, and may update a stable surface during a turn (source: user
  confirmation 2026-07-28).
- Product: the automatic tool remains display-only and prohibits URLs, HTML,
  code, actions, and side effects (source: user confirmation 2026-07-28).
