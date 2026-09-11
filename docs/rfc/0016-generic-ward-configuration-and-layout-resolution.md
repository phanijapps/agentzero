# RFC-0016: Generic Ward Configuration and Layout Resolution

- **Status:** Accepted
- **Author:** zbot maintainers
- **Approver:** phanijapps
- **Date opened:** 2026-07-19
- **Date closed:** 2026-07-19
- **Related:** [RFC-0005](0005-builder-delegation-and-ward-context-hygiene.md); [RFC-0015](0015-okf-aligned-ward-layout-and-capabilities.md); `okf-ward-foundation`; `okf-mini-obsidian-ui`

## Accepted implementation delta (2026-07-19)

The first implementation slice is intentionally smaller than the full proposal
below. It delivers only the user-editable generic template, snapshot-on-create,
generic structure/OKF lint, model nudges, and bounded context injection. Artifact
names in this RFC are default-template examples, not mandatory schema fields;
specs, plans, tasks, and operational areas may be removed or renamed in YAML.
Repair, approval grants, journaled mutation, broad OKF mutation APIs, and
cross-surface write sandboxing are deferred and are not Phase 1 requirements.
The strict generic rule primitives are owned by the fluid-template feature spec.
This delta supersedes conflicting "must implement in Phase 1" language below
without weakening the rule that no compiled fallback ward shape may exist.

## The ask

Approve a user-editable, versioned `ward-conf.yaml` as the sole activation
authority for ward layout and conformance. Seed its template at
`<vault>/config/templates/ward-conf.yaml`, copy it into each new ward, resolve
its logical artifact roles before planning, and inject the validated contract
into every spec, plan, delegation, and write context. Replace prompt-owned
layout decisions and `ward-designer` with deterministic middleware, hooks, and
`ward` lint/repair APIs. Every conformance failure must become a structured,
actionable nudge to the model; no hidden built-in layout or undeclared
conformance profile may be substituted.

Wards already use an OKF-aligned structure, but RFC-0015 fixed that structure
in prose, prompts, skills, and Rust. Those copies disagree about the placement
of a concept document and allow a model to invent paths. Users also cannot
change their ward convention without changing z-Bot itself. The question is
how to make ward structure fluid and user-owned while retaining deterministic
OKF validation and safe agent writes.

Decisions requested:

1. Make `<vault>/config/templates/ward-conf.yaml` the editable template and
   copy it to `<ward>/ward-conf.yaml` at ward creation. Existing wards use
   their own snapshot. Recommended: accept; it makes customization explicit
   without making later template edits silently rewrite live wards. Owner:
   phanijapps. Decide by 2026-07-26.
2. Treat the active YAML, including only the versioned profiles it explicitly
   references, as the complete layout and conformance authority.
   Recommended: accept; the implementation may validate the configuration
   language, path safety, and declared OKF rules, but must not contain a second
   ward shape or silently fall back to one. Owner: phanijapps. Decide by
   2026-07-26.
3. Make the default template place the canonical concept document inside its
   concept folder and require recursive OKF conformance there. Recommended:
   accept; `<concept>/<concept>.md`, `index.md`, `spec.md`, `plan.md`, tasks,
   and run history become one navigable knowledge subtree. Owner: phanijapps.
   Decide by 2026-07-26.
4. Inject the validated YAML contract and its resolved logical paths into
   intent analysis, spec creation, planning, delegation, continuation, and
   writes. Recommended: accept; spec and plan skills can then remain generic
   and never construct a filesystem path. Owner: phanijapps. Decide by
   2026-07-26.
5. Add ward lifecycle middleware, pre-write validation, a post-write hook, and
   `ward(action="lint"|"repair")`. Recommended: accept; every failure returns
   a model nudge identifying the YAML rule, affected path, and correction.
   Ambiguous or destructive cleanup remains preview-first. Owner: phanijapps.
   Decide by 2026-07-26.
6. Remove `ward-designer` and `ward_hygiene`, and make the spec and planning
   skills layout-neutral consumers of the injected contract. Recommended:
   accept; deterministic services should own mechanics while skills own
   authoring behavior. Owner: phanijapps. Decide by 2026-07-26.
7. Keep the clean-break policy: old wards are deleted or rebuilt, with no
   compatibility reader or migration. Recommended: accept; RFC-0016 explicitly
   supersedes the conflicting layout and orchestration portions of RFC-0015,
   its foundation specs, and RFC-0005. Owner: phanijapps. Decide by 2026-07-26.

## Problem & goals

RFC-0015 established the wards root as an OKF bundle, but it also embedded one
layout in several independently changing places:

