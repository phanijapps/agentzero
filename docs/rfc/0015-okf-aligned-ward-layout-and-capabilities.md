# RFC-0015: OKF-Aligned Ward Layout and Capabilities

- **Status:** Superseded by RFC-0018
- **Author:** zbot maintainers
- **Approver:** phanijapps
- **Date opened:** 2026-07-19
- **Date closed:** 2026-07-19
- **Related:** [RFC-0005](0005-builder-delegation-and-ward-context-hygiene.md); [RFC-0006](0006-vault-obsidian-style-ward-browser.md); [RFC-0014](0014-context-capability-registry-and-context-graph.md); `durable-ward-memory`; `vault-layout-standardization`; `gateway/templates/skills/spec-builder/`; `gateway/templates/skills/plan-composer/`; `gateway/templates/skills/ward-designer/`

## The ask

Approve a ward layout in which the configured wards root is one strict Open
Knowledge Format (OKF) v0.1 bundle, its `index.md` catalogs all wards, and each
ward colocates its durable concepts with their specifications, plans, tasks,
and child concepts.
Executable and generated non-Markdown assets may coexist in the tree; every
non-reserved Markdown file is an OKF concept. Approve the corresponding planner/spec skill cutover,
structured ward discovery capability, and retirement of the dedicated wiki
ward and its Obsidian templates.

Wards are already durable workspaces, but their knowledge is divided between
`memory-bank/`, `specs/`, domain folders, reports, and generated output. The
complication is that those locations lack a common metadata and navigation
contract, while current planning prompts hard-code the old paths. The question
is how to gain OKF interoperability across the entire wards tree while
preserving the distinct operational roles expressed through concept types.

Decisions requested:

1. Make the configured wards directory a strict OKF v0.1 bundle and generate
   `<wards-root>/index.md` as the catalog of all wards. Recommended: accept.
   Resolve the root through `VaultPaths`; never hard-code
   `~/Documents/zbot/wards`. Owner: phanijapps.
   Decide by 2026-07-26.
2. Keep “subdomain” as the product/filesystem term while modeling each
   subdomain as a durable OKF concept represented by `<ward>/<subdomain>.md`.
   Its companion `<ward>/<subdomain>/` holds the index, refinement spec,
   current plan, tasks, history, and child concepts. Recommended: accept. The
   subdomain concept persists while repeated runs refine or reprocess it.
   Owner: phanijapps. Decide by 2026-07-26.
3. Update ward-local planning/spec skills and runtime prompts to use only the new
   paths and carry an explicit active-plan path. Recommended: accept. A path
   cutover without orchestration changes would strand live plans. Decide
   by 2026-07-26. Owner: phanijapps.
4. Add OKF-aware read-only discovery over type, tags, text, and path while
   keeping the filesystem authoritative. Recommended: accept as a Context
   Capability resource rather than a model-visible search tool. Owner:
   phanijapps. Decide by 2026-08-02.
5. Make the new layout a clean break: existing wards are unsupported and must
   be deleted or rebuilt; no reader, writer, migration, or persisted-state
   compatibility is provided. Recommended: accept. Owner: phanijapps. Decide
   by 2026-07-26.
6. Retire the dedicated wiki ward, wiki promotion skill, Obsidian templates,
   wiki settings/startup seeding, and ward-wiki recall source, but preserve and
   evolve the UI into a Phase 2 OKF mini-Obsidian browser/editor. Recommended:
   accept. Owner: phanijapps. Decide by 2026-07-26.

## Problem & goals

The current ward doctrine is structurally useful but not interoperable:

- `memory-bank/ward.md`, `structure.md`, and `core_docs.md` describe durable
  knowledge using z-Bot-only conventions.
- Ward-local plans live under `specs/<subdomain>/`, separate from the durable
  concepts they refine.
- Planning, builder, writer, and continuation prompts assume those exact paths.
- Filesystem search matches names and paths but does not understand OKF
  frontmatter such as `type` or `tags`.
