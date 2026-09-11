# Charter

> The foundational document for this project. One page, read whole.
> Modeled on the [CNCF project charter pattern](https://contribute.cncf.io/maintainers/governance/charter/):
> mission, scope, and principles in a single place, kept stable and short.

Changes to this file go through an RFC. The rest of the docs in this repo
are scaffolding around it; this file is the why.

---

## Mission

z-Bot is a desktop AI agent that gets durable work done — researching,
building, writing, and remembering across sessions — by connecting any
OpenAI-compatible model to a memory, knowledge, and tool layer the user owns.

## Scope

What this project does:

- Runs locally as the `zbotd` daemon (HTTP/WebSocket on `:18791`) with a
  React dashboard and a CLI (`zbot`).
- Connects to any OpenAI-compatible provider — local (Ollama) or cloud.
- Durable memory on [engram](https://github.com/phanijapps/engram): facts,
  procedures, episodes, beliefs, knowledge graph, and hierarchy, with
  recency- and usage-aware recall.
- Delegated subagents and wards (persistent project workspaces the agent
  creates and navigates).
- Skills (markdown playbooks) and MCP tools; external connectors.
- Session checkpoints and restart recovery.
- Experimental: same-session peer messaging between agents; A2A zbot
  federation behind the `--a2a` flag.

What this project does **not** do:

- Hosted or multi-tenant SaaS.
- Model training or fine-tuning.
- Non-OpenAI-compatible model backends.
- Executing untrusted code outside the user's own machine.
- Autonomous action without a configured owner.

The "does not" list is at least as important as the "does" list. It's how
we — and AI agents working in the repo — know when a request is out of
bounds. If you find the project being asked to do things that aren't on
either list, that's a signal to refine this section, not to drift.

## Principles

The values that resolve ties when reasonable people disagree.

1. **Evidence over opinion.** Production traces and golden-set floors
   decide. The tool diet deleted five zero-usage tools because traces said
   so; every recall change must hold the golden recall floors
   (30/30 presence, corrections 5/5) before it merges.
2. **One engine, no duplicates.** Persistence and retrieval piggyback on
   engram; when zbot and a dependency implement the same concept, zbot's
   copy dies. The sqlite store layer (~17K lines) was retired this way.
3. **No dead code survives.** Deletions are usage-audited (rg ground
   truth), never assumed — and nothing still used gets deleted.
4. **Typed boundaries.** Traits and typed errors at every seam —
   `StoreError` at the persistence boundary, `ToolCapability` policy at
   the tool boundary. Stringly interfaces don't ship.
5. **Behavior is proven, not claimed.** Golden recall set, conformance
   suites, and characterized-inventory tests back every migration wave;
   a refactor lands with its proof attached.

## What's NOT in this charter

- Implementation choices (Rust, Axum, SQLite) — see
  [`architecture/`](architecture/).
- Process (how to contribute, PR rules) — see [`CONVENTIONS.md`](CONVENTIONS.md).
- Direction and sequencing — see [`product/roadmap.md`](product/roadmap.md).
