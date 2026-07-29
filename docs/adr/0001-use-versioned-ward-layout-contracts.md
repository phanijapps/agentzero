# ADR-0001: Use Versioned Ward Layout Contracts

- **Status:** Superseded by ADR-0003
- **Date:** 2026-07-19
- **Deciders:** phanijapps, zbot maintainers
- **Supersedes:** none
- **Related:** [RFC-0016](../rfc/0016-generic-ward-configuration-and-layout-resolution.md), [RFC-0015](../rfc/0015-okf-aligned-ward-layout-and-capabilities.md), [RFC-0005](../rfc/0005-builder-delegation-and-ward-context-hygiene.md)

## Context

Wards are durable workspaces and OKF knowledge bundles, but their layout has
been duplicated across RFC prose, specifications, prompt shards, skills, Rust
services, and tool implementations. Those copies have already disagreed about
whether a concept document is beside or inside its concept folder. Agents can
also construct paths from remembered prompt conventions, bypassing newer
documentation.

The layout must remain user-hackable, work for unrelated domains, support
non-Markdown operational resource areas, and make every concept subtree follow
OKF rules. Spec and planning workflows need exact artifact locations without
embedding one filesystem convention. Model-visible failures must be
correctable, while filesystem confinement and destructive cleanup must remain
deterministic and approval-bound.

RFC-0016 evaluated prompt conventions, compiled layouts, declarative data, and
executable templates, and accepted a clean break with no legacy ward migration
or compatibility behavior.

## Decision

> We will use a versioned, declarative `ward-conf.yaml` contract as the complete activation authority for ward layout and conformance.

The user-editable canonical template lives at
`<vault>/config/templates/ward-conf.yaml`. Ward creation validates it and
copies an independently activatable snapshot to `<ward>/ward-conf.yaml`.
Existing wards use their snapshot; later template edits do not silently
propagate.

The contract declares logical artifact roles, resource areas, generated-file
ownership, identifier formats, and explicitly selected versioned OKF profiles
and renderers. The configuration grammar and filesystem safety invariants may
be compiled, but no hidden ward shape, implicit renderer, undeclared profile,
or malformed-configuration fallback is permitted.

A deterministic resolver supplies normalized logical roles to ward lifecycle
middleware, spec and planning workflows, delegation, continuation, lint,
repair, and every z-Bot mutation boundary. Skills consume resolved roles and
do not construct paths. Conformance failures reject invalid mutations and
return structured model nudges. Destructive repair and configuration changes
require previewed, epoch-bound human/policy approval.

The accepted Phase 1 delta implements only generic template loading,
snapshot-on-create, lint, nudges, and context injection. Default artifact names
are data rather than required fields; repair, approval, transactional mutation,
and broad mutation-boundary enforcement remain possible follow-ons rather than
requirements of the first slice.

The default contract places the canonical document inside its concept folder:
`<concept>/<concept>.md`, alongside its `index.md`, repeatable `spec.md`, current
`plan.md`, tasks, and run history. It declares `src`, `data`, `reports`, and
`output` as operational resource areas. The wards root remains an OKF catalog
whose entries resolve through each ward's exported index role.

`ward-designer` and `ward_hygiene` are retired. Old wards are deleted or
rebuilt; no compatibility reader or migration is provided.

## Consequences

**Positive:**

- Users can change ward conventions without recompiling z-Bot.
- A ward has one inspectable, versioned source for every layout-sensitive
  participant.
- Spec and planning skills remain generic across ward shapes and domains.
- Recursive OKF validation, repair previews, and generated ownership become
  deterministic rather than prompt-dependent.
- Structured nudges give models a bounded correction path without silently
  accepting invalid state.

**Negative:**

- The runtime gains schema, resolver, activation, transaction, confinement,
  lint, repair, and context-injection machinery.
- Template edits can make new ward creation or a ward snapshot temporarily
  unusable until explicitly activated.
- Existing wards and their old state are intentionally not migrated.
- Snapshot isolation means template improvements do not reach existing wards
  automatically.
- Guarding shell, MCP, connector, Git, and direct filesystem mutation surfaces
  requires a broader enforcement boundary than guarding the `ward` tool alone.

**Neutral / to revisit:**

- A future RFC may add previewed template-to-snapshot adoption; it must not
  introduce implicit propagation.
- The Phase 2 mini-Obsidian UI will consume the same resolver and conformance
  results but is not part of this decision's implementation phase.
- Additional conformance profiles, renderers, placeholders, or configuration
  versions require explicit versioning and governance.

## Alternatives considered

- **Continue using prose, prompts, and skills:** rejected because distributed
  conventions already drift and cannot mechanically guard writes.
- **Compile one layout into Rust:** rejected because user customization would
  require product releases and skills would still need a parallel description.
- **Use a live vault-template reference for every ward:** rejected because a
  single edit could unexpectedly invalidate all existing wards.
- **Use layered inheritance or automatic template merging:** rejected because
  effective configuration and conflict resolution become harder to inspect;
  explicit adoption can be proposed separately.
- **Allow executable templates or plugins:** rejected because arbitrary
  computation expands the trust boundary and harms portability and
  reproducibility.
- **Keep `ward-designer` as the layout authority:** rejected because an LLM
  skill can explain policy but cannot provide deterministic conformance or
  filesystem admission.
- **Migrate old wards:** rejected because ambiguous legacy paths and content
  make automatic movement unsafe, and the accepted cutover deliberately
  permits deletion/rebuild.

## References

- [RFC-0016: Generic Ward Configuration and Layout Resolution](../rfc/0016-generic-ward-configuration-and-layout-resolution.md)
- [Open Knowledge Format v0.1](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
- [RFC-0016 layout-resolution spike](../rfc/0016-notes/layout-resolution-spike.md)