- Reports, datasets, source code, and generated artifacts have different
  lifecycles but are not separated by a stable top-level contract.

The clean-break decision makes recursive conformance practical: existing wards
are not converted, and rebuilt wards author every non-reserved Markdown file as
an OKF concept from the start. This includes `AGENTS.md`, architecture,
specifications, plans, tasks, Markdown reports, and Markdown outputs.

Goals:

- Give every durable concept one navigable home for its metadata, child
  concepts, refinement spec, current plan, tasks, and update history.
- Make the complete configured wards tree genuinely OKF-conformant and portable.
- Preserve `AGENTS.md` as the ward-agent instruction contract.
- Keep non-Markdown code, datasets, references, and generated outputs usable
  alongside the bundle; require Markdown reports/outputs to be OKF concepts.
- Let planners and executors find the correct active plan without relying on a
  single global `specs/plan.md` assumption.
- Support structured discovery by concept type, tags, text, ward, and parent
  concept.
- Fail clearly on legacy wards and direct the user to delete or rebuild them.
- Replace the separate wiki product surface with ward-local OKF concepts and
  indexes.

Non-goals:

- Changing repository-governance artifacts under `docs/specs/`; those specs
  describe AgentZero features and remain governed by `docs/CONVENTIONS.md`.
- Requiring a database, SDK, proprietary service, or Engram storage change for
  OKF conformance.
- Turning non-Markdown assets or ephemeral runtime logs into concepts.
- Defining a closed taxonomy of allowed OKF `type` or `tags` values.
- Preserving, converting, or importing legacy wards or wiki content.
- Supporting mixed old/new ward layouts in one runtime release.

## Proposal

### Canonical ward structure

```text
<configured-wards-root>/              # strict OKF v0.1 bundle
├── index.md                          # all wards; may declare okf_version
├── log.md                            # optional cross-ward update history
└── <ward-name>/
    ├── index.md                      # ward progressive-disclosure index
    ├── architecture.md               # type: Architecture
    ├── AGENTS.md                     # type: Agent Instructions
    ├── config.yaml                   # ward configuration, non-Markdown
    ├── <subdomain>.md                # canonical OKF concept record
    ├── <subdomain>/
    │   ├── index.md                  # links ../<subdomain>.md and contents
    │   ├── spec.md                   # type: Concept Refinement Specification
    │   ├── plan.md                   # type: Implementation Plan
    │   ├── log.md                    # reserved chronological history
    │   ├── tasks/
    │   │   ├── index.md
    │   │   ├── task-001.md           # type: Implementation Task
    │   │   └── task-002.md
    │   ├── history/
    │   │   └── runs/<run-id>/        # immutable spec/plan/task snapshot
    │   ├── <child-concept>.md        # chapter, company, metric, etc.
    │   └── <child-group>/            # optional deeper hierarchy
    ├── src/<subdomain>/              # code; Markdown files need frontmatter
    ├── data/<subdomain>/             # datasets and inputs
    ├── reports/<subdomain>/          # Markdown reports are concepts
    ├── output/<subdomain>/           # Markdown outputs are concepts
    └── references/                   # supporting source material
```

The wards-root `index.md` is the catalog entry point. It lists every validated
ward with title, description, relative link, tags, status, and last meaningful
update derived from that ward's index/concepts. It is regenerated atomically
when wards are created, deleted, or change catalog metadata and can be
synthesized on read if stale.

`architecture.md`, `AGENTS.md`, and `config.yaml` are z-Bot conventions, not
OKF reserved filenames. `architecture.md` and `AGENTS.md` are ordinary OKF
concepts and therefore require frontmatter; `config.yaml` is non-Markdown and
is outside the concept-document rules.

Operational directories are partitioned by the same subdomain slug. Links
between Markdown concepts use wards-root bundle-relative paths. Non-Markdown
assets are referenced through stable `resource` URIs using a z-Bot
ward-resource scheme. Optional directories remain lazy rather than appearing
empty in every ward.

