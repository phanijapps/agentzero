# Spec: Ward Archetype Registry and Creation

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [RFC-0017](../../rfc/0017-ward-layout-archetypes.md), [ADR-0002](../../adr/0002-select-complete-ward-archetypes-at-creation.md), [RFC-0016](../../rfc/0016-generic-ward-configuration-and-layout-resolution.md)
- **Brief:** `dynamic-ward-layout-archetypes`
- **Contract:** none
- **Shape:** service

Product provenance: [Dynamic Ward Layout Archetypes brief](../../product/briefs/dynamic-ward-layout-archetypes.md).

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Let a user or trusted runtime caller create a ward from an explicit closed
archetype identifier. A fresh vault exposes locally seeded `generic` and
`coding` bundles; creation validates the selected complete bundle, atomically
materializes it, copies the exact YAML snapshot, and reports durable
archetype provenance without weakening existing layout safety or reuse
semantics.

## Boundaries

### Always do

- Resolve a typed archetype identifier through `VaultPaths` and the local
  registry before any filesystem effect.
- Apply the existing bounded Ward Layout loader/compiler and no-replace
  publication rules to every archetype.
- Treat the copied `<ward>/ward-conf.yaml` bytes and digest as authority after
  creation; archetype metadata is provenance only.
- Test fresh-vault seeding, user edits, all resource ceilings, symlink/path
  attacks, atomic cleanup, and cross-platform behavior supported by the
  current creation service.

### Ask first

- Adding, removing, or renaming a public archetype identifier.
- Introducing persistence outside the existing vault/ward metadata surfaces.
- Changing a shipped archetype after this spec is approved in a way that
  removes or relocates user-visible starter paths.

### Never do

- Never accept a template path, arbitrary registry key, free-form YAML, or
  filesystem separator from a caller in place of `WardArchetypeId`.
- Never add a second layout parser, compiled fallback ward shape, inheritance
  engine, executable hook, remote fetch, or environment/shell interpolation.
- Never create a new top-level crate or bypass `gateway-services` Ward Layout
  creation and `VaultPaths` boundaries.
- Never overwrite a user-edited seeded bundle or partially publish a ward
  after validation or resource-limit failure.

## Testing Strategy

- **Identifier parsing and path/resource validation — TDD:** these are compact
  invariants with adversarial edge cases and must be red before implementation.
- **Seed migration and explicit creation — goal-based integration tests:** the
  outcome depends on `VaultPaths`, seed copying, the compiler, and atomic
  publication working together.
- **Snapshot/provenance/reuse — goal-based integration tests:** tests compare
  selected bytes, digest, reported identifier, and a later load after the
  source bundle changes.
- **Clean-start acceptance — goal-based E2E:** start the normal application
  bootstrap against a fresh database and fresh vault because existing fixtures
  cannot prove first-run seed and creation behavior.

TDD stub handoff: AC1 and AC4 are concrete enough to stub in Rust as enum
round-trip/rejection and confined-resolution tests. Per `new-spec`, no test
stub is committed during spec authoring; `work-loop` PLAN must materialize and
compile the red stubs in `gateway/gateway-services/src/ward_layout/archetype.rs`
before EXECUTE.

## Acceptance Criteria

- [x] `WardArchetypeId` accepts exactly `generic`, `coding`,
  `documentation`, `journal`, `ebook`, `research`, and `news`, serializes as
  those snake-case strings, and rejects every other value without treating it
  as a path.
- [x] A fresh vault seeds complete, editable `generic` and `coding` bundles at
  the canonical archetype registry location without overwriting an existing
  file or directory.
- [x] A caller can explicitly create a ward with `generic` or `coding`; omitting
  the optional identifier at this service boundary resolves to `generic`.
- [x] The selected bundle is resolved by a confined `VaultPaths` mapping;
  absolute paths, `..`, separators, non-UTF-8 components, aliases, hard-link or
  symlink escape, and case-fold collisions cannot select content outside the
  bundle.
- [x] Each bundle is loaded through the existing safe YAML and Ward Layout
  compiler limits; unsupported YAML features, invalid schema, path collisions,
  and undeclared starter destinations fail before publication.
- [x] The selected bundle's `ward-agent.md` is the sole doctrine source and
  renders to `AGENTS.md` through the existing byte, placeholder, no-link,
  injection, and diagnostic controls; it cannot overlay or fall back to
  another archetype's doctrine.
- [x] Starter content is limited to at most 128 files, 16 relative path
  components, 256 KiB per file, and 4 MiB total; tests at each limit succeed
  and tests one unit above each limit fail with no visible ward.
- [x] Successful creation copies byte-for-byte the selected
  `ward-conf.yaml`, materializes only its declared required tree and starter
  files, and returns a digest equal to the copied snapshot.
- [x] `WardRecord.archetype: Option<WardArchetypeId>` in the existing atomic
  `<vault>/wards/.usage.json` sidecar is the durable provenance field and is
  exposed in the created result and ward layout state; changing registry
  content after creation does not change the existing ward's snapshot, digest,
  tree, or recorded provenance, and snapshot bytes remain structural authority
  if metadata is absent or disagrees.
- [x] Missing or invalid content for an explicitly resolved registry entry
  returns a bounded actionable error, does not reveal an internal absolute
  path, does not fall back to another archetype, and leaves no final or staging
  ward behind.
- [x] Existing singular-template and doctrine users receive a documented,
  non-destructive
  transition to the `generic` bundle; a user-edited canonical template wins
  over shipped defaults and no hidden singular template remains active.
- [x] Existing Ward Layout loader, compiler, lint, and snapshot tests remain
  green, and both reference archetypes pass the same construction harness.
- [x] With a fresh database and fresh vault, normal bootstrap seeds the
  registry and explicit `coding` creation produces the coding snapshot and
  expected tree without manual fixture setup.

## Assumptions

- Technical: Rust services already load, compile, atomically materialize, and
  digest arbitrary data-only Ward Layout documents (source:
  `gateway/gateway-services/src/ward_layout/create.rs` and the approved spike
  in RFC-0017).
- Technical: template selection enters through the single
  `create_ward_from_template` adapter seam and path ownership remains in
  `VaultPaths` (source: Codegraph evidence in
  `docs/product/briefs/dynamic-ward-layout-archetypes.md`).
- Technical: no standalone external contract is required; the interface is an
  internal Rust enum and existing tool schema (source: user confirmation
  2026-07-26).
- Process: the change is governed by RFC-0017 and proposed ADR-0002 because it
  expands RFC-0016/ADR-0001 across packages (source:
  `docs/CONVENTIONS.md`, user confirmation 2026-07-26).
- Product: archetypes are selected once at creation, `generic` is both a valid
  explicit choice and safe default, and existing wards never reclassify
  (source: user confirmation 2026-07-26).
- Product: the authoritative acceptance test uses a fresh database (source:
  user confirmation 2026-07-26).
