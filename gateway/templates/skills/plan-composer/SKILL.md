---
name: plan-composer
description: Plan against the active ward template without requiring persistent plan artifacts.
---

# Plan Composer

Read the injected Active Ward Template packet and its digest. Do not assume
that plan, task, step, spec, history, concept, or index files exist. Rule IDs are
labels, not built-in behavior.

Always produce the executable plan in session state. Every step is a
self-contained briefing with exact `## Goal`, `## Agent`, `## Skills`,
`## MCPs`, `## Dependencies`, `## Inputs`, `## Outputs`, `## Acceptance`,
`## Tests`, and `## Status` fields. `## Agent` is the recommended agent and
uses an exact name from the live agent catalog;
`## Skills` and `## MCPs` contain exact canonical IDs from capability lookup,
one per line, or explicit `none`; together they are the step capabilities.
Keep agent, skills, and MCPs as separate
fields so session plan state preserves the assignment verbatim for root
delegation. If no live agent can execute a step, request a bounded replan instead
of inventing or silently substituting an agent. Persist the plan or individual task documents only
when the template contains applicable declared file rules; resolve those rules
instead of supplying remembered paths.

When no applicable persistent rule exists, keep the plan ephemeral and return
`role_not_declared` for the omitted artifact without treating it as an error.
Never invent a fallback path. Propagate the template digest with every task.

When a plan requires a new repeatable node explicitly annotated with
`operations.createConcept: true`, include a root-owned setup step that previews
it with `ward(action="dry_run", operation="create_concept", ...)` and creates it
with `ward(action="create_concept", ...)`. Do not plan shell or ordinary
file-tool creation for that node.

Any step that claims template conformance must finish with
`ward(action="lint", name=...)`. Conformance is proven only when the structured
result contains `ok: true` and `data.valid: true`; stale, unavailable, error, or
`data.valid: false` results remain unresolved and must be reported as such.
