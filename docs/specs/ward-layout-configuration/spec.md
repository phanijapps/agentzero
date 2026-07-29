# Spec: Fluid Ward Template and Conformance

- **Status:** Implemented (Phase 1)
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [ADR-0001](../../adr/0001-use-versioned-ward-layout-contracts.md), [RFC-0016](../../rfc/0016-generic-ward-configuration-and-layout-resolution.md)
- **Brief:** none
- **Contract:** none; `ward-conf.yaml` is the runtime contract
- **Shape:** integration

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it.

## Objective

Make one user-editable `ward-conf.yaml` describe a ward without compiling the
current ward shape into Rust. A fresh vault receives the default template and a
new ward receives a snapshot. The validated YAML is injected into ward-backed
agent context, and one generic linter checks the filesystem against only the
paths and conformance rules declared by that snapshot. Artifact roles such as
concept, spec, plan, tasks, source, reports, or output are template data: users
may remove, rename, or add them without a product code change.

## Generic rule language

Only the envelope and interpreter vocabulary are compiled. Role `id` values and
the presence of rule nodes are user-owned data:

```yaml
apiVersion: zbot.dev/v1alpha1
kind: WardLayout
definitions:
  concept:
    kind: directory
    children:
      - { id: concept-index, match: index.md, kind: file, format: okf-v0.1 }
      - { id: concept-document, match: "{name}.md", kind: file, format: okf-v0.1 }
      - { id: specification, match: spec.md, kind: file, format: okf-v0.1 }
      - id: nested-concepts
        match: "*"
        required: false
        repeat: true
        exclude: [tasks, history]
        $ref: concept
root:
  kind: directory
  children:
    - { id: ward-index, match: index.md, kind: file, format: okf-v0.1 }
    - id: concepts
      match: "*"
      required: false
      repeat: true
      exclude: [src, data, reports, output, .zbot]
      $ref: concept
extensions: {}
```

`definitions` keys and rule `id` values are arbitrary safe identifiers. Rule
objects are strict and support only `id`, `kind` (`file|directory`), `match`,
`required`, `repeat`, `exclude`, `format` (`raw|markdown|okf-v0.1`), `children`,
and `$ref`. A non-reference rule requires `kind`; a `$ref` rule must omit
`kind`, `format`, and `children` and inherits them from the definition. Unknown
rule keys/types, unresolved or non-advancing reference cycles, unsupported
formats, and overlapping sibling matchers of the same filesystem kind are
invalid.
`match` is one path component: a literal, `*`, a single-star pattern such as
`*.md`, or `{name}` embedded in a component. Separators, `..`, absolute paths,
`?`, character classes, and other placeholders are invalid. `{name}` is the
current directory basename.

`required` defaults true. `repeat: true` validates every existing match;
`required: false` permits zero matches. Creation materializes only required,
literal, non-reference nodes and never invents wildcard/repeatable values.
`$ref` supplies `kind` and `children` from a named definition while the
referring node supplies its identity and match. Recursive refs are evaluated
only while descending existing children and remain budget-bound.

Literal and `*` patterns of different kinds do not overlap because one
filesystem entry cannot be both file and directory. Literal/single-star
same-kind overlap is rejected statically. `{name}` is resolved per visited
directory and checked for collision then; for example a concept directory named
`spec` conflicts with sibling `spec.md` and receives a configuration nudge.

Unknown top-level metadata is retained for round-trip editing but is never
interpreted or injected. Model context contains only the typed rule projection,
resolved ward-relative values, and snapshot digest.

## Boundaries

### Always do

- Treat `apiVersion` and `kind` as the small versioned envelope and preserve
  every other YAML mapping as generic data.
- Resolve vault/template/ward locations through `VaultPaths`; never assume a
  fixed z-Bot installation path.
- Copy the template into a new ward and lint against that ward-owned snapshot.
- Open both vault templates and ward snapshots handle-relatively beneath their
  expected opened parent with no-follow, regular-file-only semantics; reject
  symlinks, special files, target swaps, and multiply-linked files.