- The RFC and foundation spec use a sibling `<concept>.md` plus `<concept>/`
  arrangement even though a concept and its refinement material are intended
  to be self-contained.
- `spec-builder`, `plan-composer`, `ward-designer`, agent prompts, and planning
  shards teach concrete paths instead of consuming resolved roles.
- Intent analysis retains a model-produced `structure` map, so path selection
  can compete with the declared convention.
- The `ward` tool creates a fixed root structure, while ordinary file tools
  can write paths without consulting a ward layout contract.
- The existing ward `config.yaml` controls provider/model inheritance and
  silently falls back when malformed. It is not a safe home for structural
  authority.

This duplication explains why a newly rebuilt `financial-analysis` ward could
still violate the accepted design: the live prompt and tool path won over the
documentation. Improving the prompt alone would reproduce the same failure.

Goals:

- Let a user change the generic ward convention by editing one vault template.
- Make a ward's checked-in snapshot the complete and inspectable authority for
  that ward after creation.
- Give every layout-sensitive participant exact logical roles and resolved
  paths rather than prose from which it guesses paths.
- Enforce the default self-contained concept shape recursively under every
  concept folder.
- Keep `src/`, `data/`, `reports/`, and `output/` as declared operational
  resource areas that may contain non-Markdown files and an `index.md`.
- Preserve strict OKF treatment of Markdown: declared reserved documents use
  their reserved behavior, and every other Markdown document must satisfy the
  template's OKF frontmatter rules.
- Convert all detectable conformance failures into model-readable correction
  nudges and prevent an invalid z-Bot-authorized mutation from being committed.
- Support explicit, previewable cleanup without silently deleting, merging, or
  overwriting user content.
- Keep the future OKF mini-Obsidian UI in scope as a consumer of the same
  resolved layout and lint results.

Non-goals:

- Designing the Phase 2 mini-Obsidian UI.
- Supporting domain-specific ward profiles such as finance or books.
- Migrating, reading, or preserving wards created under older layouts.
- Automatically propagating vault-template edits into existing ward snapshots.
- Providing an executable template language, arbitrary scripts, or hooks
  authored inside `ward-conf.yaml`.
- Making a skill the source of conformance rules.

## Proposal

### Configuration authority and lifecycle

The vault owns a canonical, user-hackable template with two explicit scopes:
the shared wards catalog and an individual ward:

```text
<vault>/
├── config/
│   └── templates/
│       └── ward-conf.yaml
└── wards/
    ├── index.md
    └── <ward>/
        ├── ward-conf.yaml
        └── ...
```

On ward creation, z-Bot validates the vault template and copies it atomically
to the ward before scaffolding anything else. On reuse, the ward snapshot is
the authority for paths below that ward; the current vault template is not
overlaid or merged. The vault template remains the authority for the shared
`wards/index.md` catalog because no individual ward may govern a sibling or
parent path. A missing or unparsable snapshot produces a conformance nudge. It
does not cause a compiled default or the current vault template to be
substituted.

`config.yaml` remains the optional provider/model override. `ward-conf.yaml`
is exclusively the structural contract. YAML is the one canonical phase-one
serialization; a JSON Schema validates its data model without creating a
second JSON configuration format.

The configuration is self-describing with `apiVersion` and `kind`. Unknown
major versions and unsupported versions fail closed with a terminal
configuration nudge. Unknown fields are accepted only where the selected
schema version explicitly permits them. Schema lookup is local and pinned by
the release; it never downloads executable or mutable remote definitions.
The data-only YAML loader rejects custom tags, merge keys, anchors/aliases,
duplicate or non-string keys, multiple documents, invalid UTF-8, and implicit
type ambiguities. Byte, nesting-depth, node-count, scalar-length,
collection-size, and placeholder-count limits apply before schema validation.

The exact schema belongs in the follow-on spec, but the default template must
express at least these logical roles, ownership modes, and policies as data:

