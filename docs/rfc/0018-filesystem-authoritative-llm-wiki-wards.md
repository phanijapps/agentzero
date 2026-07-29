# RFC-0018: Filesystem-Authoritative LLM Wiki Wards

- **Status:** Accepted
- **Author:** phanijapps
- **Approver:** phanijapps
- **Date opened:** 2026-07-28
- **Date closed:** 2026-07-28
- **Related:** RFC-0015, RFC-0017, ADR-0001, ADR-0002, `docs/specs/okf-ward-foundation/`, `docs/specs/okf-mini-obsidian-ui/`, `docs/specs/okf-ward-tool-capabilities/`, `docs/specs/bundled-ward-archetypes/`

## The ask

Approve a clean-break change from strict Open Knowledge Format (OKF) Ward
documents to filesystem-authoritative LLM Wiki Wards. Keep the versioned,
data-only `ward-conf.yaml` snapshot and the closed archetype registry, but make
ordinary interlinked Markdown the durable knowledge surface. Retain one global
`wards/index.md` catalog; replace every Ward-local and nested `index.md` with a
single canonical `<ward-id>.md` page; keep raw sources immutable; derive search,
tags, aliases, and backlinks from the files; and treat SQLite/Engram Wiki data
as a rebuildable index rather than a second source of truth.

RFC-0015 selected strict recursive OKF to unify fragmented Ward knowledge.
Since then, archetype work has shown that mandatory indexes and frontmatter
create structure faster than knowledge, while a failed journal run compiled 53
source days into one document because conformance described containers rather
than the maintenance workflow. AgentZero also already has a Karpathy-style
compiler, but it writes articles into `WikiStore` while agents write a separate
filesystem tree. The question is no longer whether Wards need durable
knowledge; it is which copy is authoritative and how little structure is
needed for that knowledge to compound.

Decisions requested:

1. Make Ward Markdown the authoritative wiki and make database search/recall
   records disposable projections. Recommended: accept; one writable truth
   prevents disk/database divergence. Owner: phanijapps. Decide by 2026-07-28.
2. Retain `wards/index.md` as the sole `index.md`, and give every Ward one
   required `<ward-id>.md` canonical page. Recommended: accept; links and search
   results carry meaningful identities without recursive navigation files.
   Owner: phanijapps. Decide by 2026-07-28.
3. Replace mandatory OKF metadata with ordinary Markdown and an optional,
   bounded property vocabulary: `tags`, `aliases`, `created`, `updated`, and
   `sources`. Standardize `[[wikilinks]]`; derive backlinks from forward links.
   Recommended: accept; it remains human-readable and Obsidian-compatible
   without rejecting useful prose for missing metadata. Owner: phanijapps.
   Decide by 2026-07-28.
4. Adopt three Ward knowledge layers: immutable raw sources, LLM-maintained
   wiki pages, and `AGENTS.md` maintenance doctrine. Add an append-only
   `log.md`. Recommended: accept; the layers have different owners and mutation
   rules. Owner: phanijapps. Decide by 2026-07-28.
5. Cut over using deleted Wards and a fresh database, with no legacy content
   migration in the first release. Recommended: accept; it matches the
   approved acceptance environment and avoids preserving a model being
   deliberately replaced. Owner: phanijapps. Decide by 2026-07-28.
6. Deliver the change in four independently shippable slices: filesystem
   model, wiki operations, storage consolidation, and UI. Recommended: accept;
   the authority boundary is decided up front without forcing one high-risk
   implementation. Owner: phanijapps. Decide by 2026-07-28.

## Problem & goals

The current product has two knowledge models:

- Ward files are governed by a versioned layout snapshot and increasingly
  strict OKF frontmatter and `index.md` conventions.
- `compile_ward_wiki` independently synthesizes database `WikiArticle` rows
  and a special `__index__` article used by recall and the Ward content API.

Neither is a projection of the other. A user can inspect and edit the Ward
without changing compiled Wiki articles, while distillation can update the
database without producing a durable file. Recursive index requirements also
encourage agents to scaffold folders and catalogs before the work needs them.
That conflicts with the compact archetype goal and makes an ordinary Markdown
page invalid solely because it lacks an OKF `type`.

Goals:

- Give humans and agents one inspectable, portable, versionable knowledge
  surface.
- Preserve the existing safe, local, data-only Ward Layout snapshot as
  structural authority.
- Preserve the seven complete creation-time archetypes and allow each to
  specialize optional knowledge and work areas.
