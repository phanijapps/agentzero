# Plan: OKF Ward Tool Capabilities

- **Spec:** [`spec.md`](spec.md)
- **Status:** Complete

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as implementation evidence changes.

## Approach

First remove the previous OKF coupling from every forbidden surface by
selectively restoring develop behavior while preserving unrelated branch work.
Then retain one narrow integration seam in root-orchestrator construction: load
the selected ward's template, append a bounded canonical projection after the
base system instruction, and place the identical packet in generic execution state.
Extend the existing Ward tool to consume that packet, and make the shipped
agent/spec/planning Markdown layout-neutral consumers of the injected template.

The riskiest part is the rollback in mixed files: several contain unrelated MCP,
recall, and provider work that must remain. Restoration is therefore hunk-based
and verified behaviorally against develop, never a whole-branch reset.

## Constraints

- No new dependency, database field, transport endpoint, UI change, middleware
  layer, or compatibility path.
- No fixed artifact roles or directories outside the editable template.
- Use existing executor state and tool registration mechanisms.
- Keep the packet in memory only and bind it to trusted session, root actor,
  and selected-ward state; never derive packet identity from model arguments.
- Preserve unrelated changes already present on the branch.
- Lint is invoked explicitly through the Ward tool only.

## Declined additions

- Tempted to retain automatic post-write linting; declining because lint
  middleware is explicitly outside this phase.
- Tempted to create a typed role registry; declining because the template must
  remain fluid and the Ward tool can resolve generic declarations directly.
- Tempted to propagate the template through every delegated execution;
  declining because only the root pre-orchestrator injection is authorized.
- Tempted to generate navigation, backlinks, or synchronized task status;
  declining because the accepted template declares no generic relationship or
  status semantics beyond explicit `operations.createConcept`.

## Construction tests

- Integration regression: an existing `financial-analysis` ward is presented to
  unchanged intent analysis and remains exactly `financial-analysis` through
  root-orchestrator bootstrap.
- Session regression: an invalid explicit Ward lint result is returned to the
  caller without changing execution/session terminal state.
- Scope audit: compare the finished OKF delta with develop and fail if OKF
  behavior remains in forbidden modules.

## Design (LLD)

### Design decisions

- Use a small state packet containing trusted session/root-actor/selected-ward
  identity, logical source, schema version, canonical bounded projection,
  digest, availability, and sanitized diagnostic. The prompt and Ward tool
  consume the same packet, preventing two competing authorities. Traces to AC
  2, 5, and 6.
- Keep validation and mutations inside the existing Ward tool. There is no
  lifecycle enforcement layer. Traces to AC 3–8 and 10–11.

### Interfaces & contracts

- Extend `WardTool::parameters_schema` and `WardTool::execute` in
  `runtime/agent-tools/src/tools/ward.rs` with `search`, `lint`, `dry_run`, and
  `create_concept`.
- Each new action uses a discriminated schema with
  `additionalProperties: false` and the limits stated in the spec.
- `search` accepts a text query plus optional exact frontmatter tag filters and
  returns bounded ward-relative results in lexical order.
- `dry_run` is a discriminated `create_concept` preview and returns only bounded
  operation types, relative paths, expected sizes, and digests without writing
  or returning generated/existing content.
- `create_concept` accepts only the bounded template-valid concept path defined
  by the spec and resolves all created paths from the template packet in state.

### State & control flow

```text
unchanged intent analysis
  -> exact ward selection
  -> load active ward template packet
  -> append packet after base root system instruction
  -> store identical packet in executor state
  -> start root orchestrator with Ward tool registered
  -> explicit Ward action reads and validates packet from state
```

No Ward result is routed through continuation or session middleware.

### Failure, edge cases & resilience

- A missing/invalid template produces an `unavailable` packet; orchestration and
  `list`, `info`, and `use` of existing wards continue, while `create`, `use` of
  a missing ward, and template-dependent actions fail with a bounded structured
  result.
- Ward mismatch or projection/digest mismatch is reported as stale state; the
  tool does not guess or reload an undeclared fallback. The packet is the
  immutable snapshot for that root execution.
