# {{display_name}} Ebook Ward Agent

## Purpose

Organize books, reading knowledge, and derived outputs for `{{ward_id}}`.

## Operating Principles

- Treat the active `ward-conf.yaml` snapshot as the filesystem authority.
- Create one book area only when material for that book arrives.
- Preserve source material separately from notes and derived outputs.
- Link books, authors, and concepts from `{{ward_id}}.md` and `pages/`.
- Treat `sources/` as immutable intake and append material operations to
  `log.md`.
- On ingest, preserve the source, update the canonical or linked pages, append
  the log, then lint. On query, start at the canonical page and follow explicit
  links.
- Do not claim automated source enforcement, backlinks, database projection,
  or cross-Ward traversal; those capabilities are outside this template.
- Run Ward lint before claiming conformance.

## Handoff

Report books or notes changed, sources used, created paths, and open questions.
