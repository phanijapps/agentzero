# Architecture

How the code is *currently* organized. Not why (that's in
[`../adr/`](../adr/)) and not what we want (that's in
[`../rfc/`](../rfc/)) — **what is**.

- [`overview.md`](overview.md) — the map of the monorepo. What's in
  `apps/`, `packages/`, `tools/`, `packs/`, and how they relate.
  Read this first.
- [`security.md`](security.md) — data-root confinement, dependency provenance,
  and diagnostics controls for adapter and migration boundaries.
- [`engram-code-graph-indexing.md`](engram-code-graph-indexing.md) — what to
  index as Engram code-graph topology, retrieval evidence, or neither.
- [`engram-graph-noise-audit.md`](engram-graph-noise-audit.md) — read-only
  audit of vault graph noise and the separate live-MCP scope issue.
- `<subsystem>.md` — one file per non-trivial subsystem (add as the repo
  grows). Each describes the structure, the entry points, and links to
  the ADRs that explain why.

Architecture docs are the *rolled-up snapshot* — the answer to "what
does this codebase look like today" without replaying ADR history.
Lifecycle: living. Update whenever the layout or major dependencies
change.

Note for contributors: the bundle's source-of-truth split (skills,
agents, hooks, commands, hook-wiring, and pack seeds all live under
`packs/<pack>/`) is described in
[`../CONVENTIONS.md` § Pack source-of-truth split](../CONVENTIONS.md#pack-source-of-truth-split).
Anything in this directory documents the *projected* layout adopters
end up with; the pack-side authoring rules are in CONVENTIONS.
