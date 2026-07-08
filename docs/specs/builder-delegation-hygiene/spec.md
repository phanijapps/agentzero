# Spec: Builder Delegation Hygiene

- **Status:** Done
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** RFC-0005: Builder Delegation and Ward Context Hygiene

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make root-to-builder delegation explicit about the kind of work being delegated
so builder-agent neither burns tokens reading unrelated docs for self-contained
artifact tasks nor skips ward hygiene when root bypasses the planner. A direct
builder delegation must carry a runtime delegation mode, receive mode-specific
executor rules, and use builder/default-agent instructions that keep ward
doctrine, `memory-bank/`, and live default-agent templates current without
overwriting user customizations.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off
before proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Add an explicit internal delegation mode model with these modes:
  `DirectArtifact`, `WardHygiene`, `WardBackedBuild`, and `StepExecutor`.
- Expose the mode through `delegate_to_agent` as an optional parameter and
  carry it through `DelegationRequest`, `DelegationContext`, child execution
  setup, and executor initial state.
- Infer a conservative default mode when root omits the field:
  `StepExecutor` for step-spec tasks, `DirectArtifact` for exact-output
  self-contained artifact tasks, and `WardBackedBuild` otherwise.
- Make executor subagent rules conditional on delegation mode. No mode may keep
  the current unconditional "read AGENTS.md + memory-bank/core_docs.md first"
  rule.
- Keep `DirectArtifact` lean: create the requested output files first, verify
  them, and return/declare artifact paths. Do not hydrate or rewrite ward
  doctrine before writing.
- Require `WardHygiene` and `WardBackedBuild` to inspect the supplied
  `<ward_snapshot>` and relevant ward files before implementation.
- Require `WardHygiene` to fill missing or empty `AGENTS.md`,
  `memory-bank/ward.md`, `memory-bank/structure.md`, and
  `memory-bank/core_docs.md` using the existing ward-designer doctrine shape.
- Require `WardBackedBuild` to update ward memory only when the work changes
  reusable structure, conventions, or registered primitives.
- Update bundled `builder-agent` and `planner-agent` templates so they describe
  the four modes, use live default agent names, and stop hardcoding stale
  `solution-agent` / `writer-agent` routing.
- Add a guarded live default-agent refresh path for existing installs. It may
  update `AGENTS.md` only when normalized current content matches a known old
  bundled template/signature, and it must write a backup first.
- Treat 4000 characters as the preferred delegation task length rather than a
  hard rejection. Longer task strings are allowed, but the tool should warn
  agents to move long instructions into ward/spec files and reference paths.

### Ask first

- Adding modes beyond the four named in this spec.
- Reintroducing a hard delegation task length rejection or changing the
  preferred/recommended warning thresholds again.
- Making `DirectArtifact` update full `AGENTS.md` or full `memory-bank/*`
  content for every standalone file task.
- Overwriting any live agent file that does not match a known default-template
  signature.
- Changing the existing ward-as-agent full-tool policy from the subagent
  capability spec.
- Adding a new persisted database column for delegation mode instead of using
  existing execution metadata/checkpoint/context fields.

### Never do

- Never rely on prompt text alone to distinguish direct artifacts from
  ward-backed builds.
- Never make builder-agent read unrelated workspace/root docs for a
  `DirectArtifact` task that names exact outputs and does not ask for repo work.
- Never skip ward hygiene when the delegated mode is `WardHygiene`.
- Never rewrite non-empty ward doctrine during `WardHygiene`; only fill missing
  or empty files unless the user explicitly asks for a rewrite.
- Never overwrite user-customized live agent instructions during default-agent
  refresh.
- Never route planner output to agents that are not seeded or discoverable in
  the live/default agent inventory.

## Testing Strategy

- Delegation mode parsing and inference: **TDD**. The mode classifier is a
  small invariant surface and should be unit-tested before wiring.
- Runtime propagation: **TDD**. Tests should prove `delegate_to_agent` arguments
  become `DelegationRequest` mode, child executor state, and mode-specific
  subagent rules.
- Prompt/template alignment: **goal-based checks**. Grep or snapshot-style
  tests should prove templates mention the four modes and do not mention stale
  `solution-agent` / `writer-agent` routing.