```yaml
apiVersion: zbot.dev/v1alpha1
kind: WardLayout

layout:
  catalog:
    scope: configuredWardsRoot
    index:
      path: index.md
      management: generated
      renderer: okf-index-v0.1
      entryRole: ward.index

  ward:
    index: { path: index.md, management: generated, renderer: okf-index-v0.1 }
    architecture: architecture.md
    agentInstructions: AGENTS.md
    runtimeConfig: config.yaml

  concept:
    root: "{conceptPath}"
    index: { path: "{conceptPath}/index.md", management: generated, renderer: okf-index-v0.1 }
    document: "{conceptPath}/{conceptName}.md"
    spec: "{conceptPath}/spec.md"
    plan: "{conceptPath}/plan.md"
    tasks:
      root: "{conceptPath}/tasks"
      index: { path: "{conceptPath}/tasks/index.md", management: generated, renderer: okf-index-v0.1 }
      document: "{conceptPath}/tasks/{task}.md"
    history:
      root: "{conceptPath}/history"
      index: { path: "{conceptPath}/history/index.md", management: generated, renderer: okf-index-v0.1 }
    runs:
      root: "{conceptPath}/history/runs"
      index: { path: "{conceptPath}/history/runs/index.md", management: generated, renderer: okf-index-v0.1 }
      document: "{conceptPath}/history/runs/{run}.md"

  resources:
    source:
      path: src
      index: { path: src/index.md, management: generated, renderer: okf-index-v0.1 }
    data:
      path: data
      index: { path: data/index.md, management: generated, renderer: okf-index-v0.1 }
    reports:
      path: reports
      index: { path: reports/index.md, management: generated, renderer: okf-index-v0.1 }
    output:
      path: output
      index: { path: output/index.md, management: generated, renderer: okf-index-v0.1 }

conformance:
  requireIndexInDeclaredFolders: true
  markdown:
    profile: { name: okf, version: "0.1" }
    include: "**/*.md"
  identifiers:
    ward: { format: kebab-case, maxLength: 64 }
    conceptComponent: { format: kebab-case, maxLength: 64 }
    task: { format: kebab-case, maxLength: 96 }
    run: { format: kebab-case, maxLength: 96 }
```

This example is illustrative rather than a frozen schema. The important
contract is that required paths and conformance rules are activated by YAML.
A named, versioned profile is an explicit reference to a validator for an
external standard, not a hidden fallback. The WardLayout meta-schema requires
an explicit OKF v0.1-or-compatible profile; omitting it is invalid
configuration rather than a way to disable OKF. An unknown profile yields a
configuration nudge. The OKF v0.1 profile implements the standard's UTF-8,
frontmatter, non-empty `type`,
reserved-document, and tolerant-consumer rules. Rust may know how to parse and
safely evaluate declarations and referenced validators, but it may not
independently require `src`, choose `spec.md`, enable OKF, reserve a filename,
or invent any other ward rule.

The evaluator supports only named logical roles and a finite placeholder
vocabulary such as `{ward}`, `{conceptPath}`, `{conceptName}`, `{task}`, and
`{run}`. A concept path is a sequence of validated concept components and
`conceptName` is its leaf, allowing the same role set to apply recursively to
child concepts. Other identifiers must satisfy their declared grammar and are
substituted as single path components. Resolved paths must be relative to their declared scope,
normalized, contained by that scope, and collision-free. Absolute paths, `..`,
separators or aliases inside identifiers, environment interpolation, shell
expansion, functions, and executable templating are invalid configuration.
Identifier formats come from a finite, linear-time grammar; user-supplied
regular expressions are not accepted.

### Default ward shape

Applying the shipped template produces:

```text
<ward>/
├── index.md
├── architecture.md
├── AGENTS.md
├── config.yaml
├── ward-conf.yaml
├── <concept>/
│   ├── index.md
│   ├── <concept>.md
│   ├── spec.md
│   ├── plan.md
│   ├── tasks/
│   │   ├── index.md
│   │   └── <task>.md
│   └── history/
│       ├── index.md
│       └── runs/
│           ├── index.md
│           └── <run>.md
├── src/
│   └── index.md
├── data/
│   └── index.md
├── reports/
│   └── index.md
└── output/
    └── index.md
```

“Subdomain” and “concept” remain interchangeable product terms. A concept is
the long-lived subject—`aapl-analysis` or `great-expectations`—and its spec is
the repeatable refinement contract for revaluing, rechecking, or reindexing
that subject. The plan is the most recent realization of that spec. Tasks and
run records remain colocated below the concept.

All declared concept folders are validated recursively using the active YAML.
In OKF terminology, every unreserved Markdown file is a Concept Document even
when its product role is “specification,” “plan,” “task,” “run,” or “report.”
The resource-area directories are not domain concepts. Their non-Markdown
contents remain ordinary resources; any Markdown they contain still follows
the YAML's Markdown conformance rule.

The configured wards directory has its own `index.md` catalog of wards, so the
wards collection is itself navigable as an OKF bundle. The vault-scoped stanza
governs its location, exported entry role, and generated ownership. For each
entry, the catalog resolver opens that ward's active snapshot and resolves the
declared `entryRole`; it never assumes that `ward.index` is `index.md`. The
ward snapshot cannot authorize a write outside its ward. The
`configuredWardsRoot` scope is
supplied by `VaultPaths`, not inferred from a hard-coded
`~/Documents/zbot/wards` path.