- Keep new Wards compact while providing one unambiguous canonical page.
- Make source immutability, ingest, query, maintenance, and lint responsibilities
  explicit in Ward doctrine.
- Support useful tags and aliases without making frontmatter mandatory.
- Calculate backlinks, broken links, and orphans from explicit forward links.
- Let FTS, vector search, embeddings, and recall accelerate the filesystem
  without becoming an independently writable knowledge base.
- Keep `.zbot/specs/<concept>/spec.md`, `plan.md`, and `tasks/` hidden from the
  human knowledge surface while remaining available to execution workflows.

Non-goals:

- Defining a universal ontology or closed tag taxonomy.
- Replacing the data-only `ward-conf.yaml` language or snapshot digest model.
- Reclassifying or rewriting existing Wards when a later ask has a different
  type.
- Building automatic conversion for existing OKF Wards or database Wiki rows
  in the first release.
- Requiring embeddings, an MCP server, Obsidian, Git, or any proprietary
  service to navigate a small Ward.
- Shipping the full graph/editor UI in the filesystem-model slice.
- Treating model-written semantic judgments such as contradiction or staleness
  as hard structural errors.

## Proposal

### Authority and layers

The configured filesystem is authoritative:

```text
<vault>/wards/
├── index.md
└── <ward-id>/
    ├── <ward-id>.md
    ├── AGENTS.md
    ├── log.md
    ├── ward-conf.yaml
    ├── sources/
    ├── pages/
    ├── <archetype-specific optional areas>/
    └── .zbot/
        └── specs/
            └── <concept>/
                ├── spec.md
                ├── plan.md
                └── tasks/
```

Only the first four Ward files are required at creation. Optional directories
are created on first use. `sources/` contains user-curated raw inputs and is
immutable to the LLM after ingest. `pages/` is the default maintained
knowledge namespace; an archetype may additionally declare meaningful
namespaces such as journal `entries/YYYY/`, research `subjects/`, news
`briefings/`, or coding `docs/`. `AGENTS.md` defines ownership and workflows.
`log.md` records ingests, filed queries, and lint passes using parseable dated
headings.

The filesystem page is the only durable article. Database rows, embeddings,
FTS records, link edges, tag indexes, and recall projections must be
reconstructable from files. A projection may lag temporarily and report that
state, but it must never silently overwrite a newer file.

### Canonical identity and navigation

`wards/index.md` remains a product-managed global catalog. It links to each
Ward's canonical page:

```markdown
- [[financial-analysis/financial-analysis|Financial Analysis]]
```

Each Ward has exactly one required `<ward-id>.md`; it is the Ward overview,
curated navigation surface, and natural cross-Ward link target. A Ward contains
no other reserved `index.md`. Specifications and task folders use their
semantic filenames (`spec.md`, `plan.md`, and task names) rather than nested
indexes.

Page identity is its Ward-relative path without the `.md` suffix. Optional
`aliases` provide alternate human names but do not replace path identity.
Path-qualified links resolve deterministically. An unqualified link resolves
only when exactly one page or alias matches inside the active Ward; ambiguity
is a lint finding, not an arbitrary choice.

Generated page and directory components use canonical lowercase ASCII slugs:
1–64 letters, numbers, hyphens, or underscores; they cannot start with a dot,
equal `.` or `..`, contain separators, or collide under case folding. Reserved
infrastructure names—including `AGENTS.md`, `ward-conf.yaml`, `log.md`,
`index.md`, `.zbot`, and archetype-declared source roots—cannot be claimed as
page identities. Every page creation and link target is resolved without
following symlinks and must remain canonically beneath the active Ward before
any read or write. Human display names and non-ASCII names belong in the H1 and
`aliases`, not filesystem identity.

Cross-Ward links may appear as explicit navigation, but the resolver marks
them external. Ingest, query, recall, lint expansion, and projection do not
follow an external-Ward target unless the active request explicitly names that
cross-Ward scope or a user approves it. The global Ward catalog may link to
canonical pages for navigation without granting an agent authority to read
their contents.

### Markdown and metadata

Knowledge files are UTF-8 Markdown. YAML frontmatter is optional. When present,
the first slice recognizes only enough structure to support navigation:

```yaml
---
tags:
  - valuation
aliases:
  - DCF
created: 2026-07-28
updated: 2026-07-28
sources:
  - "[[sources/damodaran-valuation]]"
---
```

Unknown properties are preserved. Known properties are bounded and
type-checked when present; the absence of frontmatter is valid. Forward
`[[wikilinks]]` are stored in Markdown. Backlinks are derived from those links
and are not independently authoritative. Generated backlink sections, if a
future UI or export needs them, must be delimited and replaceable.

