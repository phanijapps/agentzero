# Spec: Ward Agent Doctrine Template

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** user confirmations 2026-07-20 and 2026-07-21
- **Brief:** none
- **Contract:** none
- **Shape:** integration

> **Mode:** full (filesystem and model-prompt trust boundaries).
>
> **Spec contract:** this document defines what "done" means. The implementing
> change must match this spec, or update it before implementation continues.

## Objective

Move the initial Ward-agent doctrine out of Rust and into a bounded,
user-editable Markdown template at `config/templates/ward-agent.md`. New wards
render that template into their root `AGENTS.md`; existing wards retain their
own doctrine unchanged. Ward-agent synthesis reads only a complete, safe,
bounded doctrine and otherwise continues with its platform-owned identity plus
a nonterminal diagnostic.

## Boundaries

### Always do

- Seed the bundled Ward-agent template with create-new semantics and never
  overwrite a user-edited vault template.
- Replace only the literal `{{ward_id}}` and `{{display_name}}` placeholders.
- Enforce a 12 KiB byte ceiling on both source template and rendered doctrine.
- Load the persona template lazily: a layout with no required literal root
  `AGENTS.md` must not depend on the persona template's validity.
- Require templates and loaded doctrine to be regular, single-link, valid UTF-8
  files and reject hard links or special-file inputs. Linux traversal is
  descriptor-relative and no-follow from a verified vault root. Other targets
  use canonical pre/post confinement and opened-file identity checks; template
  bytes are not written until the created target is revalidated.
- Preserve existing `AGENTS.md` bytes and never migrate or rewrite an existing ward.
- Keep the template layout-neutral; the active Ward layout remains the only
  authority for filesystem shape.
- Preserve current synthesized instructions byte-for-byte for normal, in-budget
  doctrine after the existing trim behavior.

### Ask first

- Adding model-generated personas, domain inference, or a doctrine approval workflow.
- Adding budget fields to settings or changing the public Ward tool interface.
- Migrating or rewriting any existing ward doctrine.

### Never do

- Change intent analysis, Ward recommendation/reuse behavior, routing,
  delegation, runner continuation, lifecycle, session state, APIs, WebSockets,
  UI, or the Ward tool JSON schema.
- Partially inject an oversized, invalid, or unsafe `AGENTS.md`.
- Treat Ward doctrine as higher priority than platform-owned system identity.
- Introduce a template engine, model tokenizer, database field, compatibility
  path, or new dependency.

## Testing Strategy

- **TDD (AC1-AC4):** focused `gateway-services` unit tests cover seeding,
  preservation, placeholder rendering, layout neutrality, and unsafe/oversized
  template rejection, including a layout that does not declare `AGENTS.md`.
- **TDD (AC5-AC7):** focused `gateway-execution` unit tests cover exact normal
  doctrine loading and complete fallback for oversized, non-UTF-8, symlink,
  hard-link, special-file, and missing doctrine.
- **Goal-based (AC8):** compare pre/post hashes of protected orchestration,
  Ward-tool, API/service, and UI files; inspect the diff for schema changes.
- **Mechanical:** `cargo fmt --all --check`, focused crate tests,
  `cargo check --workspace`, and `cargo clippy --all-targets -- -D warnings`.

Stub tally: 7 TDD acceptance criteria covered; 1 goal-based criterion has no stub.

## Acceptance Criteria

- [x] AC1: Startup seeds `config/templates/ward-agent.md` from the bundled
  template when absent and preserves an existing regular user-edited file.
- [x] AC2: A new Ward whose active layout declares root `AGENTS.md` renders the
  editable template with its canonical Ward ID and readable display name.
- [x] AC3: The template source and rendered doctrine are each limited to 12 KiB,
  valid UTF-8, regular single-link files; invalid inputs fail Ward creation
  before publication. A layout without a required literal root `AGENTS.md`
  remains creatable even when the persona template is invalid.
- [x] AC4: Existing ward directories and existing `AGENTS.md` files are never
  modified by seeding or Ward creation.
- [x] AC5: An in-budget regular `AGENTS.md` is loaded in full and reaches the
  existing delimited Ward-doctrine prompt section unchanged except for current
  leading/trailing trim semantics.
- [x] AC6: Oversized, non-UTF-8, symlinked, hard-linked, or special-file doctrine is not
  partially injected; synthesis continues with the platform identity and a
  fixed diagnostic of at most 160 ASCII bytes. The diagnostic contains only a
  stable `WARD_DOCTRINE_UNAVAILABLE:<reason>` code and generic guidance; it
  never contains source bytes, filesystem paths, or raw OS error text. A
  structured warning records the code and Ward name only, never a path or payload.
- [x] AC7: A missing `AGENTS.md` retains the current safe behavior: synthesis
  continues without a Ward-doctrine section and without a failure diagnostic.
- [x] AC8: Protected intent-analysis, routing, delegation, runner, lifecycle,
  state/log API, Ward-tool schema, and UI files are byte-identical to the
  pre-change baseline.

## Assumptions

- Technical: `create_ward_from_template` is the active template-backed Ward
  creation primitive and already publishes atomically on Linux (source:
  repository inspection and codegraph 2026-07-21).
- Technical: this feature performs all template loading and rendering before
  calling the existing platform-specific publication primitive; it does not
  redefine that primitive's cross-platform guarantees (source: design review
  2026-07-21).
- Technical: Ward-agent doctrine is loaded only for synthesized `ward:<name>`
  agents in `gateway/gateway-execution/src/invoke/setup.rs` (source: repository
  inspection and codegraph 2026-07-21).
- Product: the user wants a generic hackable template, not inferred domain
  personas or a rigid Rust schema (source: user confirmation 2026-07-20).
- Product: the 12 KiB ceiling is a deterministic prompt-size guard, not a
  model-specific token budget (source: implementation assumption 2026-07-21).
- Process: all pre-existing worktree modifications are user-owned and must be
  preserved (source: worktree inspection 2026-07-21).