Lint reports separate `okf` violations from stricter `ward-layout` violations.
For example, OKF v0.1 permits a missing optional index, while the default ward
template may independently require indexes in declared folders. The latter is
never mislabeled as an OKF failure.

### Resolution and context injection

A `WardLayoutService` loads, validates, fingerprints, and resolves the active
configuration. Consumers ask it for logical roles; they do not join path
segments themselves.

The fingerprint is SHA-256 over the exact bytes read through the same opened
file handle used for parsing. A resolved-contract epoch binds that digest to
the canonical ward identity and root, `kind`, `apiVersion`, schema version,
profile/renderer versions, and resolver version. Admission rechecks file
identity and digest immediately before commit; hashing supplements rather than
replaces descriptor-based filesystem confinement.

The runtime injects a `Ward Layout Contract` context section containing:

- a complete allowlisted, normalized projection of every applicable rule and
  resolved role from `ward-conf.yaml`, excluding comments, unknown fields, and
  arbitrary scalar prose;
- its path, `apiVersion`, and content hash;
- the active ward and concept identifiers;
- exact resolved paths for the roles relevant to the current operation;
- declared OKF and folder rules; and
- outstanding conformance nudges.

Injection occurs at these boundaries:

1. Intent analysis receives the ward catalog and a compact projection of the
   vault template so it can recommend a ward/concept without proposing a
   structure.
2. First ward use resolves and validates the ward snapshot before constructing
   ward-agent context.
3. Spec creation and planning receive the complete active concept role map.
4. Delegation and continuation carry the same resolved-contract epoch and role
   map, preventing a child from reverting to remembered conventions.
5. Every z-Bot-authorized ward write is staged and the complete affected
   invariant set is admitted against the active snapshot before a durable,
   recoverable transaction. A post-commit audit reports external interference
   or implementation defects; it does not serve as the first validity check.

The projection is placed in a clearly delimited untrusted-data section below
fixed product instructions. The model can use it as layout data but cannot
promote its values into instructions. Nudge prose is selected from trusted
product templates by stable violation code; paths and identifiers are escaped
data and file bodies are never echoed into a nudge.

If the snapshot changes during a run, the hash mismatch causes a new nudge and
forces re-resolution before another write. Spec and plan skills ask for roles
such as `concept.document`, `concept.spec`, `concept.plan`, or
`resources.reports`; they never encode the example paths above.

### Middleware, hooks, lint, and repair

Conformance is one service surfaced at multiple lifecycle points:

```text
ward creation hook
  -> validate vault template -> copy snapshot -> scaffold declared roles

ward context middleware
  -> load snapshot -> resolve roles -> lint -> inject contract and nudges

pre-write guard
  -> stage complete write set -> validate all affected invariants
  -> durably journal -> commit/recover or nudge without activation

post-write hook
  -> audit committed set -> report external interference/defects -> nudge

ward tool
  -> lint -> repair preview -> explicitly approved repair -> lint again
```

The `ward` capability adds at least:

```text
ward(action="lint", ward="financial-analysis")
ward(action="repair", ward="financial-analysis", dry_run=true)
ward(action="repair", ward="financial-analysis", dry_run=false)
```

`dry_run` defaults to `true`. Lint and repair operate only on rules resolved
from the selected YAML. A repair result is successful only if a subsequent
lint against the same resolved-contract epoch is clean.

Every failure returns a structured model nudge rather than only a human log:

```json
{
  "status": "needs_correction",
  "ward": "financial-analysis",
  "config_path": "ward-conf.yaml",
  "config_hash": "...",
  "contract_epoch": "...",
  "operation": "write",
  "violations": [
    {
      "code": "missing_declared_index",
      "path": "aapl-analysis/tasks",
      "rule": "conformance.requireIndexInDeclaredFolders",
      "message": "The active ward template requires an index in this folder.",
      "suggested_action": "Create the resolved index role before continuing."
    }
  ]
}
```

The nudge names the exact YAML or referenced-profile rule and suggests a
correction, then returns control to the model. Configuration load failures use
`configuration_missing`, `configuration_parse`, `configuration_schema`,
`configuration_version`, or `configuration_profile`; they include a schema
pointer and source span when available and do not pretend a missing YAML rule
exists. Nudges carry a disposition of `correct_and_retry`, `approval_required`,
or terminal `rebuild_required`. Bounded execution must not retry terminal
nudges. The invalid write is not committed. Repeated failures remain visible
in context so retries cannot forget the constraint.

