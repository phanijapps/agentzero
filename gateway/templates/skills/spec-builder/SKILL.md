---
name: spec-builder
description: Write a refinement artifact only when the active ward template declares one.
---

# Spec Builder

Read the injected Active Ward Template packet before choosing any path. Rule IDs are
user data: do not infer a spec, concept, task, history, or index role from an ID
or from prior ward conventions.

For a concrete refinement slug, a matching declared specification role is applicable;
the user's request does not need to say spec by name. Resolve that rule beneath the
selected ward-relative parent and write the artifact before returning execution steps.
Respect its declared format and sibling rules. The content should capture
the objective, current evidence, acceptance criteria, constraints, outputs,
verification, and rerun behavior.

If the applicable role is genuinely absent, write nothing and return
`role_not_declared` with the template digest. Never invent a fallback path,
directory, or filename.

Return the selected ward, resolved ward-relative path, and template digest when
a file is written. Do not claim completion before successful ward lint and before
returning execution steps.