- Existing ward confinement is reused. Creation resolves the nearest existing
  parent, rejects traversal/symlink/file-type/case-collision hazards, stages the
  complete new concept, revalidates, and publishes without clobbering.
- Dry-run and create-concept share resolution code so previews cannot disagree
  with writes.

## Tasks

### T0a: Session API and UI remediation hunks are restored to develop behavior

**Depends on:** none

**Touches:** rollback-only hunks in
`apps/ui/src/features/logs/useSessionTrace.ts`,
`apps/ui/src/features/logs/useSessionTrace.test.ts`,
`apps/ui/src/services/transport/types.ts`,
`services/api-logs/src/repository.rs`,
`services/api-logs/src/service.rs`, and
`services/api-logs/src/types.rs`.

**Tests:**

- Restore develop fixtures for session list/detail ordering, row identity, and
  UI trace rendering (AC 11).
- Prove no continuation-root collapsing, canonical-session aggregation, or
  OKF-driven client trace regrouping remains.
- Diff audit preserves unrelated API and UI feature hunks.

**Approach:**

- Use the superseded `session-observability-reconciliation` spec and the
  current-vs-develop diff to inventory only its API/UI aggregation and rendering
  hunks before editing; record a one-to-one hunk-to-behavior map.
- Prohibit whole-file restores and remove only inventoried hunks in one cleanup
  commit.

**Done when:** the six production files and named UI test have
develop-equivalent session observability behavior, with unrelated hunks intact.

### T0b: Execution-state and continuation remediation hunks are restored to develop behavior

**Depends on:** none

**Touches:** rollback-only hunks in
`services/execution-state/src/repository.rs`,
`services/execution-state/src/service.rs`,
`services/execution-state/src/types.rs`,
`gateway/gateway-execution/src/continuation.rs`,
`gateway/gateway-execution/src/lifecycle.rs`,
`gateway/gateway-execution/src/runner/continuation_watcher.rs`,
`gateway/gateway-execution/src/runner/core.rs`,
`gateway/gateway-execution/src/runner/execution_stream.rs`,
`gateway/gateway-execution/src/runner/session_invoker.rs`,
`gateway/gateway-execution/tests/continuation_watcher_tests.rs`, and
`gateway/gateway-execution/tests/lifecycle_tests.rs`.

**Tests:**

- Restore develop fixtures for execution/session status updates, continuation
  spawning, and startup behavior (AC 11).
- Prove no OKF-remediation root-status reconciliation, stale-running recovery,
  or child-to-root terminal aggregation remains.
- Diff audit preserves unrelated continuation, provider, MCP, and recall hunks.

**Approach:**

- Use the superseded `session-observability-reconciliation` and
  `ward-lint-continuation-recovery` specs plus current-vs-develop diffs to
  inventory only their status-reconciliation and continuation-recovery hunks;
  record a one-to-one hunk-to-behavior map.
- Prohibit whole-file restores and remove only inventoried hunks in one cleanup
  commit.

**Done when:** execution/continuation behavior and the two named tests are
develop-equivalent for the inventoried remediation, with unrelated hunks intact.

### T1: Previous OKF coupling is removed while develop behavior and unrelated branch work remain

**Depends on:** T0a, T0b

**Touches:** rollback-only hunks in
`gateway/gateway-execution/src/middleware/intent_analysis.rs`,
`gateway/gateway-execution/src/runner/invoke_bootstrap.rs`,
`gateway/gateway-execution/src/invoke/executor.rs`,
`runtime/agent-tools/src/tools/execution/{shell.rs,write_file.rs,edit_file.rs}`

**Tests:**

- Regression fixtures demonstrate exact existing-ward reuse and unchanged
  develop routing (AC 1 and 11).
- Diff audit separates and preserves non-OKF MCP/recall/provider hunks.

**Approach:**

- Record an exact pre-edit inventory of OKF-only hunks against develop, prohibit
  whole-file restores, and remove only those hunks in a cleanup commit.
- Remove Ward layout guards from the three ordinary file tools and automatic
  lint prompt/state handling from the rollback-only executor hunks. Lifecycle
  and continuation remain regression-test surfaces only.

**Done when:** forbidden surfaces have develop-equivalent OKF behavior, their
targeted tests pass, and unrelated branch features remain in the diff.

