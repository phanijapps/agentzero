# Research archetype contract (shared)

Every research-producer skill (`stock-analysis`, `news-research`,
`product-research`, `competitive-analysis`, `academic-research`,
`market-research`, `technical-research`, `policy-research`, and any
future archetype) follows this contract. Each archetype's SKILL.md
declares only the differences: activation triggers, subject slug
convention, typical artifact filenames, and cross-source ingest profile.

Everything structural lives here.

## Ward folder layout

```
research/<archetype>/<subject>/<date-slug>/
├── _index.md                 # session MOC
├── <artifact-1>.md           # flat — no subfolders under the session
└── <artifact-N>.md
```

- `<archetype>` — the producer skill's name, kebab-case.
- `<subject>` — what was researched: ticker, topic, product, industry,
  paper family, policy. Kebab-case. Defaults to the ward slug when the
  archetype skill can't derive something more specific.
- `<date-slug>` — ISO date, optionally with a user-meaningful suffix
  (`2026-04-16`, `2026-04-16-q1`, `2026-04-16-pre-earnings`,
  `2026-04-16-morning`). Same-day collision on the same subject appends
  `-2`, `-3`, … so each run is its own folder.

## Vault destination

`wiki` moves `research/` → `40_Research/` whole-tree. No router changes
needed per archetype; the nesting is preserved.

## `_index.md` shape

Frontmatter per [`obsidian_conventions.md`](obsidian_conventions.md),
with these research-specific keys:

```yaml
---
title: "<Session title>"
type: research
archetype: <archetype>
subject: <subject>
date_slug: <date-slug>
date_conducted: 2026-04-16
tags: [research, <archetype>, <subject-tag>, …]
aliases: [...]
source: "ward: <ward-name>"
---
```

Body (omit any empty section — do not leave empty headings):

```markdown
# <Session title>

> Synopsis — thesis and verdict in one paragraph.

## What was done

- <process step>
- <process step>

## Outputs

- [[<artifact-1>]] — one-liner
- [[<artifact-N>]] — one-liner

## Key findings

- <finding>
- <finding>

<!-- manual -->
<!-- /manual -->
```

## Artifact files

Flat under the session folder (no subfolders). Filename is kebab-case
`.md`, decided by the agent based on content (archetype SKILL.md lists
typical examples). Frontmatter:

```yaml
---
title: "<Artifact title>"
type: report
archetype: <archetype>
subject: <subject>
date_slug: <date-slug>
source: "ward: <ward-name>"
date_generated: 2026-04-16
tags: [<archetype>, <subject-tag>]
---
```

Body is the artifact content — no forced section vocabulary. Wikilink
rewriting is optional (research artifacts are often figure-and-table
heavy; forced rewriting hurts readability).

## Main-KG ingest — one call at the end

Payload:

- **Exactly one** `research:<archetype>:<subject>:<date-slug>` summary
  entity. Properties include `vault_path` → `_index.md`, `archetype`,
  `subject`, `date_slug`, `date_conducted`, `thesis`, `artifacts` list,
  `tags`.
- **Archetype-specific cross-source entities** — each archetype declares
  which real-world entities it ingests (organizations, people, products,
  works, policies). IDs use the standard `<type>-<kebab-slug>` scheme so
  entities collapse across sessions (`organization-tesla-inc` is ONE
  node whether it came from `stock-analysis/tsla/*` or
  `news-research/tsla/*`).
- **Optional relationships** — typed edges from the session summary to
  its subjects are allowed and encouraged:
  - `about: <entity-id>` — the session is about this entity
  - `mentions: <entity-id>` — entity appears but isn't the subject
  - `cites: <entity-id>` — source material cited (news-research,
    academic-research)

Relationships carry `properties.evidence` pointing at the artifact or
`_index.md` lines where the entity features.

## Memory facts

- One `domain.research.<archetype>.<subject>.<date-slug>.summary` fact
  (default scope) — title, thesis, key findings.
- Global-scope facts for findings durable beyond this snapshot, keyed
  `domain.research.<archetype>.<subject>.finding.<kebab-name>`. Examples:
  `domain.research.stock-analysis.tsla.finding.margin-compression-thesis`.
- Skip ephemeral per-session numbers (Q1 2026 gross margin 17.3%) —
  those live in the artifact file, not in memory.

## Retrieval

Find prior snapshots of a subject:
- Use injected knowledge graph/context resources for
  `research:<archetype>:<subject>:*` when present — they return dated
  snapshot summary nodes with `vault_path` properties.
- Or vault walk: `40_Research/<archetype>/<subject>/*/_index.md`.

Find all archetype activity on a real entity (e.g. "every session that
mentioned Tesla"):
- Use injected neighbor/entity context when present; otherwise search the
  research vault indexes for the entity. The `organization-tesla-inc` node
  accumulates `about` / `mentions` edges from every research session.

## Retention

Each snapshot is durable — do not overwrite or prune prior dated
snapshots programmatically. The user decides when to archive
(`60_Archive/` manually).

## What research archetypes do NOT do

- Do not decompose the subject into entity pages inside the session
  folder. (That's book-reader territory — fictional characters, themes,
  events. Research is session-shaped: process + outputs + findings.)
- Do not emit `*.kg.json` or per-artifact graph JSON. The session
  summary entity + cross-source entities go to the main KG; everything
  else is prose.
- Do not rename or reshape artifact files once written — they are the
  session's record.
- Do not edit memory-bank, AGENTS.md, or any ward infrastructure.
