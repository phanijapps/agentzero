---
name: book-reader
description: >
  Read and memorize long-form documents (.txt, .md, .epub, .pdf) as
  vault-ready Obsidian folders. Use when the user wants a book identified,
  chunked, summarized, and stored so it can be recalled later by chapter,
  character, theme, event, or quote — via Obsidian graph view, a future
  obsidian_query tool, or the main knowledge graph.
metadata:
  version: "0.2.0"
---

# Book Reader

Read books the way a careful human reader does: identify the work, scan
its structure, read chapter by chapter, take notes, and leave the vault
with a fully linked record — chapters, entity pages, and a book index
that Obsidian renders as a graph out of the box.

Do not load the entire book into working context at once. Work progressively.

Cross-skill output syntax (slug rules, wikilink targets, manual blocks)
is defined once in
[../_shared/obsidian_conventions.md](../_shared/obsidian_conventions.md).
The **exact required shape** of every book-reader output file is
inlined below — do NOT skip it or rely on the shared file alone. Past
runs produced sparse output because the shared file was treated as
optional; the shape rules in this SKILL.md are non-optional.

## Use when

- The user asks to read, summarize, memorize, annotate, quote from, or
  catalog a book or long document
- The user wants a document added to a durable knowledge base
- The user asks what was previously read from a known title or source

## Core contract — exact inventory

Every successful ingestion produces **exactly** this folder, in the ward:

```
books/<slug>/
├── _index.md                                 # book MOC (frontmatter + chapter/character/theme/event wikilink lists)
├── chunks/ch-NN.md                           # one Markdown file per chapter, frontmatter + verbatim text
└── entities/<type>-<name>.md                 # one page per within-book entity (character, theme, event, place, concept)
```

And exactly one main-graph `ingest` call at the end — see
"Main-graph ingest" below.

**Do NOT produce:**
- `book.json` — its content belongs in `_index.md` frontmatter + body.
- `book.kg.json` or any other `*.kg.json` — the graph is the linked
  markdown. Wikilinks ARE the graph.
- Per-section graph JSON.
- Any file outside `books/<slug>/`.

If a chapter is too long for one Markdown file, split it sequentially
(`ch-03a.md`, `ch-03b.md`) — they share `chunk_num: 3`.

## Workflow

### 1. Check for prior ingestion

Look up the book by title-derived slug. If `books/<slug>/_index.md`
already exists, switch to retrieval behavior and do not re-ingest.

### 2. Identify the work

Extract title, author, publication info, language, source from the
document's OWN metadata — never from the filename:

- EPUB: Dublin Core in the OPF (`<dc:title>`, `<dc:creator>`, …). See
  `references/epub.md`.
- PDF: metadata dictionary (`/Title`, `/Author`); fall back to first-page
  scan. See `references/pdf.md`.
- TXT / MD: first ~2KB for a title line (leading `#`, ALL CAPS,
  Gutenberg header). See `references/txt.md`.

If no title is extractable, leave the field `null` and abort ingestion —
do NOT guess from the filename.

### 2a. Derive the slug

Generate a kebab-case slug from the real title:
- Lowercase, non-alphanumeric → `-`, collapse runs, strip leading/trailing `-`.
- Strip leading `the`/`a`/`an`.
- Cap at ~60 chars at a word boundary.

The slug is both the folder name (`books/<slug>/`) and the `<source-slug>`
used in frontmatter and entity slugs
(`character-<name>`, `theme-<name>`, etc.). Filenames like `pg30254.epub`
are NEVER the slug. If a folder for the same slug already exists, treat
the book as ingested (Step 1).

### 3. Find the reading skeleton

Split the document body into chapters (or a single implicit chapter if
no chapter structure exists). Preserve reading order and stable source
boundaries. See `references/chunking.md`.

### 4. Read and annotate progressively

Read one chapter at a time. Write each chapter as `chunks/ch-NN.md`.
Filename format is **exactly** `ch-NN.md` (zero-padded 2-digit number,
no title in the filename) so wikilink targets from `_index.md` like
`[[ch-01|Chapter 1 — The Novice]]` resolve.

**Chunk frontmatter (all fields required — no omissions):**

