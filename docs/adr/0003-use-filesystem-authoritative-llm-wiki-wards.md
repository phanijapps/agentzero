# ADR-0003: Use Filesystem-Authoritative LLM Wiki Wards

- **Status:** Accepted
- **Date:** 2026-07-28
- **Deciders:** phanijapps, zbot maintainers
- **Supersedes:** ADR-0001
- **Related:** [RFC-0018](../rfc/0018-filesystem-authoritative-llm-wiki-wards.md), [RFC-0016](../rfc/0016-generic-ward-configuration-and-layout-resolution.md), [ADR-0002](0002-select-complete-ward-archetypes-at-creation.md), LLM Wiki Ward Foundation

## Context

AgentZero needs durable Ward knowledge that humans and agents can inspect,
version, link, and repair. ADR-0001 correctly made each Ward's validated,
versioned `ward-conf.yaml` snapshot the structural authority, but it also
selected strict OKF profiles, recursive indexes, and a fixed concept companion
tree. Experience with compact archetypes showed that those mandatory
containers create more structure than the knowledge warrants. Separately,
database Wiki articles can diverge from the files agents and users actually
edit.

The replacement must retain snapshot isolation, closed local archetypes,
filesystem confinement, bounded data-only templates, atomic publication, and
non-destructive treatment of existing Wards. It must also give knowledge one
writable source of truth, keep fresh Wards compact, support ordinary Markdown,
and permit derived search, tags, aliases, and backlinks without making those
projections authoritative.

## Decision

> We will use confined Ward Markdown files as the authoritative LLM Wiki and retain each Ward's versioned `ward-conf.yaml` snapshot as its structural authority.

The configured wards root contains the sole `index.md`, which catalogs Wards
with links to their canonical pages. Every fresh Ward begins with exactly one
canonical `<ward-id>.md` page, `AGENTS.md`, append-only `log.md`, and its
validated `ward-conf.yaml` snapshot. Optional `pages/`, `sources/`, archetype
work areas, and `.zbot/specs/<concept>/` paths remain lazy.

Ordinary UTF-8 Markdown is valid without mandatory OKF metadata. A later Wiki
operations slice may recognize a bounded optional property vocabulary and
derive tags, aliases, forward links, backlinks, and health findings. Raw
sources remain a distinct provenance layer. Database Wiki/search records,
embeddings, and graph data are disposable projections rebuilt from the
filesystem; they are never a second writable authority.

The retained `ward-conf.yaml` contract continues to be validated and copied
exactly at creation. It declares permitted structure but does not prescribe a
recursive navigation index or mandatory document profile. New Wards use
validated lowercase canonical slugs. Existing Wards are not migrated by this
decision; acceptance uses deleted Wards and a fresh database.

## Consequences

**Positive:**

- Knowledge has one portable, inspectable, versionable writable authority.
- Fresh Wards remain useful with four root files and no empty work trees.
- Human-readable Markdown and wikilinks work without mandatory OKF ceremony.
- Search, tags, aliases, and backlinks can be rebuilt and improved without
  changing durable content.
- Snapshot isolation and the existing filesystem security boundary remain in
  force.

**Negative:**

- Database projection freshness and rebuild behavior require follow-on work.
- Link/property parsing, safe multi-file edits, provenance enforcement, and
  conflict handling must be specified before mutation features ship.
- A clean break intentionally provides no automatic legacy-Ward migration.
- Complete archetype bundles duplicate small layout declarations because
  inheritance remains forbidden.

**Neutral / to revisit:**

- This decision does not select a UI, graph visualization, ranking algorithm,
  or frontmatter parser.
- ADR-0002's closed seven-archetype selection remains unchanged.

## Alternatives considered

- **Retain mandatory OKF.** Rejected because recursive indexes and required
  metadata produced disproportionate scaffolding and made useful plain
  Markdown invalid.
- **Make the database primary.** Rejected because it hides durable knowledge
  from ordinary filesystem workflows and preserves two divergent editing
  surfaces.
- **Treat filesystem and database as co-equal authorities.** Rejected because
  bidirectional synchronization creates ambiguity, conflicts, and partial
  failure modes.
- **Allow unconstrained Markdown and paths.** Rejected because structural
  confinement, snapshot isolation, and bounded templates are essential safety
  controls even when document metadata is relaxed.

## References

- [RFC-0018: Filesystem-Authoritative LLM Wiki Wards](../rfc/0018-filesystem-authoritative-llm-wiki-wards.md)
- [Karpathy: LLM Wiki](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f)
