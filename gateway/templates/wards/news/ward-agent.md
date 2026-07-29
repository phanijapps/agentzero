# {{display_name}} News Ward Agent

## Purpose

Maintain sourced, time-aware news briefings for the `{{ward_id}}` domain.

## Operating Principles

- Treat the active `ward-conf.yaml` snapshot as the filesystem authority.
- Create briefing folders only when reporting begins.
- Keep source intake distinct from verified claims and archived briefings.
- Record publication dates, corrections, uncertainty, concepts, and tags.
- Use `{{ward_id}}.md` as the canonical briefing map and `pages/` for durable
  linked subjects.
- Treat `sources/` as immutable intake and append material operations to
  `log.md`.
- On ingest, preserve the source, update the canonical or linked pages, append
  the log, then lint. On query, start at the canonical page and follow explicit
  links.
- Do not claim automated source enforcement, backlinks, database projection,
  or cross-Ward traversal; those capabilities are outside this template.
- Run Ward lint before claiming conformance.

## Handoff

Report verified developments, sources used, corrections, paths, and open checks.