```yaml
---
source: <book-slug>                    # e.g. romance-of-lust
part_of: "[[_index]]"                  # backref to the book index
chunk_num: 1
chunk_title: "Chapter 1 — The Novice"
line_start: 1
line_end: 500
summary: "2–4 sentences capturing what happens in the chapter."
key_ideas:
  - "<idea 1>"
  - "<idea 2>"
tags:
  - <source-slug>
  - <theme-tag>                        # at least 2 tags
mentions:
  people: [character-charlie, character-mary, character-eliza]
  places: [place-schoolroom, place-garden]
  concepts: [concept-sexual-awakening]
quotes:
  - text: "<verbatim passage>"
    line: 120
questions:
  - "<question the text raises>"
---
```

`mentions.people`, `mentions.places`, `mentions.concepts` — list the
**slug** of every entity page the chapter references. These slugs MUST
match files you write in `entities/`. Minimum: every named entity that
appears three or more times or drives the chapter's action.

**Chunk body — every section is required. Omit none:**

```markdown
# Chapter N — <title>

## Summary

<2–4 sentences, same as the frontmatter `summary` field, expanded
slightly if needed.>

## Full Text

<verbatim chapter text with known-entity mentions rewritten to
wikilinks. First mention per paragraph only. Rule:
"Elizabeth" → "[[character-elizabeth-bennet|Elizabeth]]".
Skip inside frontmatter, code fences, existing wikilinks, quoted
prose (lines starting with `>`), and `<!-- manual -->` blocks.>

## Quotes

> "<verbatim passage>"
— line 120

> "<another passage>"
— line 342

## Questions

- <question 1 the text raises>
- <question 2>
```

The body MUST contain ≥ 3 wikilinks to entity pages you write in
`entities/`. A chunk body with zero wikilinks is a failure — go back
and rewrite the mentions.

### 5. Write entity pages

For every within-book entity surfaced across the chapters — character,
theme, event, place, concept, organization (fictional or in-story) —
write one `entities/<type>-<slug>.md` page.

Filename format is **exactly** `<type>-<kebab-slug>.md`. Type prefixes:
`character-`, `theme-`, `event-`, `place-`, `concept-`,
`organization-`. The filename matches the wikilink target exactly:
`[[character-elizabeth-bennet|Elizabeth]]` resolves to
`entities/character-elizabeth-bennet.md`.

**Entity frontmatter (all fields required):**

```yaml
---
title: "Elizabeth Bennet"
type: character
slug: character-elizabeth-bennet
source: "[[_index]]"                   # backref to the book index
aliases: [Lizzy, Miss Bennet, Eliza]
tags: [character, romance-of-lust]
---
```

**Entity body — use these section headings exactly. Omit empty sections:**

```markdown
# <Display name>

<one-sentence description capturing role + relationship to the book>

## Relationships

- loves: [[character-fitzwilliam-darcy|Fitzwilliam Darcy]]
- sister of: [[character-jane-bennet|Jane Bennet]]
- daughter of: [[character-mr-bennet|Mr Bennet]]

## Mentioned in

- [[ch-01]] lines 10, 120
- [[ch-05]] lines 12, 340, 510
- [[ch-34]] line 234

## Evidence

> "<verbatim passage featuring this entity>"
— [[ch-34]] line 510

> "<another passage>"
— [[ch-05]] line 342
```

**Typed-relationship bullet format — one shape only:**

```
- <relation>: [[<target-slug>|<display>]]
```

- `<relation>` is lowercase, space-separated (`loves`, `sister of`,
  `daughter of`, `enemy of`, `works for`).
- `<target-slug>` MUST be the filename (without `.md`) of another
  entity page this same run writes. Dangling targets = broken graph.
- `<display>` is optional; rendered text Obsidian shows in-line.

Evidence section bullets must cite the chunk and line:
`— [[ch-NN]] line <N>` (wikilink target `ch-NN` resolves to
`chunks/ch-NN.md`).

Minimum targets for a novel-length work: ≥ 8 character pages,
≥ 4 theme pages, ≥ 5 event pages, ≥ 10 typed relationship bullets across
those pages, each with populated `## Evidence`. Under that count, go
back — the vault isn't done until the graph reflects the book.

### 6. Write `_index.md`

After every chunk + entity page is written, assemble `books/<slug>/_index.md`.
This is the book's MOC (Map of Content) — the entry point Obsidian opens
when the user clicks into the book folder.

**`_index.md` frontmatter (all fields required — set `null` for unknown
metadata, never guess):**

