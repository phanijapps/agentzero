# Changelog

All notable user-visible changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> Maintenance: this file is updated in the same PR that introduces the
> change. CI will warn (configurable: block) when a PR touches code that
> changes user-visible behavior but does not touch this file.
>
> Entries can be drafted from conventional commits: `git log --oneline`
> filtered to `feat:` and `fix:` since the last tag is a starting point,
> not a finished product. Rewrite for users, not contributors. See the
> [Common Changelog guidance](https://common-changelog.org/) — the audience
> is humans who use the software, not humans who wrote it.

## [Unreleased]

### Added

- **Agent-driven intent analysis** — a small intent agent searches indexed
  resources itself (skills, agents, wards, procedures) and returns a JSON
  contract including `solution_path` (seeds the planner) and task
  `complexity` (S/M/L/XL, drives iteration budget).
- **Structured failure feedback** — a tool call failing twice with identical
  arguments now injects an explicit nudge naming the call and the last error,
  instead of a generic stuck warning.
- **Failed-episode avoid-list** — sessions start with the ward's recent
  failed episodes (and their learnings) marked `[AVOID]`, so the agent
  doesn't repeat known-bad approaches.
- **Belief Network reachable** — new `belief` tool surfaces synthesized
  beliefs and contradictions to the model (previously wired but unreachable).

### Changed

- **Memory layer runs on engram end-to-end** — fact retrieval fuses
  semantic, lexical, and recency lanes via engram's weighted reciprocal-rank
  fusion; recall access reinforces facts (mention count + last-accessed
  decay refresh). Measured on the production path: recall precision@5
  46.7% → 76.7%, correction recall 2/5 → 5/5.
- **Gateway decomposed** — the 49-field AppState god object became six
  composed state groups (stores/services/execution/transport/workers/vault);
  A2A and durable-agent tasks moved out of the shell root.
- **Tool surface diet** — `glob`, `memory` (broad variant), `graph_query`,
  and `query_resource` tools removed after production traces showed zero
  usage; memory reads consolidated into `recall` (now with exact-key lookup);
  agent-control tools hidden by default.
- **Session fork on fast models fixed** — planner delegation no longer fails
  when the model dispatches it milliseconds after ward creation.
- **Procedure recall carries the call contract** — recalled procedures now
  include their declared parameters and success record, so the model calls
  them correctly the first time.

### Removed

- The legacy SQLite memory/knowledge store layer (~17,000 lines) is retired —
  all memory persistence goes through the engram adapter.


### Added

- Optional A2A 1.0 federation lets explicitly paired zBots discover one
  another on a LAN/VPN and delegate bounded text work asynchronously. Remote
  work is durable, authenticated, peer-scoped, and restricted to a
  respond-only execution profile; no broker or second listener is required.
- Ward creation now supports explicit `generic` and `coding` archetypes from a
  local, editable registry. Existing singular Ward templates transition
  non-destructively into `generic`, while created Wards retain their copied
  layout and recorded archetype.
- First-time commissioning now offers an explicit recommended Full Zbot memory
  profile with built-in embeddings, pinned recall/governance configuration, and
  a restart-verified activation screen. The safe baseline remains available
  and preserves existing memory configuration.

### Changed

- Research requests are now persisted before acceptance and recovered through
  the existing local durable-work worker after daemon interruption, while the
  WebSocket protocol and live Research experience remain unchanged.
- New Wards for every bundled archetype now start with one Ward-named canonical
  page, agent instructions, an append-only log, and their layout snapshot.
  Specialized work folders, linked pages, sources, and
  `.zbot/specs/<concept>/` planning files remain lazy.
- `wards/index.md` is now the sole index and catalogs each Ward with a
  canonical wikilink. Ward Markdown uses the filesystem-authoritative LLM Wiki
  model without mandatory OKF frontmatter.
- Journal Ward templates now route daily material to one
### Deprecated

- (nothing yet)

### Removed

- The deprecated standalone WebSocket listener on port `18790`, together with
  the `--ws-port` and `--legacy-ws-port-enabled` daemon flags, has been
  removed. External integrations must migrate from `ws://<host>:18790` to
  `ws://<host>:18791/ws` (or the configured HTTP port plus `/ws`) before
  upgrading.

### Fixed

- (nothing yet)

### Security

- A2A federation is disabled by default, stores per-peer bearer credentials in
  an owner-readable local trust file, rejects unsafe endpoint resolution and
  redirects, throttles failed authentication, and treats all discovered
  metadata and remote results as untrusted data.
- Commissioning completion now rejects non-local originless peers before body
  parsing and provisions fixed memory-profile files without following symlinks
  or overwriting conflicting content.