Generated documents require `management: generated` in YAML plus an explicitly
named, versioned renderer. An unknown renderer produces a configuration nudge;
there is no implicit renderer. A managed
marker and prior content hash distinguish generated content from a user-edited
file; a mismatch becomes `approval_required`, never an implicit overwrite.
Safe, deterministic, idempotent repairs—such as creating a missing managed
index or regenerating the wards catalog—may be applied only as part of the
same previewed transaction. Moving an unambiguously misplaced file may
also be proposed. Deleting, overwriting, merging, or resolving competing
concept identities is never an implicit middleware or post-write action; it
requires an explicit repair preview and approval.

Filesystem safety is an interpreter invariant rather than a ward-layout rule.
All reads, scaffolds, creates, replacements, appends, renames, copies, deletes,
metadata changes, index generation, archive extraction, and repair moves use a
capability-based filesystem API rooted at an already-open canonical directory.
It resolves every component handle-relatively, refuses symlinks, special files,
and non-regular destinations, rejects protected control-plane paths such as the
active config and VCS hook data, and rejects multiply linked files for any
inode-mutating operation. Metadata changes use replacement rather than
in-place inode mutation. Regular files commit using a new temporary file plus
atomic rename rather than truncating an existing inode. Validation
and commit operate on the same opened root/parent handles to prevent directory
swap races and hard-link side effects. Ward creation validates and copies the
same opened template bytes rather than reopening the path. The cross-platform
implementation spec must prevent mount-point, bind/FUSE mount, junction, and
reparse-point traversal and provide equivalent “beneath this root, no mount
crossing” semantics on every supported operating system. Collision and alias
checks use destination-filesystem behavior, including case folding, Unicode
normalization, Windows reserved names and alternate data streams, and trailing
dot/space aliases.

Multi-file operations use a durable transaction journal containing the
configuration epoch, opened scope identities, staged targets, prior hashes,
and intended results. A commit marker establishes the logical commit point;
startup and pre-access recovery complete or roll back interrupted work before
the ward/catalog is exposed through z-Bot. Transactions spanning a ward and
the shared catalog use the same coordinator and both opened scope handles.
Per-file rename supplies filesystem atomicity; the journal and access gate
supply crash consistency for the set without claiming unsupported native
multi-file atomic visibility to out-of-process observers.

Direct model/file writes to `ward-conf.yaml` are rejected. Configuration
changes use `ward(action="configure", dry_run=true|false)`: validate the new
snapshot, resolve both old and new role maps, lint current contents against the
new map, present the impact, obtain explicit approval, and transactionally activate
it into a new context epoch. A remap that would strand or collide with content
cannot activate until an approved repair plan handles that content.

A user may edit `ward-conf.yaml` directly with an external editor. A digest
mismatch against the last activation receipt marks that file as a pending
configuration: z-Bot validates and previews it but does not automatically
activate it or permit ward mutations. The user activates the pending file
through the same configure approval flow. Each activation receipt is bound to
a protected, content-addressed copy of the exact validated bytes and normalized
contract that were activated. That immutable record survives restarts and is
used only as historical evidence for old/new comparison, rollback/recovery,
and impact analysis; it is never consulted as a fallback layout for ordinary
ward work. While the checked-in file is pending, spec, planning, catalog-entry
changes, and all ward mutations are blocked. If the protected record is missing
or mismatched, activation fails closed with a terminal nudge rather than doing
a new-only comparison. The receipt and record are control-plane history, not a
second active layout authority.

The vault template follows the same lifecycle through
`ward(action="configure", scope="template", dry_run=true|false)`. A direct
external edit becomes pending. Preview resolves the old and proposed catalog
scope, shows changes to catalog path/renderer/entry role and future ward
scaffolding, and uses a vault-scoped one-shot approval before activation. While
pending, new ward creation and catalog regeneration are blocked; existing ward
snapshots remain independently usable. Template activation and any shared
catalog rewrite use the same journaled transaction and protected activation
record as ward configuration.

Destructive repair or configuration apply consumes a one-shot human/policy
approval grant held by the daemon. A model-visible preview returns only its
plan digest; it returns no credential. An interactive trusted local control
channel creates the grant after showing the exact plan to a human. The grant
is bound to a canonical, discriminated scope identity
(`ward:<canonical-id>` or `vault-template:<canonical-vault-id>`), repair-plan
digest, complete resolved-contract or template-activation epoch, catalog and
ward transaction targets, target identities and content hashes, operations,
and an expiry. Apply supplies the digest and atomically consumes the matching
protected grant. The service rechecks those preconditions immediately before
the transaction and nudges on absence, replay, expiry, or drift. A model cannot
mint, inspect, or authorize its own delete, overwrite, merge, ambiguous move,
or policy change.