- Ward hygiene behavior: **TDD plus goal-based checks**. Unit tests should prove
  mode-specific rules direct `WardHygiene` to fill only missing/empty files and
  `DirectArtifact` to skip pre-work doc hydration.
- Live-agent refresh safety: **TDD**. Tests should prove matching old templates
  refresh with backups while customized content is preserved.
- Workspace compatibility: **goal-based checks**. `cargo check --workspace`,
  focused gateway-execution tests, and `git diff --check` must pass or document
  unrelated pre-existing failures.

## Acceptance Criteria

- [x] `delegate_to_agent` accepts an optional delegation mode argument whose
  allowed values map to the four internal modes.
- [x] `DelegationRequest` and `DelegationContext` carry delegation mode through
  spawn and callback registration without changing persisted DB schema.
- [x] Delegation mode inference classifies step-spec tasks as `StepExecutor`,
  exact-output self-contained artifact tasks as `DirectArtifact`, explicit
  hygiene tasks as `WardHygiene`, and all other builder implementation work as
  `WardBackedBuild`.
- [x] Child executor initial state exposes the delegation mode so tools and
  future policies can inspect it.
- [x] Executor subagent rules are mode-specific and no longer contain an
  unconditional docs-first rule for all executor subagents.
- [x] `DirectArtifact` rules tell builder to write named outputs first, verify
  them, and return/declare artifact paths without reading unrelated docs.
- [x] `WardHygiene` rules tell builder to fill missing/empty ward doctrine and
  memory-bank files, preserve non-empty files, and report updated paths.
- [x] `delegate_to_agent` allows long task strings but warns above the
  preferred 4000-character range and recommends file/path references for
  unwieldy instructions.
- [x] `WardBackedBuild` rules tell builder to read supplied ward context before
  coding and update `memory-bank/core_docs.md` only for new reusable primitives
  or changed reusable structure.
- [x] Bundled `builder-agent` instructions describe the four modes and no longer
  force reading all ward docs for direct artifact tasks.
- [x] Bundled `planner-agent` instructions use live/default agent names and do
  not hardcode unavailable `solution-agent` or `writer-agent` assignments.
- [x] Existing live default agents can be refreshed safely when they match known
  old bundled signatures, with backups written before replacement.
- [x] Customized live agent instructions are not overwritten by the refresh
  path.
- [x] Focused tests cover direct-artifact, ward-hygiene, ward-backed-build, and
  step-executor mode behavior.

## Assumptions

- Technical: `DelegationRequest` has no delegation-mode field today; it carries
  `task`, `context`, `skills`, `complexity`, and `parallel`. (source:
  `gateway/gateway-execution/src/delegation/context.rs`)
- Technical: `delegate_to_agent` schema has no explicit mode parameter today,
  so root can only encode mode in task text/context. (source:
  `runtime/agent-runtime/src/tools/delegate.rs`)
- Technical: executor subagent rules currently force "enter ward, read
  AGENTS.md + memory-bank/core_docs.md" unconditionally. (source:
  `gateway/gateway-execution/src/invoke/setup.rs`)
- Technical: new wards are seeded with minimal `AGENTS.md` plus empty
  `memory-bank/{ward.md,structure.md,core_docs.md}`; agents are expected to
  curate content. (source: `runtime/agent-tools/src/tools/ward.rs`)
- Technical: default-agent seeding skips an agent folder if it already exists,
  so stale live default agents need a guarded refresh path separate from plain
  seeding. (source: `gateway/gateway-services/src/agents.rs`)
- Product: delegation modes are `direct_artifact`, `ward_hygiene`,
  `ward_backed_build`, and `step_executor`. (source: user-approved direction
  2026-06-02)
- Product: `direct_artifact` should declare or return artifacts but should not
  update full ward doctrine/memory-bank unless the task creates reusable
  primitives or explicitly asks for ward setup. (source: investigation accepted
  2026-06-02)
- Product: guarded live-agent refresh may update only default agents whose
  instructions match known stale bundled signatures; user-customized agent files
  must never be overwritten. (source: investigation accepted 2026-06-02)
- Process: active specs use Objective, Boundaries, Testing Strategy,
  Acceptance Criteria, Assumptions, and a linked plan. (source:
  `docs/specs/runtime-context-control/spec.md`)
- Process: RFC-0005 is the constraining proposal for this implementation slice;
  its body should be completed before implementation PR approval. (source:
  user-approved direction 2026-06-02)
