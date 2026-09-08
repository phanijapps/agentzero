# Roadmap

> Direction for the next 2-4 quarters. **Not commitments.** The whole point
> of writing this down is that it can change.

**Last updated:** 2026-09-08
**Reviewed:** quarterly. Next review: 2026-12-08.

If the current date is more than 90 days past "Last updated", treat this
file as stale and ask before relying on it.

## Now (current quarter)

What we're actively working on. Each item should link to a spec in
`docs/specs/` once one exists.

- **Rig-only execution** — shipped. [spec: rig-only-execution/spec.md]
  Complete Rig cutover: sole engine, no legacy executor, no engine-selection
  fallback. All 11 waves passed, all acceptance criteria met.
- **Execution consolidation** — in progress. [spec: exec-consolidation-waves/spec.md]
  Multi-wave cleanup: ExecCtx (one context type), ToolSpec const table,
  TurnSignal engine loop, typed errors, distillation extraction. Most waves
  complete; remaining work is decomposition of large-but-healthy files.

## Next (following 1-2 quarters)

What we expect to pick up after Now. These are intentions, not promises.

- **Clean up the codebase to be leaner and clean code.**
  Ongoing effort to reduce god files, eliminate stringly-typed errors,
  consolidate context types, and move subsystem logic into focused crates.
  Principles: data modeling over file splitting, one context type per
  subsystem, typed errors, no dead code. The lessons from the intent
  analysis rewrite ([docs/lessons/2026-09-08-intent-analysis-rewrite.md](../lessons/2026-09-08-intent-analysis-rewrite.md))
  are the playbook.

## Later

Things we believe matter but aren't actively planning. Items here serve
two purposes: signal to contributors that we'd accept a PR, and let us
say "not now" without saying "never."

- **UI migration to Topcoat** — migrate the dashboard from the current
  React web UI to [tokio-rs/topcoat](https://github.com/tokio-rs/topcoat).
  Topcoat is a TUI (terminal UI) framework from the tokio-rs ecosystem.
  This would move the agent dashboard from web-based to terminal-based,
  making z-Bot a fully terminal-native agent for users who prefer that.

## Not in scope

Things that have come up and that we've explicitly decided are *not*
in scope. This is the most valuable section for AI agents and new
contributors — it prevents wasted exploration of dead ends.

- **Rebuild agent engine from scratch.** Rig is the sole engine; no custom
  loop will replace it. The rig-only cutover is complete and stable.
- **New storage backends.** Engram + SQLite are the persistence layer; no
  SurrealDB, Postgres, or other backends. The engram adapter handles all
  storage policy (supersede, governance, lifecycle).

## How this file is maintained

- **Owners:** the maintainers (or the steering committee, if one exists).
- **Updates:** roadmap items move between sections via small PRs. Substantive
  additions or deletions go through an RFC.
- **Review cadence:** quarterly. The review updates the "Last updated" date
  even if no items change — fresh eyes, fresh dates.
- **Drift signal:** if items in "Now" haven't moved in two consecutive
  reviews, either they're not actually being worked on (move them out)
  or the roadmap doesn't reflect what the team is doing (rewrite it to
  match).