All page parsing is resource-bounded. A Markdown page retains the existing
1 MiB read ceiling; frontmatter is limited to 64 KiB and eight nested YAML
levels; `tags`, `aliases`, and `sources` each contain at most 32 scalar values;
and a page contributes at most 1,024 resolved links. YAML tags, duplicate
mapping keys, non-scalar recognized values, alias/path case-fold collisions,
and recursive alias resolution are rejected. Parser construction tests include
malformed and adversarial Markdown/YAML plus fuzz or property-based coverage.

### Security and publication boundaries

Raw sources, pages, frontmatter, aliases, link labels, and retrieved excerpts
are untrusted content. LLM operations wrap them in provenance-labelled data
boundaries; they are never concatenated into system, developer, doctrine, or
tool-authority instruction slots. Only the runtime policy and the active
Ward's validated `AGENTS.md` doctrine may direct the maintenance workflow.
Content cannot grant permissions, expand Ward scope, select tools, or override
source immutability.

Source immutability is enforced, not merely prompted. Initial ingest publishes
a source through a resolver-mediated, no-replace operation and records its
content digest. After publication, every model-visible z-Bot mutation surface,
including shell/write/edit adapters and connector/MCP publication paths, must
deny writes, renames, deletion, link substitution, or special-file replacement
beneath an immutable source root. A correction requires a distinct,
user-authorized flow that records the old and new digest in `log.md`. All
source operations reject symlinks, hardlink aliases, and non-regular files and
revalidate opened handles to limit check/use races.

Multi-file wiki changes use a bounded transaction plan. The plan records every
target's existence and preimage digest, stages new regular single-link files
inside the Ward without exposing them as active content, and aborts on any
digest, identity, epoch, or containment mismatch. Activation uses no-follow,
no-clobber publication plus a commit epoch/manifest so readers and projection
builders consume only the last complete set. A durable recovery record permits
rollback or completion after interruption. Database projections are rebuilt
only from the committed filesystem epoch and record its digest; they never
participate in committing content.

### Operations and lint

Every archetype doctrine defines the same lifecycle:

1. **Ingest:** preserve the raw source, inspect existing pages, update before
   duplicating, link affected pages, update the canonical page, and append the
   operation to `log.md`.
2. **Query:** read the canonical page or derived index, follow relevant pages
   and sources, answer with provenance, and optionally file durable new
   synthesis.
3. **Lint:** separate deterministic structure/link checks from semantic
   maintenance suggestions.

Hard structural findings include unsafe paths, symlinks, invalid UTF-8,
missing or multiple canonical pages, invalid known properties, broken or
ambiguous explicit links, source-root policy violations, and layout
violations. Wiki-health warnings include orphans, missing useful links,
possible duplicates, stale claims, contradictions, and source gaps.

The journal archetype additionally requires one
`entries/YYYY/YYYY-MM-DD.md` file per source day unless the user explicitly
requests a compilation. Coding retains native code and build layout; source
code is not forced into Wiki metadata.

### Storage consolidation and rollout

Delivery is staged:

1. **Filesystem model:** canonical page materialization, one global catalog,
   plain-Markdown archetype bundles, doctrine/log starters, structural lint,
   and fresh-vault tests.
2. **Wiki operations:** wikilink/property parsing, derived tags, aliases,
   backlinks, broken-link/orphan checks, and bounded ingest/update operations.
3. **Storage consolidation:** change `compile_ward_wiki` to plan and publish
   safe filesystem edits, rebuild `WikiStore` from those files, remove
   `__index__`, and retarget recall and Ward content APIs.
4. **UI:** canonical home, page browser/editor, source provenance, tag and
   backlink panels, graph navigation, and health findings.

Slice 1 must define the final authority contract but need not remove every old
database reader before it ships. It must not add a new database writer that
deepens the split. Until Slice 3, existing compiled Wiki data is legacy recall
state and cannot be presented as the authoritative Ward content.

Accepted RFCs and ADRs remain immutable history. On acceptance, RFC-0015 is
marked superseded, a new ADR supersedes the OKF-specific clauses of ADR-0001
while retaining versioned snapshots, and affected Draft specs are marked
`Archived`. Specifically, archive `okf-ward-foundation` and
`okf-mini-obsidian-ui`, update the active `bundled-ward-archetypes` spec to
remove OKF/index assumptions, and preserve `okf-ward-tool-capabilities` as a
Shipped historical contract. Changed behavior ships through the new follow-on
specs rather than rewriting that historical contract.