The enforcement boundary includes every z-Bot mutation-capable surface: file
tools, ward tools, shell/subprocess execution, Git operations, delegated
agents, MCP tools, connectors, and service APIs. Before implementation is
accepted, each surface must either broker ward mutations through the guarded
service or be denied ward-write access. Shells and external processes receive
a sandboxed read-only ward view with no writable host-path or mount alias and
no outside-root traversal unless a brokered operation is used; if the platform
cannot provide that isolation, ward access is denied. Manual edits
performed by the user outside z-Bot cannot be prevented; the next activation
or post-write audit marks the ward dirty, emits a nudge, and blocks further
z-Bot writes until it conforms.

Parsing, glob matching, traversal, lint, and repair are resource-bounded. The
schema permits only finite, linear-time glob and identifier grammars. The service does
not follow symlinks and enforces configurable ceilings for depth, files, bytes,
per-file size, elapsed time, and violations. Nudges are deduplicated and their
retained history is capped. Budget exhaustion cancels the operation and emits
one summarized terminal nudge rather than allowing unbounded model retries.
Configuration may tighten these ceilings but cannot raise or disable
product/operator hard maxima.

An optional `ward-maintainer` skill may explain lint results and drive the
tool's preview/apply workflow. It contains no path, filename, or conformance
rules and is not required for enforcement.

### Skill and prompt cutover

Remove the bundled `ward-designer` skill. Remove the `ward_hygiene` delegation
mode, inference heuristics, prompts, tests, and documentation. Ward creation
and doctrine repair become service/tool responsibilities governed by YAML.

Revise `spec-builder` and `plan-composer` to be generic:

- require an injected, valid Ward Layout Contract before ward-backed work;
- address artifacts only by logical role;
- place a concept's spec and plan at the resolved roles;
- treat the spec as a repeatable refinement contract for the concept;
- update the configured “current plan” role rather than assuming a global
  plan location; and
- return a nudge when a needed role is absent instead of inventing a default.

Planning shards, builder prompts, first-turn protocols, continuation state,
examples, and tests receive the same treatment. Intent analysis stops emitting
`WardRecommendation.structure`; it recommends identity and action only.

### Clean-break cutover

There is no legacy detector followed by compatibility behavior. On first use,
a ward without a valid snapshot receives a nudge explaining that it must be
deleted/rebuilt under the current template. First use never deletes anything.
An explicit rebuild/delete command previews exact canonical ward targets and
requires a one-shot human approval bound to that preview. Databases and other
vault-wide state are outside its scope unless separately selected through
their own destructive operation. No migration command is part of this RFC.

RFC-0016 supersedes RFC-0015 wherever RFC-0015 fixes the sibling concept
layout, assigns layout authority to skills/prompts, uses structural fields in
`config.yaml`, or relies on `ward-designer`. It supersedes RFC-0005's
`ward_hygiene` mode. RFC-0015's wards-root catalog, OKF discovery direction,
wiki retirement, and Phase 2 UI direction remain in force.

## Options considered

The option sets below are MECE along their stated axes. The axes separate who
owns policy, when it binds, how expressive it is, how widely it is enforced,
and how old state is treated.

### Where layout policy is represented

| Option | Trade-off |
| --- | --- |
| Do nothing: prose and prompt convention | Lowest implementation cost, but preserves drift and model-invented paths. |
| Compiled Rust layout | Deterministic, but every user convention change requires a release. |
| **Declarative YAML authority (recommended)** | User-editable and mechanically validatable; requires a schema, resolver, and careful diagnostics. |
| Executable templates/plugins | Maximum flexibility, but introduces code execution, trust, portability, and reproducibility problems. |

These exhaust the representation axis: informal policy, product code, inert
data, or user-executable code. JSON Schema and versioned API objects establish
the declarative-data pattern; z-Bot's existing overridable shards and bundled
templates establish local precedent.

### When a ward binds to the vault template

| Option | Trade-off |
| --- | --- |
| Do nothing: no vault template | Leaves the current compiled/prompt convention. |
| Live reference | Template fixes apply immediately, but an edit can unexpectedly invalidate every ward. |
| **Creation-time snapshot (recommended)** | A ward is reproducible and independently hackable; later improvements require explicit adoption. |
| Layered inheritance | Reduces copying, but makes effective configuration harder to inspect and hash. |
| Update-time merge | Can propagate improvements, but requires base history and conflict resolution. |