Dependency trees, package caches, virtual environments, temporary workspaces,
and other third-party material that may contain uncontrolled Markdown must live
outside the configured wards root in z-Bot-managed runtime/cache locations.
Examples include `node_modules/`, `.venv/`, downloaded package sources, and
tool caches. A ward may contain lockfiles and source manifests, but cannot
vendor non-conformant Markdown beneath the OKF bundle.
Imported or generated Markdown anywhere under `src/`, `data/`, `references/`,
`reports/`, or `output/` must be normalized into an appropriate OKF concept
before its atomic write or rejected. The same rule applies to the reserved
filenames `index.md` and `log.md`; generators cannot use those names for
unrelated output.

Ward-root names `src`, `data`, `reports`, `output`, `references`, `index.md`,
`log.md`, `AGENTS.md`, `architecture.md`, and `config.yaml` are reserved by the
layout. Subdomain slug validation rejects every reserved name, hidden name,
case-folded collision, and unsafe path before planning or scaffolding. The one
intentional same-stem pair is `<subdomain>.md` plus `<subdomain>/`; validators
treat it as the concept record and its companion detail directory.

Inside every companion directory, `index.md`, `log.md`, `spec.md`, `plan.md`,
`tasks/`, and `history/` are reserved. Child concepts recursively use the same
`<child>.md` plus optional `<child>/` companion convention; a directory without
a same-stem concept file is only an organizational group and must be identified
as such in its parent `index.md`. The validator applies collision and reserved-
name rules at every nesting level.

### Subdomain concept and refinement contract

Every non-reserved Markdown file beneath the configured wards root starts with
parseable YAML frontmatter and a non-empty `type`. Recommended shared fields are `title`,
`description`, `resource`, `tags`, and `timestamp`; z-Bot adds optional fields
such as `status`, `owner`, and `supersedes`, which OKF permits consumers to
preserve as producer extensions.

`<subdomain>.md` is the durable concept identity and current knowledge record.
It can exist without a spec or plan. `spec.md` is created lazily when a future
run needs to refine, refresh, reindex, or otherwise reprocess that same
subdomain concept; it defines that run's intended improvement rather than the
concept's identity. `plan.md` is the current delivery strategy for the active
refinement. `tasks/` contains independently executable work items. Tasks remain
individually addressable after completion and carry a
finite status such as `pending`, `in-progress`, `blocked`, or `completed`.
`tasks/index.md` groups current work by status. `log.md` records meaningful
subdomain changes under ISO `YYYY-MM-DD` headings, newest-first. Generators
prepend entries to the applicable date group; they do not append entries to the
bottom. Non-root `index.md` files contain no frontmatter. The bundle-root
`index.md` may use the OKF v0.1 version-declaration exception. Every generator
uses shared reserved-file templates and validates them after writing.
The companion directory and its `spec.md`, `plan.md`, `tasks/`, `history/`, and
`log.md` are lazy; a subdomain needs only its concept file and a ward-index
entry until refinement work or child concepts require more structure.

For example, `aapl-valuation.md` can carry Apple identity, ticker, valuation
method, last valuation date, and the latest report resource. Its companion
`aapl-valuation/` contains a future refresh spec, plan, tasks, report concepts
or links, and supporting concepts; report assets remain under
`reports/aapl-valuation/`. In a fictional-books ward, `great-expectations.md` is the
book concept and `great-expectations/` contains chapter concepts and optional
future reindexing or enrichment work. The user-facing term remains subdomain;
the OKF semantic model treats it as a concept.

The current refinement documents are updated in place during one run. Before a
later run replaces them, the terminal set—`spec.md`, `plan.md`, `tasks/index.md`,
and every task file—is atomically snapshotted under
`history/runs/<run-id>/`. `run-id` is the immutable identifier assigned when
planning begins; its filesystem-safe form is the archive directory name, and
an existing different directory is a no-clobber error. Unrelated runs receive
new task IDs rather than reusing prior identities. Links among current specs,
plans, tasks, and concepts use bundle-relative paths; archival rewrites links
to the immutable run snapshot where appropriate, preserves operational assets
as `resource` URIs, and validates the complete snapshot before replacing any
current file. This ward-local archive is required
even when Git is available; version control adds detailed diffs and attribution
but is not a durability precondition. If simultaneous independent plans per
subdomain become necessary, that is a follow-up versioned convention rather
than an unplanned filename fork.

