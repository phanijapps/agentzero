---
name: spec-builder
description: Write a refinement artifact only when the active ward template declares one.
---

# Spec Builder

Read the injected Active Ward Template packet before choosing any path. Rule IDs are
user data: do not infer a spec, concept, task, history, or index role from an ID
or from prior ward conventions.

If the user's requested refinement artifact has a matching declared file rule,
resolve that rule beneath the selected ward-relative parent and write only that
file. Respect its declared format and sibling rules. The content should capture
the objective, current evidence, acceptance criteria, constraints, outputs,
verification, and rerun behavior.

If no declared rule can hold the requested artifact, write nothing and return
`role_not_declared` with the template digest. Never invent a fallback directory
or filename.

Return the resolved ward-relative path and template digest when a file is
written.
