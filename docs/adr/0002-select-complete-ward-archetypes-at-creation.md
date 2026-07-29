# ADR-0002: Select Complete Ward Archetypes at Creation

- **Status:** Accepted
- **Date:** 2026-07-26
- **Deciders:** phanijapps
- **Supersedes:** none
- **Related:** [RFC-0017](../rfc/0017-ward-layout-archetypes.md), [Dynamic Ward Layout Archetypes brief](../product/briefs/dynamic-ward-layout-archetypes.md)

## Context

ADR-0001 established a versioned, data-only `ward-conf.yaml` contract and made
the exact configuration copied into a ward the authority for that ward.
RFC-0016 implemented the first slice around one vault template at
`config/templates/ward-conf.yaml`. That singular template creates a generic,
documentation-oriented shape regardless of whether the new work is coding,
journaling, ebook organization, research, or news.

The Ward Layout loader, compiler, materializer, snapshot digest, and linter are
already generic. The missing architectural boundary is selection: ward
creation reads one fixed template, and intent analysis can emit a free-form
structure proposal that must not become filesystem authority.

This decision extends ADR-0001's singular-template creation clause; it does
not retire ADR-0001's snapshot authority, resolver, versioning, or conformance
decisions.

The choice is constrained by:

- A created ward must retain one deterministic and inspectable layout
  snapshot.
- Model output is untrusted and must not author YAML or select filesystem
  paths.
- Configuration remains local, user-editable, data-only, schema-bounded, and
  non-executable.
- Existing wards cannot change meaning when a later ask has a different task
  type.
- Invalid configuration must fail before atomic publication and must not
  activate a compiled fallback layout.

## Decision

> We will select one complete, allowlisted ward archetype at ward creation and
> make the copied `ward-conf.yaml` snapshot the immutable structural authority
> for that ward.

The initial closed identifiers are `generic`, `coding`, `documentation`,
`journal`, `ebook`, `research`, and `news`. An explicit valid user override
wins. Otherwise, intent analysis may recommend a closed identifier only when a
new ward is being created. Absent, low-confidence, unavailable, or
unrecognized classification resolves to `generic`; a missing or invalid
registry bundle fails closed.

Each archetype is one complete local bundle containing a data-only
`ward-conf.yaml`, bounded starter Markdown, and a bounded `ward-agent.md`
template rendered to the ward's `AGENTS.md` through the existing doctrine
loader and injection controls. Bundles are resolved through a confined
`VaultPaths` mapping from the closed identifier. They cannot inherit or merge
layouts, execute hooks, interpolate shell or environment values, fetch remote
content, or accept a model-authored path.

Creation validates the complete selected bundle before effects, stages the
declared tree, copies the exact YAML bytes into the ward, records the
archetype identifier as provenance, computes the active digest from the copied
snapshot, and publishes atomically. On reuse, only the ward snapshot is
structural authority; current intent output and later registry edits are
ignored.

## Consequences

**Positive:**

- New wards begin with a structure suited to their primary work without
  introducing another layout interpreter.
- Filesystem policy becomes deterministic after a bounded identifier is
  selected, even when the recommendation originated with a model.
- Existing snapshots, digest semantics, lint behavior, and local
  customization remain intact.
- Complete bundles are independently inspectable and construction-testable.
- `generic` provides a documented conservative fallback for classifier
  uncertainty.

**Negative:**

- Shared declarations and doctrine may be duplicated across seven complete
  bundles.
- Adding or renaming an archetype becomes a governed contract change rather
  than an arbitrary directory addition.
- Registry template fixes affect future wards only; existing wards must be
  rebuilt until an explicit adoption workflow exists.
- The create contract, intent contract, provenance state, seed migration, and
  test harness all gain archetype awareness.

**Neutral / to revisit:**

- Archetype-specific operations such as `add_book` and
  `create_daily_entry` are deferred.
- Explicit, previewed migration between archetypes may be proposed later.
- If repeated safe changes across every bundle become materially costly, a
  separately governed composition model may be reconsidered.
- Fresh-database and fresh-vault construction is the authoritative end-to-end
  acceptance environment for the first release.

## Alternatives considered

- **Keep one generic template.** Rejected because it cannot make coding,
  journal, ebook, research, and news wards useful at creation.
- **Compose a generic base with archetype overlays.** Rejected for the first
  release because merge precedence, deletions, and multi-source provenance
  weaken the one-complete-snapshot model.
- **Generate or switch layouts per request.** Rejected because model output
  would change paths, digests, plans, and conformance behavior after creation.
- **Require user-only selection.** Rejected because it forgoes safe automatic
  setup; the closed recommendation plus explicit override retains the same
  policy boundary.
- **Let skills, prompts, executable hooks, or remote templates define an
  archetype.** Rejected because those choices create probabilistic,
  code-execution, or supply-chain authority outside the bounded Ward Layout
  contract.

## References

- [RFC-0017: Ward Layout Archetypes](../rfc/0017-ward-layout-archetypes.md)
- [RFC-0016: Generic Ward Configuration and Layout Resolution](../rfc/0016-generic-ward-configuration-and-layout-resolution.md)
- [ADR-0001: Use Versioned Ward Layout Contracts](0001-use-versioned-ward-layout-contracts.md)