## Options considered

The option space is exhaustive along six independent axes: authoritative
storage, navigation identity, metadata strictness, operational layers,
transition policy, and delivery breadth. Combinations within those axes are
implementation variations, not additional decision categories.

### Authoritative storage

| Option | Trade-off |
| --- | --- |
| **Filesystem primary; database derived (recommended)** | Portable and inspectable with rebuildable acceleration; requires projection freshness and safe file publication. |
| Database primary; Markdown export | Central transactions and search are simpler, but human file edits and Git/Obsidian become secondary exports. |
| Coordinated dual authority | Both surfaces can write, but conflict resolution and partial failure become permanent product complexity. |
| Do nothing: OKF files plus independent WikiStore | Lowest immediate cost; preserves the divergence and user confusion this RFC addresses. |

### Navigation identity

| Option | Trade-off |
| --- | --- |
| Recursive `index.md` at each level | Familiar web-directory convention, but creates indistinguishable search results and eager bookkeeping. |
| **One global index plus Ward-named canonical pages (recommended)** | Keeps global discovery while giving every Ward a meaningful, linkable identity. |
| Search-only, no curated home | Minimal maintenance, but weak progressive disclosure for humans and cold-start agents. |

### Metadata strictness

| Option | Trade-off |
| --- | --- |
| Mandatory OKF profile | Strong mechanical uniformity, but ordinary useful Markdown can fail for metadata unrelated to its content. |
| **Plain Markdown with optional bounded properties (recommended)** | Supports tags, aliases, dates, and sources when useful while keeping prose valid. |
| Unconstrained Markdown with no recognized properties | Maximally flexible, but loses deterministic tag, alias, provenance, and lint behavior. |

### Operational layers and logging

This axis is exhaustive by where raw evidence, maintained synthesis, and
maintenance history live:

| Option | Trade-off |
| --- | --- |
| **Separate sources, maintained pages, doctrine, and log (recommended)** | Makes ownership and mutation rules inspectable and keeps operational history distinct from knowledge. |
| Mix sources, synthesis, and history in one Markdown namespace | Fewer paths, but provenance, immutability, and current synthesis become difficult to distinguish. |
| Keep synthesis/history in database-only Wiki records | Efficient for recall, but invisible to ordinary file inspection and independent from Ward edits. |
| Do nothing: strict OKF tree without an ingest log contract | Retains current conformance but does not define how knowledge compounds or how maintenance is audited. |

For transition, the exhaustive choices are automatic migration, compatibility
reading, manual per-Ward migration, or clean break. The clean break is
recommended because acceptance explicitly uses deleted Wards and a fresh
database. For delivery, the choices are template-only, staged vertical slices,
or a full-stack big bang. Staged slices are recommended because template-only
leaves dual authority intact and a big bang couples filesystem safety, LLM
mutation, persistence, recall, API, and UI risk.

## Risks & what would make this wrong

Pre-mortem:

- **The LLM fragments knowledge into many near-duplicate pages.** Mitigate with
  update-before-create doctrine, exact path identity, alias collision checks,
  duplicate suggestions, and bounded ingest review.
- **Derived search becomes stale and answers from old content.** Store source
  path/content digest and projection time; surface lag; rebuild idempotently;
  never let the projection overwrite files.
- **Optional metadata becomes an undocumented schema by accumulation.** Keep
  the recognized vocabulary deliberately small, preserve unknown properties,
  and require governance for hard validation additions.
- **Source immutability blocks legitimate user corrections.** The LLM may not
  mutate sources; users can replace or annotate them explicitly, after which a
  new ingest updates the wiki and log.
- **Wikilink ambiguity produces incorrect connections.** Prefer
  Ward-relative path identity, require qualified links on collisions, and fail
  lint rather than guessing.
- **A source or page injects instructions into the maintaining LLM.** Preserve
  content/instruction separation, provenance-label all excerpts, and reject
  content-originated scope or tool changes.
- **A link leaks content from another Ward.** Treat cross-Ward targets as
  external references and require request scope or user approval before
  traversal.
- **Concurrent human and agent edits overwrite one another.** Require
  preimage digests, commit epochs, staged publication, and fail closed on
  mismatch.
- **Semantic lint is mistaken for objective truth.** Keep contradiction,
  staleness, and missing-concept analysis advisory and evidence-linked.
- **The staged rollout temporarily exposes both old and new Wiki surfaces.**
  Label database-only articles legacy, prevent new competing writers, and make
  the canonical filesystem page the Ward UI summary as early as practical.