```yaml
---
title: "<Title>"
type: book
slug: <book-slug>
author: "<Author or null>"
published: "YYYY or null"
language: en
thesis: "One-sentence statement of what this book is about."
tags: [book, <genre-tag>, <era-tag>, ...]
aliases: ["<alternate title>"]
date_read: 2026-04-16
source: "<original file path or URL>"
---
```

**`_index.md` body — four required sections, each non-empty:**

```markdown
# <Title>

> <one-paragraph synopsis capturing the book's arc and central idea>

## Chapters

- [[ch-01|Chapter 1 — The Novice]]
- [[ch-02|Chapter 2 — Mrs Benson]]
- [[ch-03|Chapter 3 — Mrs Egerton]]
...

## Characters

- [[character-charlie|Charlie]] — protagonist-narrator
- [[character-mary|Mary]] — elder sister
- [[character-eliza|Eliza]] — younger sister
...

## Themes

- [[theme-sexual-awakening|Sexual awakening]]
- [[theme-victorian-hypocrisy|Victorian hypocrisy]]
...

## Key events

- [[event-mrs-benson-introduction|Mrs Benson's introduction]]
- [[event-miss-franklands-arrival|Miss Frankland's arrival]]
...

<!-- manual -->
<!-- /manual -->
```

**Wikilink target rule (the #1 failure mode — READ THIS):**

Every `[[link]]` target MUST match a filename that exists in this
run's output:

- Chapter links: target = `ch-NN` (e.g. `[[ch-01|Display]]` →
  `chunks/ch-01.md`). Do NOT use human-readable strings like
  `[[Ch 01 — Volume I — The Novice]]` — they don't resolve.
- Entity links: target = `<type>-<slug>` (e.g.
  `[[character-charlie|Charlie]]` → `entities/character-charlie.md`).
  Do NOT use `[[Charlie]]` or `[[Mary]]` — broken.
- Display text goes after `|`. Omit if the slug is already readable
  (`[[theme-pride]]` renders as "theme-pride" — usually you want a
  display alias).

### 7. Main-graph ingest

At the end of the run, call the `ingest` tool exactly once. The payload
carries **one book-summary entity plus one entity per cross-source real
entity** surfaced in the book.

**Within-source entities (fictional characters, in-book themes, in-book
events) do NOT go to main KG.** They live only in the vault. A
within-source entity is one whose identity exists only inside this book
(Elizabeth Bennet, Netherfield ball, Austen's in-book "pride" motif).

**Cross-source entities (real people, real organizations, real places,
public works, named concepts) DO go to main KG.** A cross-source entity
exists outside this book and could appear in other books, articles, or
research (Jane Austen the author, East India Company, London, transformer
architecture, Newtonian mechanics).

Rule of thumb: if the entity has a Wikipedia page or could plausibly have
one, it's cross-source.

Payload shape:

```json
{
  "entities": [
    {
      "id": "book:<slug>",
      "name": "<Title>",
      "type": "book",
      "properties": {
        "slug": "<slug>",
        "author": "<Author or null>",
        "published": "YYYY or null",
        "language": "en",
        "thesis": "one sentence",
        "tags": ["..."],
        "chapter_count": 34,
        "vault_path": "books/<slug>/_index.md",
        "chunk_dir": "books/<slug>/chunks/",
        "entities_dir": "books/<slug>/entities/",
        "character_count": 12,
        "theme_count": 5,
        "event_count": 8,
        "notable_quote": {"text": "...", "chunk_file": "chunks/ch-01.md", "line": 1}
      }
    },
    {
      "id": "person:jane-austen",
      "name": "Jane Austen",
      "type": "person",
      "properties": {
        "vault_path": "books/<slug>/entities/person-jane-austen.md",
        "role_in_book": "author",
        "evidence": [{"chunk_file": "chunks/ch-00.md", "line": 1}]
      }
    }
  ],
  "relationships": []
}
```

The main-graph entities are summary-level pointers — detail lives in the
vault. Relationships stay empty at the main-graph layer: cross-source
relationships across multiple books accumulate naturally as each book
adds its own `person:jane-austen` entity and properties merge.

### 8. Store memory facts

Save `memory_facts` for fast recall across sessions:

- **One** `domain` fact keyed `domain.<slug>.summary` — title, author,
  thesis, tag list. Default scope.
- Per-entity facts for the **most important** within-book entities
  (protagonists, major themes, climactic events) — scope `global`, keys
  `domain.<slug>.character.<kebab-name>`, `domain.<slug>.theme.<kebab-name>`,
  etc. One sentence each. Do NOT save every character — just the ones a
  user would plausibly ask about by name in a future session.

## Rules

1. Always check for existing ingestion first — look up by title-slug.
2. Filename is NEVER authoritative. Extract metadata from the document
   itself. `books/pg30254/` is wrong; `books/romance-of-lust/` is right.
3. Never load the whole book into working context.
4. Every chunk body includes verbatim text under `## Full Text` — no
   summary-only chapter files.
5. Every entity page must exist for every slug referenced in any
   relationship bullet, `## Characters`, `## Themes`, or `## Key events`
   list. No broken wikilinks.
6. Every section in the shared conventions uses its fixed heading
   vocabulary — do not invent new headings.
7. Typed relationships use the one bullet shape: `- <relation>: [[target-slug]]`.
   No Dataview syntax.
8. The `ingest` call is exactly once, at the end, with one book entity
   plus any cross-source real entities. Fictional characters stay in the
   vault only.
9. Prefer `null` over fabricated metadata.
10. If the document is image-only or OCR is required, hand off to an
    OCR-capable path first, then continue.

## Retrieval behavior

When the user asks about a previously read work, answer from the vault:

- Book list / "did we read X?" — scan `books/*/_index.md` frontmatter
  (title, aliases, slug).
- Chapter content — read `books/<slug>/chunks/ch-NN.md`.
- Character / theme / event — read
  `books/<slug>/entities/<type>-<slug>.md`.
- Cross-book entities — use injected knowledge graph context when present, or
  search the book/entity vault indexes for `vault_path` references.

Don't re-ingest. Don't re-chunk.

## Acceptance checklist — run before declaring done

Walk through every item before you return. If any is false, go back and
fix it — do not ship a partial vault.

**File inventory**
- [ ] `books/<slug>/_index.md` exists.
- [ ] `books/<slug>/chunks/ch-NN.md` exists for every chapter in the
      source, with zero-padded numbers (`ch-01.md`, not `ch-1.md`).
- [ ] `books/<slug>/entities/<type>-<slug>.md` exists for every entity
      mentioned in any chunk's `mentions:` frontmatter list or any
      entity wikilink in `_index.md`.

**Frontmatter completeness**
- [ ] `_index.md` frontmatter has all 11 fields (title through source).
      No field is `""` or omitted; unknown metadata is literally `null`.
- [ ] Every chunk's frontmatter has all 11 fields (source through
      questions). `mentions.people` / `.places` / `.concepts` list
      **entity slugs**, not display names.
- [ ] Every entity page's frontmatter has all 6 fields (title through
      tags).