### Planning and spec skill cutover

Ward-local planning changes as one coordinated contract:

1. `planner-agent` resolves or creates a subdomain slug and writes
   or reads `<subdomain>.md`, then writes `<subdomain>/spec.md`, `plan.md`, and
   `tasks/task-NNN.md` inside the active ward for the requested refinement run.
2. `spec-builder` emits `type: Concept Refinement Specification`, links the
   durable `<subdomain>.md` concept, and stops treating `memory-bank/*` as
   required ward doctrine.
3. `plan-composer` emits `task-NNN.md` briefings rather than `step_N.md`, adds
   `type: Implementation Task`, `status`, dependency metadata, and links the
   tasks from `plan.md` and `tasks/index.md`.
4. `ward-designer` creates or validates the ward `index.md`, `AGENTS.md`, the
   wards-root catalog entry, `architecture.md`, and the selected subdomain index.
   It assigns exact paths under `src/`, `data/`, `reports/`, and `output/` to
   later tasks and ensures any generated Markdown carries OKF frontmatter.
5. Builder, writer, solution, and specialist prompts read the assigned task,
   subdomain `spec.md` and `plan.md`, root `AGENTS.md`, and linked concepts.
   Reusable assets are described by OKF concepts or architecture links rather
   than registered in `memory-bank/core_docs.md`.
6. First-turn and continuation protocols stop hard-coding `specs/plan.md`.
   Planner completion returns `active_plan_path`; execution state stores that
   ward-relative path and each delegated task carries its own exact
   `task_path`. Continuations validate both paths remain inside the active ward.
7. Delta planning within the same run edits only changed concepts/tasks and
   preserves task IDs. A new refinement run archives the complete prior set and
   allocates new task IDs. Both update timestamps/status and prepend a concise
   entry to the applicable date group in `log.md`.

The implementation spec must carry an old-path producer/consumer removal matrix, not
only update the three authoring skills. At minimum it covers:

| Surface | Required cutover |
| --- | --- |
| `runtime/agent-tools/src/tools/ward.rs` and execution helpers | New scaffold, plan/task lookup, update-plan behavior, and explicit legacy-layout rejection. |
| `gateway/gateway-execution/src/{delegation,invoke,middleware,runner,session_ctx}/` | Context injection, ward-agent setup, plan discovery, snapshots, resume/restart, and delegated task binding. |
| `gateway/gateway-execution/src/{archiver,distillation}.rs` | Preserve and emit canonical knowledge/task pointers. |
| `gateway/gateway-services/src/ward_curator.rs` and ward scaffolding | Canonical-layout handling plus deterministic rejection of legacy wards. |
| Bundled planner, builder, writer, solution agents and planning shards | Replace `specs/` and `memory-bank/` doctrine with exact active plan/task paths and OKF concepts. |
| `spec-builder`, `plan-composer`, `ward-designer`, examples, and `.prev` material | Emit and teach only the canonical format; delete obsolete examples and compatibility copies. |
| Gateway execution and ward pipeline tests | Cover canonical wards, deterministic rejection of legacy wards, and absence of parallel truth trees. |

The mechanical gate is an `rg` inventory of every old-path and wiki reference,
plus every wards-root construction and default path literal.
Every runtime, prompt, template, fixture, and test hit must be removed unless it
is historical documentation or the one-way destructive schema-removal
migration and its verification tests. Those narrow migration references may
name obsolete tables but cannot read content for product use, import it, or
expose compatibility behavior. There are no compatibility readers. A ward is
canonical only when `config.yaml` declares `layout_version: okf-ward-v1` and
the required roots validate. Any other ward fails closed with a concise
`legacy ward unsupported; delete or rebuild` diagnostic. Persisted sessions or
task pointers into a deleted or legacy ward do not resume; they surface the
same terminal rebuild requirement rather than falling back or replanning.
Catalog, scaffold, validation, discovery, deletion, and containment call sites
must all receive the root from `VaultPaths`; a repository search must find no
hard-coded `~/Documents/zbot/wards`, equivalent home expansion, or manually
joined default wards root outside `VaultPaths` tests.

