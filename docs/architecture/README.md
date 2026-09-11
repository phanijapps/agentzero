# Architecture

How the code is *currently* organized. Not why (that's in
[`../adr/`](../adr/)) and not what we want (that's in
[`../rfc/`](../rfc/)) — **what is**.

- [`architecture.md`](architecture.md) — the system map: daemon, gateway,
  agent runtime, cognitive substrate, stores. Read this first.
- [`security.md`](security.md) — data-root confinement, dependency provenance,
  and diagnostics controls for adapter and migration boundaries.
- [`engram-code-graph-indexing.md`](engram-code-graph-indexing.md) — what to
  index as Engram code-graph topology, retrieval evidence, or neither.
- [`components/`](components/) — one directory per non-trivial subsystem
  (execution loop, intent analysis, ward scaffolding, …). Each describes
  the structure, the entry points, and links to the ADRs that explain why.
  See [`components/index.md`](components/index.md).
- [`future-state/`](future-state/) — the two living forward docs:
  [`path-to-release.md`](future-state/path-to-release.md) (cross-platform
  packaging tracker) and [`compaction-strategy.md`](future-state/compaction-strategy.md)
  (proposed, not yet implemented).
- [`../gateway-decompose-deck.html`](../gateway-decompose-deck.html) — the
  AppState decomposition plan as shipped (49 flat fields → 6 composed
  groups, five waves).

Architecture docs are the *rolled-up snapshot* — the answer to "what
is this system today?" When code changes shape, these change with it in
the same PR. A design that hasn't shipped yet belongs in an RFC or a
future-state doc, not here.