Falsifiable assumptions:

- A canonical Ward page plus search is enough navigation for hundreds of pages
  without recursive indexes.
- Existing Ward Layout confinement and snapshot behavior can remain unchanged
  while document format policy becomes less strict.
- A database projection can be rebuilt from Markdown with acceptable startup
  or background-index cost.
- Users prefer readable, editable Markdown over mandatory interoperability
  metadata.
- The fresh-database clean break is acceptable for the first release.

Drawbacks:

- Strict OKF interoperability and its required `type` metadata are abandoned.
- Link parsing, alias resolution, projection freshness, and source ownership
  become product responsibilities.
- `<ward-id>.md` requires a bounded dynamic starter destination that the
  current materializer does not support.
- Enforcing immutable sources across every model-visible mutation surface and
  making multi-file edits recoverable are cross-cutting runtime work, not
  doctrine-only changes.
- Four slices take longer than a template patch and require temporary
  compatibility boundaries.
- LLM-maintained wikis still require user review for consequential synthesis;
  lower bookkeeping cost does not make generated knowledge infallible.

## Evidence & prior art

The riskiest Slice 1 assumption was that a data-only template could already
materialize `<ward-id>.md`. A source trace and focused Ward construction test
showed otherwise: both platform materializers deliberately skip required
patterns containing `{name}`. The existing literal-node construction test
passed. Slice 1 therefore needs a small, confined canonical-starter rendering
feature rather than YAML-only edits. Plain Markdown is already a supported
`NodeFormat`, and Ward search already treats frontmatter as optional, so the
format relaxation does not require a replacement layout engine.

Repo precedent:

- RFC-0015 made the filesystem authoritative in principle but chose recursive
  OKF and planned to remove the separate Wiki. This RFC retains filesystem
  authority while changing its knowledge convention.
- ADR-0001 established versioned, declarative, snapshot-on-create
  `ward-conf.yaml`; that safety boundary remains.
- RFC-0017 and ADR-0002 established complete, closed archetype bundles and
  snapshot isolation; those decisions remain.
- `gateway/gateway-execution/src/ward_wiki.rs` already implements incremental
  LLM compilation, title reuse, similarity deduplication, tags, and an index,
  but persists them only through `WikiStore`.
- `runtime/agent-tools/src/tools/ward.rs` already searches Markdown and
  extracts optional titles and tags with filename fallback.
- `gateway/src/http/ward_content.rs` currently derives Ward summaries from the
  database-only `__index__`, identifying the API cutover point.

External prior art:

- Karpathy's [LLM Wiki pattern](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f)
  defines immutable raw sources, an LLM-maintained directory of Markdown
  pages, and an agent schema, with ingest/query/lint workflows and distinct
  content index and chronological log roles.
- Obsidian supports path-qualified `[[wikilinks]]` and aliases as a network of
  knowledge, while documenting interoperability limits for
  Obsidian-specific block references.
  [Obsidian internal links](https://obsidian.md/help/links)
- Obsidian derives linked mentions from outgoing links rather than requiring
  authors to maintain reverse links.
  [Obsidian backlinks](https://obsidian.md/help/plugins/backlinks)
- Obsidian stores optional tags and aliases as YAML properties and treats
  properties as small, atomic, human- and machine-readable metadata.
  [Obsidian properties](https://obsidian.md/help/properties)
- QMD indexes Markdown into a local SQLite FTS/vector store and retrieves
  documents by their source paths, demonstrating a filesystem-content plus
  derived-index model.
  [QMD README](https://github.com/tobi/qmd/blob/main/README.md)
- Git tracks recoverable file history and exposes who changed what and when,
  complementing an append-only operational log without making Git mandatory.
  [GitHub: About Git](https://docs.github.com/en/get-started/using-git/about-git)

## Open questions

None for RFC acceptance. Projection scheduling and UI graph rendering are
feature-design choices constrained by the decisions above and belong in their
respective specs.

## Follow-on artifacts

- A superseding ADR that retains versioned Ward Layout snapshots and replaces
  strict OKF with filesystem-authoritative LLM Wiki Markdown.
- Spec: `docs/specs/llm-wiki-ward-foundation/`
- Spec: `docs/specs/llm-wiki-operations/`
- Spec: `docs/specs/filesystem-wiki-projection/`
- Spec: `docs/specs/llm-wiki-ward-ui/`
- Mark RFC-0015 superseded and archive the Draft `okf-ward-foundation` and
  `okf-mini-obsidian-ui` specs after acceptance; update the active
  `bundled-ward-archetypes` spec in place.