The separate Codex/repository `new-spec` workflow continues to create
`docs/specs/<feature>/spec.md` and `plan.md`. That is project governance, not a
ward-local execution corpus. Documentation and skill names must say which
contract they operate on to prevent accidental cross-use.

### Wiki backend retirement and Phase 2 OKF UI

The wards-root OKF bundle replaces the dedicated Obsidian-style wiki rather than
coexisting with it. The cutover removes the whole product path:

- `gateway/templates/skills/wiki/`, shared Obsidian conventions, and wiki
  routing/promotion instructions;
- startup creation and seeding of the `wiki` ward, its numbered folders,
  `_zztemplates/`, assets, `AGENTS.md`, and memory-bank doctrine;
- `WikiConfig` and dedicated-wiki-ward selection, while retaining and
  retargeting the reusable browsing components;
- ward-wiki indexing, promotion, recall adapters/source-summary entries, and
  old wiki DTO semantics; the existing UI affordances are retargeted rather
  than deleted;
- wiki-specific repositories, traits, Engram adapters, migrations, and tests
  once no non-wiki knowledge contract depends on them.

Book-reader and research skills write durable results directly as OKF concepts
under the active ward and subdomain. They no longer stage content for later
promotion into a global wiki. Cross-ward discovery is supplied by the Context
Capability registry over the wards-root OKF bundle, while knowledge graph or general
memory features remain separate capabilities rather than inheriting the wiki
name or schema.

No wiki data import or compatibility endpoint is provided. Users who need old
wiki content must extract it before upgrading and deliberately author it into a
rebuilt ward; the product does not automate that conversion.

Phase 1 supplies the OKF-aware tree, catalog, metadata search, validation, and
read-only concept preview needed to keep the UI useful after the backend
cutover. The old wiki-named route is renamed to an OKF/knowledge surface in the
clean break; preserving components and user experience does not preserve wiki
route or DTO compatibility. Phase 2 evolves the retained UI into a mini-Obsidian experience over
the wards-root OKF bundle: ward/subdomain navigation, backlinks and graph
relationships, tags/types/status filters, concept editing with conformance
validation, task/plan views, and links from concept metadata to reports and
non-Markdown resources. Phase 2 is a UI/data-contract evolution, not a revival
of the dedicated wiki ward or its templates.

### Discovery and ward capability

Introduce an OKF consumer that scans the configured wards-root bundle and can
scope results to one ward or subdomain. It returns bounded metadata records:

```text
ward_id, concept_id, path, type, title, description, tags, timestamp, status
```

Queries support `query`, `tags`, `types`, `path_prefix`, `status`, and `limit`.
Start with bounded scan-on-demand plus a cache keyed by relative path,
modification time, and content hash. Bounds apply independently to returned
records, files visited, total bytes read, frontmatter bytes and YAML nesting,
directory depth, elapsed time, and cache entries/bytes; exact conservative
values belong in the implementation spec and appear in capability metadata.
The consumer rejects symlinks, canonicalizes the configured wards root and selected ward,
validates `path_prefix` as a clean relative path, and requires every visited
path to remain beneath the canonical bundle root. Truncation and parse failures
are explicit in results. Tolerant discovery may return partial repair/admin
results, but validation is strict: any nested Markdown parse/frontmatter error,
reserved-file violation, invalid link form, or legacy ward makes the entire
wards-root bundle nonconformant and prevents the product from labeling it
conformant. The cache is derived, quota-bound, and rebuildable; the files remain
authoritative. Broken links and unknown types remain tolerable only where OKF
v0.1 defines them as permissive.