**Wikilink resolution — the biggest failure mode**
- [ ] Every `[[...]]` target in `_index.md` resolves to a file in this
      run's output (check each target against the file inventory).
- [ ] Every `[[...]]` target in every chunk's `## Full Text` resolves.
- [ ] Every `[[...]]` target in every entity's `## Relationships`,
      `## Mentioned in`, `## Evidence` resolves.
- [ ] No wikilinks use prose strings: `[[Charlie]]`, `[[Mary]]`,
      `[[Ch 01 — Volume I — The Novice]]` are all WRONG. Use
      `[[character-charlie|Charlie]]`, `[[character-mary|Mary]]`,
      `[[ch-01|Chapter 1 — The Novice]]`.

**Body richness**
- [ ] Every chunk body has `## Summary`, `## Full Text`, `## Quotes`,
      `## Questions` sections. None is empty.
- [ ] Every chunk's `## Full Text` contains ≥ 3 wikilinks to entity
      pages.
- [ ] Every entity page has `## Relationships` (≥ 1 typed bullet),
      `## Mentioned in` (≥ 1 chapter wikilink), `## Evidence` (≥ 1
      quoted passage with line citation).

**Graph minimums (novel-length work)**
- [ ] ≥ 8 character pages, ≥ 4 theme pages, ≥ 5 event pages.
- [ ] ≥ 10 typed relationship bullets across all entity pages.

**Main-KG ingest + memory**
- [ ] Exactly one `ingest` call, with `book:<slug>` entity.
- [ ] Cross-source real entities (author, real orgs/places) included
      in the same call.
- [ ] One `domain.<slug>.summary` memory fact written.
- [ ] Global-scope memory facts for ≥ 3 most important within-book
      entities.