- Inject a bounded normalized representation as delimited data, not trusted
  instructions.
- Apply an OKF profile only when the template declares it; distinguish OKF
  findings from template-structure findings.

### Ask first

- Changing the generic template-language primitives, version envelope, or
  supported OKF profile.
- Adding executable templates, environment interpolation, user regexes, or a
  dependency.

### Never do

- Define Rust fields for today's artifact roles or require `spec`, `plan`,
  `tasks`, `src`, `data`, `reports`, or `output` in compiled code.
- Invent an undeclared path, role, renderer, or fallback ward shape.
- Add destructive repair, approval brokers, migration, compatibility readers,
  Phase 1 UI work, or broad concept-mutation APIs in this slice.
- Follow a symlink outside the configured wards root or expose raw host paths,
  secrets, provider configuration, or file bodies in model nudges/context.

## Testing Strategy

- **Envelope, YAML limits, generic preservation, path expansion, and lint rules
  — TDD:** exact input/output invariants are unit tested.
- **Template seeding, ward snapshot creation, recursive lint, and context
  injection — integration tests:** temporary vaults exercise real files.
- **Fluidity — goal-based E2E:** two structurally different valid templates,
  including one with no spec/plan roles, run without source changes.

No visual QA is required.

## Acceptance Criteria

- [ ] A fresh vault writes `config/templates/ward-conf.yaml` only when absent;
  restarts never overwrite a user-edited template.
- [ ] The loader requires only supported `apiVersion` and `kind`, rejects unsafe
  YAML before unbounded allocation (131,072 raw bytes; depth 32; 4,096 nodes;
  512 entries per collection; 4,096 bytes per scalar), including invalid UTF-8,
  aliases/anchors, merge keys, custom tags, duplicate/non-string keys, and
  multiple documents, and preserves arbitrary non-rule metadata without
  deserializing today's artifact names into Rust fields.
- [ ] Creating a ward validates and copies the exact template bytes to
  `<ward>/ward-conf.yaml` before creating paths declared by that snapshot. It
  opens the canonical wards root once, rejects an existing destination, creates
  beneath that handle with create-new/no-follow semantics, rejects symlink or
  non-directory intermediates and target swaps, and never overwrites a file.
- [ ] Adding, renaming, or removing a declared artifact—including removing all
  spec/plan/task entries—changes setup, lint, and injected context without a
  Rust, prompt, or skill edit.
- [ ] The generic linter validates required declared files/directories,
  repeatable/nested concept patterns, and declared Markdown/OKF rules while
  allowing undeclared non-Markdown contents in resource directories.
- [ ] Lint is capped at depth 32, 10,000 visited entries, 5,000 files, 64 MiB
  total bytes, 1 MiB per Markdown file, five seconds, and 500 findings. Budget
  exhaustion cancels traversal, returns one terminal bounded nudge, and never
  retries automatically.
- [ ] `ward(action="lint")` returns a bounded deterministic report tied to the
  snapshot digest; setup and post-write middleware use the same linter.
- [ ] Invalid YAML, unresolved placeholders, unsafe paths, collisions, missing
  required nodes, and OKF violations fail closed with a trusted-template nudge
  containing only bounded ward-relative typed data.
- [ ] The default template produces the accepted OKF-style ward and recursively
  applies its concept rules, while operational resource areas remain
  non-concepts and may contain non-Markdown files.
- [ ] Old wards receive `rebuild_required`; no legacy read, migration, automatic
  deletion, approval service, or hidden fallback layout exists.

## Assumptions

- Technical: `serde_yaml::Value` can retain the generic data body while Rust
  validates a small typed envelope and bounded interpreter primitives (source:
  workspace dependencies and user correction 2026-07-19).
- Product: subdomain and concept remain interchangeable labels, but neither is
  a mandatory compiled field (source: user confirmation 2026-07-19).
- Product: specs/plans/tasks are optional template choices and may disappear in
  future ward templates (source: user correction 2026-07-19).
- Product: users delete old wards/database state; there is no cutover work
  (source: user confirmation 2026-07-19).