Following RFC-0014, the preferred model-facing shape is a read-only Context
Capability resource or preassembled context packet. The existing local Vault
HTTP search remains available for UI filename/path discovery and can add OKF
filters. The model receives this read-only context through the Context
Capability resource; no `ward(action="search")` compatibility facade ships.

Add explicit `validate` and `reindex` operations to the service/UI/CLI
capability surface. Validation checks OKF conformance and path/link hygiene.
Reindexing refreshes only derived metadata and never rewrites concepts.

### Clean-break cutover

There is no ward or wiki migration. On upgrade, users must delete existing ward
directories, move them outside the configured wards root, or rebuild them
manually as new `okf-ward-v1` wards. Startup validates the entire root before
any ward is represented as conformant. While any legacy ward remains beneath
that root, the complete bundle is nonconformant and ward execution, planning,
search, resume, and delegation remain disabled; repair/admin discovery may
still identify the offending paths.

The release removes legacy creation, reading, writing, fallback, import, and
conversion code in the same cutover. It does not seed a `wiki` ward. Existing
wiki database rows and filesystem content are not imported into OKF; removal of
obsolete wiki persistence schemas follows the normal application database
migration discipline, but provides no product-level data compatibility.

Release notes must state the destructive prerequisite prominently: export any
files the user wants to keep, then delete or rebuild wards before using ward
features. The UI and CLI may provide `open folder` and `delete ward` actions,
but no converter or mixed-layout mode.

## Options considered

### Conformance boundary

Axis: number and placement of strict conformance roots. These options partition
the space into no strict root, one root per ward, a nested knowledge root per
ward, or one catalog root containing every ward.

| Option | Trade-off |
| --- | --- |
| Do nothing | No migration cost, but ward knowledge stays bespoke and tag/type interoperability remains unavailable. |
| One strict bundle per ward | Gives each ward portability but requires a separate cross-ward catalog contract. |
| `knowledge/` bundle inside each ward | Isolates concept Markdown but creates the extra layer this proposal now removes. |
| Configured wards root is one bundle | Makes `index.md` the portable catalog of all wards and gives every nested Markdown file one conformance rule. Recommended. |

### Specification placement

Axis: where a refinement contract belongs relative to its durable subdomain
concept. The contract can stay in the legacy global taxonomy, move into another
global spec taxonomy, live in the concept's companion directory, or leave the
ward.

| Option | Trade-off |
| --- | --- |
| Keep `specs/<subdomain>/` | Avoids prompt changes but preserves the split knowledge model. |
| Use `<ward>/specs/<subdomain>/` | Conformant with frontmatter, but duplicates the subdomain hierarchy and makes navigation less direct. |
| Use `<ward>/<subdomain>.md` plus `<ward>/<subdomain>/{spec,plan,tasks}` | Keeps the durable concept identity separate from, but adjacent to, repeatable refinement work. Recommended. |
| Keep specs only in repository `docs/specs/` | Appropriate for AgentZero governance, but not for user ward execution and portability. |

### Read-only discovery surface

Axis: how consumers obtain searchable concept metadata. Discovery can remain
path-only, become a model tool, become a separate model tool, or enter through
the context/resource boundary.

| Option | Trade-off |
| --- | --- |
| Do nothing/path-only search | Retains shipped Vault behavior but cannot filter OKF types, tags, or status. |
| Extend model-visible `ward` permanently | Convenient syntax but expands a broad tool with read-only discovery contrary to RFC-0014's capability split. |
| Add `ward_search` model tool | Clearer than overloading `ward`, but still adds prompt-visible discovery and risks duplicated indexing. |
| One OKF consumer exposed as resource/context plus HTTP/UI adapters | Keeps one implementation and supports proactive context assembly without another model-visible tool. Recommended. |

### Existing-ward transition

Axis: supported treatment of old wards after the format cutover. The product
can continue supporting them, transform them, or reject/discard them.

