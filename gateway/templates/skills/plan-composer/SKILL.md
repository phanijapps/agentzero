---
name: plan-composer
description: Plan against the active ward template and persist applicable declared plan/task artifacts.
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
of inventing or silently substituting an agent. For a concrete refinement slug,
every matching declared required plan role is applicable. An optional repeatable
task role is applicable only when the concrete plan is decomposed into task artifacts;
never create placeholder tasks. Persist every applicable artifact before returning execution steps;
resolve the declared rules instead of supplying remembered paths;
no task index is required when no matching declared index rule exists.

When an applicable persistent role is genuinely absent, keep that artifact
ephemeral and return `role_not_declared` without treating it as an error.
Never invent a fallback path. Propagate the selected ward and template digest
with every task. Do not claim persistence or return execution steps until a
successful ward lint confirms the written structure.

When a plan requires a new repeatable node explicitly annotated with
`operations.createConcept: true`, include a root-owned setup step that previews
it with `ward(action="dry_run", operation="create_concept", ...)` and creates it
with `ward(action="create_concept", ...)`. Do not plan shell or ordinary
file-tool creation for that node.

Any step that claims template conformance must finish with
`ward(action="lint", name=...)`. Conformance is proven only when the structured
result contains `ok: true` and `data.valid: true`; stale, unavailable, error, or
`data.valid: false` results remain unresolved and must be reported as such.