### T2: Root orchestrator receives one state-backed active template packet

**Depends on:** T1

**Touches:** `gateway/gateway-execution/src/invoke/{executor.rs,setup.rs}`, `gateway/gateway-services/src/ward_layout/**`, `gateway/gateway-services/src/paths.rs`, `gateway/templates/ward-conf.yaml`

**Tests:**

- Executor integration test asserts prompt order and byte-identical canonical
  projection/digest state, including delimiter breakout, instruction-like YAML,
  unknown/secret fields, aliases, complexity limits, and logical-source-only
  projection, with both the 64-KiB projection and 96-KiB post-encoding limits
  tested (AC 2).
- Invalid and missing templates produce a bounded bootstrap diagnostic without
  a session-terminal error or compiled-layout fallback (AC 5).

**Approach:**

- Reduce the existing layout service to bounded safe YAML loading, allowlisted
  canonical structural projection, and generic state packet creation needed by
  injection and the Ward tool.
- Inject only while constructing the root orchestrator, after intent has
  selected the exact ward.

**Done when:** root executor construction proves the single packet is present in
both prompt and state before orchestration begins.

### T3: Ward tool performs state-backed OKF operations safely

**Depends on:** T2

**Touches:** `runtime/agent-tools/src/tools/ward.rs`, `runtime/agent-tools/src/lib.rs`, `runtime/agent-tools/src/tools/mod.rs`, `gateway/gateway-execution/src/invoke/executor.rs`

**Tests:**

- TDD unit tests cover action schemas, tag/query search, structured lint,
  zero-write dry-run, template-directed concept creation, missing/stale state,
  action/output bounds, path traversal, prefixed/alternate paths, NULs,
  symlink escape, non-directory parents, case collisions, existing targets,
  and competing no-clobber creation (AC 3–8).
- Identity tests reject cross-session, cross-root-execution, delegated, and
  selected-ward mismatches before filesystem access (AC 7).
- Integration test confirms the executor registers the expanded Ward tool using
  the same state packet (AC 2–4 and 7).

**Approach:**

- Preserve existing `list`, `info`, and existing-ward `use` behavior; preserve
  `create`/missing-ward `use` semantics when a valid template packet is
  available and fail structurally otherwise.
- Add the four actions and factor only the shared resolution needed to guarantee
  dry-run/create parity.
- Resolve concept creation only through a repeatable template node explicitly
  annotated `operations.createConcept: true`; never infer concept semantics from
  a path, role name, or filename.
- Serialize the allowlisted projection as compact sorted-key JSON and compute
  lowercase SHA-256 over those exact bytes; use the common bounded result
  envelope from the spec for all four actions.

**Done when:** all Ward operations pass and no mutation can escape the selected
ward or bypass the state-held template.

### T4: Agent and skill Markdown follow the injected template generically

**Depends on:** T2

**Touches:** `gateway/templates/agents/*.md`, `gateway/templates/skills/{plan-composer,spec-builder}/**`

**Tests:**

- Prompt snapshot tests use two incompatible ward templates, including one
  without specs, plans, tasks, or conventional resource directories, and prove
  the exact layout-neutral instructions supplied to the model (AC 9).
- Absence checks reject fixed ward paths and mandatory artifact names (AC
  9–11).

**Approach:**

- Restore generic develop guidance first, then add only instructions to consult
  the injected active template before choosing paths.
- Keep planning/spec behavior useful when the active template declares no
  corresponding artifact.

**Done when:** the same agents and skills correctly follow both incompatible
templates without embedded layout knowledge.

### T5: Integrated OKF scope and regression gates are green

**Depends on:** T3, T4

**Touches:** `gateway/gateway-execution/tests/**`, `runtime/agent-tools/src/tools/ward.rs`, `docs/specs/okf-ward-tool-capabilities/**`

**Tests:**

- Run targeted Ward, executor, intent, continuation, and session tests.
- Run `cargo fmt --all`, affected-crate Clippy with warnings denied, affected
  tests, and `cargo check --workspace` (AC 1–11).
- Run a final diff audit proving no OKF changes exist outside the authorized
  surfaces except rollback removals (AC 11).

**Approach:**