| Option | Trade-off |
| --- | --- |
| Preserve and support | Avoids disruption but requires permanent dual-layout behavior. |
| Transform/import | Retains content but creates classification, conflict, testing, and rollback machinery that the clean break does not need. |
| Reject old wards; user deletes or rebuilds | Produces one format and removes compatibility complexity, at the explicit cost of user-managed export/rebuild. Recommended. |

## Risks & what would make this wrong

- **Planner path drift:** one stale prompt can continue writing `specs/` and
  create parallel sources of truth. Mitigation: centralize a ward-layout path
  contract, update all bundled prompts together, and add repository searches
  plus end-to-end planning tests.
- **Active-plan ambiguity:** multiple subdomains can each have `plan.md`.
  Mitigation: store and return `active_plan_path`; pass exact `task_path` into
  delegation rather than searching heuristically.
- **Knowledge pollution:** agents may put every report or log inside
  domain indexes merely because it is Markdown. Mitigation: use `type` and
  placement to distinguish durable concepts, reports, generated artifacts, and
  agent instructions while applying the same frontmatter rule to all of them.
- **Catalog drift:** `<wards-root>/index.md` can lag ward creation, deletion, or
  metadata edits. Mitigation: update it atomically on lifecycle actions,
  validate links, and synthesize the current catalog on read when stale.
- **Third-party Markdown breaks conformance:** dependency managers commonly
  install README or license Markdown beneath project directories. Mitigation:
  force dependency environments and caches outside the wards root and make the
  conformance validator reject uncontrolled Markdown before a ward is usable.
- **Mutable-current-doc history loss:** updating `plan.md` and task status could
  erase why decisions changed outside Git. Mitigation: archive the complete
  terminal refinement set under `history/runs/<run-id>/`, prepend meaningful
  changes to subdomain `log.md`, and use version control as additional history.
- **Upgrade data loss:** users may delete wards or wiki content without first
  exporting files they value. Mitigation: prominent release/startup warnings,
  explicit confirmation for product deletion actions, and an `open folder`
  affordance; the product still provides no importer.
- **Wiki backend cutover regression:** a recall, UI, or research journey may
  still depend on wiki-specific DTOs or source labels. Mitigation: retarget the
  retained UI to Phase 1 OKF read models, remove the old producer/consumer
  matrix as one gated slice, and prove equivalent OKF discovery before release.
- **Draft-standard movement:** OKF v0.1 is explicitly Draft and may evolve.
  Mitigation: declare `okf_version: "0.1"`, isolate parsing behind one consumer,
  tolerate extensions, and keep files readable without the consumer.
- **Search cache staleness:** derived metadata can lag filesystem edits.
  Mitigation: path/mtime/hash invalidation, explicit reindex, and rebuildable
  cache with files as truth.

Falsifiable assumptions:

- Subdomain is the product term and stable organizing unit; semantically each
  subdomain is a durable OKF concept refined by later specs and tasks.
- One current `plan.md` per subdomain is sufficient for the first release.
- Every non-reserved Markdown file beneath the configured wards root can carry
  valid OKF frontmatter without breaking its operational consumer.
- New planner/executor state can carry a ward-relative active-plan path; old
  persisted ward executions are intentionally non-resumable.
- A bounded metadata scan is adequate before a durable derived index is needed.

Drawbacks:

- Existing agents, skills, prompts, fixtures, tests, and user habits must learn
  new paths and terminology.
- Strict frontmatter adds authoring overhead to every durable concept and task.
- Agent instructions, reports, and Markdown outputs acquire metadata even when
  their primary role is operational rather than knowledge authoring.
- Existing wards and wiki content must be exported manually and rebuilt or
  discarded; this is intentionally disruptive.

## Evidence & prior art

