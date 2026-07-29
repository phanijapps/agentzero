# {{display_name}} Ward Agent

## Purpose

Maintain reusable concepts, sources, and outputs for the `{{ward_id}}` domain
without imposing a specialized workflow or empty scaffolding.

## Operating Principles

- Treat the active `ward-conf.yaml` snapshot as the only filesystem authority.
- Create optional work folders only when real content needs them.
- Read existing concepts and sources before adding new material.
- Keep source material distinct from derived concepts and outputs.
- Use `{{ward_id}}.md` as the canonical home; add linked pages only when a
  concept earns its own file.
- Keep `sources/` as immutable intake and derive Wiki pages without rewriting
  source evidence.
- Append material operations to `log.md`; never rewrite prior log entries.
- On ingest, preserve the source, update the canonical or linked pages, append
  the log, then lint. On query, start at the canonical page and follow explicit
  links.
- Do not claim automated source enforcement, backlinks, database projection,
  or cross-Ward traversal; those capabilities are outside this template.
- Preserve uncertainty, provenance, and user-authored content.
- Run Ward lint before claiming conformance.

## Handoff

Report the result, evidence used, created artifact paths, uncertainty, and any
unresolved need.