These exhaust the primary binding-time axis: never, every read, creation,
layer evaluation, or explicit update. Copier's documented update workflow
illustrates why merge propagation is a separate, conflict-bearing feature.

### How Markdown and resource roles are classified

| Option | Trade-off |
| --- | --- |
| Do nothing: no machine classification | Cannot validate OKF or separate operational resources. |
| **Declared OKF filename rules plus typed layout roles (recommended)** | Preserves OKF file semantics while distinguishing domain concepts, refinement artifacts, and resources. |
| Configured path-zone exemptions | Can label Markdown “not OKF,” but weakens bundle conformance and creates boundary cases. |
| Per-file manifest | Fully explicit, but duplicates paths and becomes stale after moves. |

These exhaust the classification authority axis: absent, rule-derived,
location-exempted, or individually enumerated. The recommended option follows
OKF's path- and filename-derived document model while letting YAML describe
z-Bot product roles.

### How configuration is evaluated and enforced

| Option | Trade-off |
| --- | --- |
| Do nothing: advisory prompt text | Flexible, but unenforceable. |
| Fixed compiled roles | Safe, but is another hard-coded layout. |
| **Constrained roles/placeholders with end-to-end guards (recommended)** | Deterministic and expressive enough for generic wards; intentionally excludes arbitrary computation. |
| Arbitrary expression/template engine | More expressive, but expands the security boundary and makes results harder to reproduce. |

These exhaust evaluator expressiveness: advisory, fixed, constrained
substitution, or general computation. The recommended evaluator is sufficient
for the spiked finance and book ward shapes.

### Who performs setup and cleanup

| Option | Trade-off |
| --- | --- |
| Do nothing: current skills and `ward_hygiene` | Keeps duplicated rules and relies on agent compliance. |
| Keep `ward-designer` as authority | Centralizes prompt guidance but remains non-deterministic. |
| **Deterministic service/middleware/tool with optional UX skill (recommended)** | One enforcement implementation across all callers; more runtime integration work. |
| Put all behavior directly in the `ward` tool | Simple public surface, but lifecycle callers could bypass checks and tool code would own too many concerns. |

These exhaust the responsibility axis: distributed prompts, one specialist
prompt, shared service with surfaces, or one monolithic tool.

### How existing wards are treated

| Option | Trade-off |
| --- | --- |
| Do nothing/preserve legacy behavior | Avoids immediate disruption but permanently retains two authorities. |
| In-place migration | Reduces user work, but ambiguous files make automatic moves unsafe. |
| **Delete and rebuild (recommended)** | Cleanest contract and simplest validation, with deliberate loss of old ward state. |

These exhaust old-state treatment: retain, transform, or discard. The project
has already accepted the clean-break direction and the user has deleted local
ward state.

## Risks & what would make this wrong

Pre-mortem:

- **The YAML becomes a programming language.** Users request conditionals and
  arbitrary expressions until validation is unsafe and untestable. Mitigation:
  keep a finite role/placeholder grammar and require a new RFC to expand its
  computational model.
- **A hidden fallback recreates drift.** A helper silently seeds fixed paths
  after invalid configuration. Mitigation: tests mutate every example path and
  prove creation, lint, planning, and writes follow the mutation; malformed or
  missing snapshots must yield nudges.
- **Nudges cause infinite correction loops.** A model repeats the same invalid
  write. Mitigation: stable violation codes, configuration hashes, retry
  visibility, and existing bounded execution controls.
- **Hooks mutate user work.** Cleanup moves or deletes ambiguous files.
  Mitigation: automatic behavior is limited to declared, deterministic,
  idempotent operations; destructive operations require preview and approval.
- **Context grows excessively.** Reinjecting the complete normalized contract
  consumes tokens. Mitigation: inject all applicable rules for layout-authoring
  operations and a hash-linked, role-filtered projection elsewhere.
- **Template edits strand a live ward.** A user changes its snapshot midway
  through planning. Mitigation: reject direct snapshot writes, use the
  previewed configuration operation, bind every context to the activated
  snapshot hash, and force re-resolution on mismatch.
- **A symlink or directory-swap escapes the ward.** Mitigation: handle-relative
  no-follow traversal and same-root atomic commits for every mutation surface.