- Exercise the complete intent-to-root-orchestrator-to-Ward-tool flow.
- Remove any residual abstraction or enforcement not required by the spec.

**Done when:** all mechanical gates pass and adversarial/security review returns
clean for the finished scoped diff.

### T6: Template-derived Ward artifacts remain conformant and model-maintained

**Depends on:** T3, T4

**Touches:** `gateway/gateway-execution/src/invoke/ward_layout_adapter.rs`,
`gateway/gateway-services/src/ward_layout/create.rs`,
`gateway/templates/{ward-agent.md,skills/plan-composer/SKILL.md}`, and focused
tests beside those implementations.

**Tests:**

- **TDD:** `dry_run` and `create_concept` produce the same required
  template-derived files, resolve `{name}`, and scaffold every declared
  `okf-v0.1` file with the metadata accepted by the generic linter.
- **TDD:** fresh-Ward and concept scaffolds use the same minimal OKF metadata
  format contract and are independently lint-valid, while allowing their
  template-derived metadata values to differ.
- **TDD:** an incompatible template with opaque role IDs and no navigation or
  task artifacts remains fully template-derived and receives the same generic
  guidance.
- **Regression:** existing traversal, symlink-escape, case-collision,
  descriptor-relative no-follow, and no-clobber publication cases remain green.
- **Goal-based:** generic Ward/planning guidance explicitly directs concept
  creation through the Ward tool and permits a conformance claim only after a
  structured lint result has `ok=true` and `data.valid=true`; stale,
  unavailable, and invalid results are reported as unresolved without becoming
  terminal session failures.
- **Goal-based:** a diff allowlist confirms T6 production changes remain within
  the four declared paths and do not touch intent, orchestration, lifecycle,
  APIs/UI, ordinary file tools, or automatic mutation paths.

**Approach:**

- Correct the existing scaffold formatting in place; do not add a renderer,
  typed role registry, middleware, or another mutation path.
- Do not infer or maintain links, backlinks, task summaries, or completion
  status because the template declares no generic relationship or status-sync
  operation.
- Preserve intent analysis, orchestration, delegation, continuation, session
  persistence/API/UI, and ordinary file-tool behavior byte-for-byte.

**Done when:** explicit Ward concept creation produces a lintable required tree,
generic instructions explicitly require that action and a valid lint result,
and focused plus workspace gates pass without touching forbidden surfaces.

## Rollout

- Land the removal of superseded OKF coupling as an independently reviewable
  cleanup commit, followed by separate injection, Ward capability, and Markdown
  commits.
- Clean break with no old-ward migration or backward-compatibility behavior.
- No database cutover, API migration, feature flag, or UI rollout.
- Rollback reverts only the new injection/Ward-capability/Markdown commits; the
  reviewed cleanup of superseded coupling and unrelated MCP/recall/provider
  changes remain. User-owned ward data is not modified by installation/startup.

## Risks

- Mixed OKF and unrelated changes in large execution files make broad restores
  dangerous; hunk-level comparison and targeted regression tests are mandatory.
- A user-editable template is untrusted input; prompt delimiting and filesystem
  confinement must prevent instruction injection and path escape.
- Prompt and state drift could create two authorities; byte-identical packet and
  digest assertions are the gate.

## Changelog

- 2026-07-21: Completed T6 after scaffold, confinement, template-guidance,
  regression, and workspace gates passed.

- 2026-07-20: Replaced the broad OKF implementation plan with the narrow
  template-injection and Ward-tool-only design after user correction.
- 2026-07-20: Implemented root-only canonical template injection and immutable
  in-memory state, the four explicit Ward actions, fluid concept resolution,
  generic agent/plan/spec guidance, and removal of superseded OKF lifecycle,
  session, API/UI, ordinary-tool, and ward-triggered planner coupling.
- 2026-07-20: Kept security proportional to this phase: bounded template and
  search/lint reads, prompt-data isolation, Linux descriptor-relative ward
  traversal/publication, no-clobber writes, and fail-closed concept creation on
  platforms without the required primitive.
- 2026-07-21: Added T6 after live-Ward verification found direct filesystem
  concept creation, incompatible OKF scaffold metadata, stale navigation/task
  summaries, and dismissed stale lint results.