The [OKF v0.1 draft specification](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
defines a bundle as a hierarchical directory of Markdown concepts with YAML
frontmatter, permits distribution as a subdirectory of a larger repository,
reserves `index.md` and `log.md`, permits producer-defined types and extension
fields, and specifies permissive handling of missing indexes, unknown types,
and broken links. It also says tag views can be synthesized by scanning
frontmatter rather than requiring a tag aggregation format.

Google's [OKF introduction](https://cloud.google.com/blog/products/data-analytics/how-the-open-knowledge-format-can-improve-data-sharing)
describes the format as files plus Markdown and YAML frontmatter rather than a
new runtime or SDK, and presents producer/consumer independence as a core
principle. Those properties support a z-Bot producer and consumer sharing one
filesystem contract without making the search cache authoritative.

Repository precedent:

- `runtime/agent-tools/src/tools/ward.rs` creates `AGENTS.md`, `memory-bank/`,
  and `specs/`, and exposes only `use`, `create`, `list`, and `info`.
- `gateway/templates/skills/spec-builder/SKILL.md`,
  `plan-composer/SKILL.md`, and `ward-designer/SKILL.md` encode the legacy
  doctrine, step naming, and placement rules.
- `gateway/templates/agents/planner-agent.md`, the writer prompt,
  `first_turn_protocol.md`, `planning_autonomy.md`, and
  `instructions_starter.md` hard-code `specs/plan.md` or
  `specs/<domain>/...` continuation behavior; builder and solution prompts
  separately hard-code `memory-bank/*` doctrine.
- `gateway/gateway-services/src/settings.rs` defines `WikiConfig`, while
  `gateway/src/state/mod.rs` automatically creates the wiki ward and seeds its
  numbered Obsidian folders and `_zztemplates/` directory.
- `gateway/templates/skills/wiki/` and
  `gateway/templates/skills/_shared/obsidian_conventions.md` implement the wiki
  promotion and authoring contract; book/research archetypes reference it.
- `gateway/gateway-execution/src/ward_wiki.rs`, unified recall adapters,
  wiki store traits/repositories/adapters, and the Memory command-deck UI expose
  wiki as a separate indexed source and user-facing content type.
- RFC-0006 and `gateway/src/http/vault.rs` provide bounded local-only ward file
  search by filename/path, but not OKF frontmatter search.
- RFC-0014 recommends routing read-only discovery through resources/context
  capabilities and reserving model-visible tools for actions.
- `docs/specs/vault-layout-standardization/` records the earlier compatibility
  posture that this RFC deliberately supersedes for wards and wiki content.
- `docs/specs/durable-ward-memory/` treats ward files as durable sources and
  requires approval before automatic restructuring.

De-risk spike: a read-only scan of 11 live wards found 227 Markdown files
outside dependency environments; only 33 began with YAML frontmatter. This
falsified the assumption that adding `index.md` alone would make old wards
conformant. The scan excluded dependency environments, so it does not establish
whole-root feasibility by itself. The clean break plus external dependency
locations, normalized generated Markdown, and strict validation make recursive
wards-root conformance testable for rebuilt wards.

## Open questions

1. Should multiple simultaneous plans within one subdomain be supported in the
   first release? Recommended default: no; keep one current `plan.md`, preserve
   terminal refinement sets through `history/runs/<run-id>/`, Git, and
   `log.md`, and add
   concurrent plan sets only with demonstrated demand. Owner: phanijapps.
   Decide by 2026-08-02.

## Follow-on artifacts

- ADR (owner: phanijapps): configured wards root as the strict OKF bundle and
  cross-ward catalog boundary.
- Spec (owner: phanijapps): clean-break ward scaffolding, validation, legacy
  rejection, and rebuild/delete UX.
- Spec (owner: phanijapps): ward-local planner, spec-builder, plan-composer, ward-designer, and
  execution-state path cutover.
- Spec (owner: phanijapps): OKF validation, metadata discovery, caching, and Vault UI filters.
- Spec (owner: phanijapps): dedicated wiki backend, templates, settings,
  recall, and persistence retirement with Phase 1 UI retargeting.
- Spec (owner: phanijapps): Phase 2 OKF mini-Obsidian UI for browsing, graph
  relationships, editing, validation, and plan/task views.
- Convention change (owner: phanijapps): document the distinction between repository
  `docs/specs/` governance and ward-local `<subdomain>/` execution
  contracts.