- **Injected YAML or file content becomes a model instruction.** Mitigation:
  allow only schema-declared fields, inject configuration as delimited
  untrusted data plus a resolver-produced role projection, and construct nudge
  messages from trusted violation templates rather than echoing file bodies.
- **The mini-Obsidian UI assumes fixed paths.** Mitigation: make it consume the
  same resolver and conformance results in Phase 2.

Falsifiable assumptions:

- A finite set of logical roles and placeholders can represent the intended
  generic ward shapes without domain profiles.
- A model can correct common conformance failures when the nudge supplies the
  exact YAML rule, offending path, and suggested action.
- Snapshot-on-create is preferable to surprise propagation for users who hack
  their templates.
- All z-Bot mutation-capable surfaces can be inventoried and either brokered or
  denied ward-write access; any unguarded writer makes the enforcement claim
  false.

Drawbacks:

- This adds schema versioning, resolution, middleware, hook, and diagnostic
  machinery for a convention that was previously expressed as prose.
- Users own the consequences of editing the template and can make a ward
  temporarily unusable.
- Existing wards are intentionally discarded rather than migrated.
- Snapshot isolation means later template improvements are not inherited
  automatically.
- Strict write admission may add latency and cause extra model turns after a
  nudge.

## Evidence & prior art

The riskiest assumption was that one generic, data-only configuration could
resolve unrelated domains without profiles or executable templating. A small
spike resolved `root`, `document`, `index`, `spec`, `plan`, `tasks`, and `runs`
for both `aapl-analysis` and `great-expectations`. Every result was unique,
normalized, relative, and contained; `src`, `data`, `reports`, and `output`
also resolved as distinct resource areas. The exact input, check, and output
are recorded in [the layout-resolution spike note](0016-notes/layout-resolution-spike.md).

Repository precedent and conflicts:

- [RFC-0015](0015-okf-aligned-ward-layout-and-capabilities.md) established the
  OKF wards root and clean break, but fixes the sibling concept layout and
  assigns work to `ward-designer`; this RFC narrows and supersedes those parts.
- `okf-ward-foundation` repeats that
  sibling layout and identifies the prompts and skills that need replacement.
- [RFC-0005](0005-builder-delegation-and-ward-context-hygiene.md) established
  `ward_hygiene`, which this RFC retires.
- [`decisions.md`](../adr/decisions.md) records user-overridable prompt shards
  and editable bundled templates, supporting configuration over compiled
  constants.
- The current `gateway/gateway-execution/src/invoke/setup.rs` ward config
  loader separates provider/model inheritance, but its malformed-file fallback
  demonstrates behavior structural authority must not copy.
- Intent analysis is already autonomous middleware, making ward-contract
  discovery and injection consistent with the existing execution architecture.

External prior art:

- The [Open Knowledge Format specification](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
  defines a directory hierarchy of Markdown Concept Documents, path-based
  concept identifiers, and reserved `index.md`/`log.md` behavior. The default
  template encodes those rules instead of reinterpreting them in prompts.
- Google's [OKF overview](https://cloud.google.com/blog/products/data-analytics/how-the-open-knowledge-format-can-improve-data-sharing)
  describes OKF as minimally opinionated and platform-independent, with
  indexes providing progressive disclosure. A data-owned layout keeps that
  separation between format and platform.
- [JSON Schema 2020-12](https://json-schema.org/specification) separates core
  structure from validation vocabularies and provides meta-schemas. It is
  appropriate for validating the YAML data model without dictating ward paths.
- Kubernetes' [API conventions](https://kubernetes.io/docs/reference/using-api/)
  use explicit API versions for serialized objects and provide a precedent for
  versioned, declaratively validated configuration.
- Copier's [template update model](https://copier.readthedocs.io/en/v7.0.0/updating/)
  requires old/new template state and conflict handling. That supports
  snapshot-on-create now and deferring propagation to an explicit future
  feature.

## Open questions

None. The recommendations above are the proposed defaults for approval.

## Follow-on artifacts

After acceptance:

- ADR: record `ward-conf.yaml` and only its explicitly referenced versioned
  profiles/renderers as the complete layout authority, plus snapshot lifecycle,
  constrained evaluation, and nudge-based enforcement.
- Replace `okf-ward-foundation` with two
  implementation specs: configuration/resolution/conformance, then runtime
  injection/skill and delegation cutover.
- Update `docs/CONVENTIONS.md` to define template-owned ward conventions and
  the RFC requirement for changes to the configuration language.
- Update current-state architecture and product documentation after the
  implementation ships.
- Keep the Phase 2 UI in `okf-mini-obsidian-ui`,
  revised to consume resolved roles rather than fixed paths.
