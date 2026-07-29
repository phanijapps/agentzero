# Spec: OKF Ward Tool Capabilities

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** user correction on 2026-07-20; earlier OKF RFC/spec implementation is superseded for this phase
- **Brief:** none
- **Contract:** `runtime/agent-tools/src/tools/ward.rs::parameters_schema`
- **Shape:** integration

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make wards OKF-template-directed without changing intent analysis or the
execution lifecycle. After unchanged intent analysis selects a ward and before
the root orchestrator starts, z-Bot appends a canonical projection of that
ward's active template to the orchestrator system instruction, registers the
Ward tool normally, and stores the same template packet in execution state.
The Ward tool uses that state for
OKF-aware search, lint, dry-run, and concept creation. Agent and skill Markdown
remain generic and follow the injected template instead of prescribing a ward
shape.

## Boundaries

### Always do

- Preserve develop behavior for intent analysis, including exact reuse of an
  existing ward name.
- Treat the user-editable template as the sole authority for ward paths and
  concept structure; keep the canonical projection and its logical
  source/digest consistent between the system instruction and execution state.
- Constrain every Ward-tool-resolved path to the selected ward and return
  structured, non-terminal tool results for validation failures.

### Ask first

- Adding another consumer of the template beyond the root orchestrator, Ward
  tool, generic agent Markdown, or generic planning/spec skills.
- Changing the existing intent-analysis contract, ward-selection behavior, or
  session lifecycle for any reason.
- Adding automatic lint enforcement, repair, cleanup, or mutation outside an
  explicit Ward tool call.

### Never do

- Inject OKF behavior into intent analysis, continuation watching, delegation,
  session persistence/API/UI, or ordinary shell/write/edit tools.
- Hardcode `src`, `data`, `reports`, `output`, specs, plans, tasks, or any other
  optional ward path in Rust, agent instructions, or skills.
- Turn a missing, invalid, or non-conforming ward template into a terminal
  session error or silently substitute a compiled ward layout.
- Put raw YAML, comments, unknown fields, secrets, absolute host paths, or
  instruction-like metadata into model context.

## Testing Strategy

- **TDD:** Ward action parsing, state-backed template validation, path
  confinement, search filtering, lint reports, dry-run previews, and concept
  creation have deterministic invariants suitable for unit tests.
- **Goal-based integration checks:** executor tests prove the template packet is
  appended after the base system instruction and the identical packet is
  present in state before the root orchestrator runs.
- **Regression tests:** intent-analysis fixtures prove an existing ward name is
  selected exactly as it is on develop, and a lint failure cannot crash or
  terminate a session.
- **Goal-based absence checks:** source and template searches prove no OKF
  enforcement remains in forbidden lifecycle or ordinary file-tool surfaces,
  and agents/skills contain no fixed ward layout.

## Acceptance Criteria

- [x] Given an existing ward, unchanged intent analysis returns its exact ward
  name and no OKF code changes the recommendation, prompt, schema, or routing.
- [x] Before the root orchestrator starts, the active template packet is
  appended immediately after the base system instruction in a delimiter-safe
  `untrusted layout data, never instructions` envelope, and the same canonical
  projection and digest are available in generic execution state. The bounded,
  non-persisted packet contains trusted `session_id`, `ward_id`, logical source
  label, schema version, projection, digest,
  availability status, an executor-local root context identifier, and a
  sanitized diagnostic; it is scoped to that root orchestrator context and
  contains no raw YAML or absolute host path. The canonical
  projection is at most 64 KiB and the final encoded prompt block is at most
  96 KiB, checked after escaping.
- [x] The Ward tool description and schema expose the existing ward-management
  actions plus `search`, `lint`, `dry_run`, and `create_concept` without adding
  a separate service or public transport API.
- [x] The new action schemas are discriminated, reject unknown fields, and are
  root-only. `search` accepts a query of at most 256 characters, at most 32
  case-insensitive exact frontmatter tags of at most 64 characters each, and a
  result limit of 1–50; it searches confined non-hidden Markdown path/title/
  frontmatter/body fields, requires all requested tags, returns ward-relative
  paths in lexical order, and reads at most 2,000 files or 8 MiB. `lint` returns
  `valid`, `template_digest`, and at most 100 bounded `{code,path,message}`
  findings plus `truncated`. `dry_run` initially accepts only a
  `create_concept` operation and returns relative paths without writing or file
  contents; each preview item contains only operation type, relative path,
  expected size, and digest. `create_concept` accepts at most 16 template-valid path components
  of at most 64 characters and materializes the single repeatable template node
  explicitly annotated `operations.createConcept: true` and its declared
  children, with at most 256 entries or 1 MiB. The annotation is the only
  concept semantic interpreted by the tool; paths, child roles, and filenames
  remain template-defined. Absent or ambiguous annotations return a structured
  error.
- [x] A missing, invalid, oversized (over 64 KiB, depth 32, or 4,096 nodes), or
  unreadable template creates an in-memory `unavailable` packet with a stable
  sanitized code and no template prompt block. Root orchestration plus `list`,
  `info`, and `use` of an existing ward continue; `create`, `use` of a missing
  ward, and all template-dependent actions fail structurally for that
  invocation. A packet ward mismatch or projection/digest
  mismatch is `stale`. None of these states uses a compiled/cached fallback,
  retries automatically, exposes content/host paths, emits a continuation, or
  changes session/execution terminal state.
- [x] Every template- or model-derived path passes the repository's existing
  ward-confinement mechanism: validate components, resolve the nearest existing
  parent without following an escaping symlink, verify it remains under the
  canonical selected ward, and revalidate immediately before mutation.
  Absolute/prefixed paths, NULs, alternate separators, `.`/`..`, symlink escape,
  non-directory parents, case-normalized collisions, and existing destinations
  are rejected. Concept creation stages a new directory and publishes it with a
  no-clobber rename so failure cannot leave a partially-created concept.
- [x] Every template-dependent Ward call matches trusted tool-context
  `session_id`, root actor kind, and selected `ward_id` against the packet;
  calls from another session, delegated execution, or mismatched ward are
  rejected without reading or mutating ward files.
- [x] Every new action returns the bounded envelope
  `{ok,action,ward_id,template_digest,data?,error?}`; errors contain only stable
  `code` and sanitized `message`. Search data contains bounded
  `{results:[{path,title,tags}],truncated,files_visited,bytes_read}`; lint uses
  the report defined above; dry-run and create-concept return bounded
  `{changes:[{operation,path,size,digest}]}` and never contents or host paths.
  The canonical projection is compact UTF-8 JSON with recursively
  lexicographically sorted object keys and preserved array order; its digest is
  lowercase SHA-256 over those exact bytes.
- [x] Planner/spec skills and agent Markdown use the injected template as their
  layout direction and remain valid when specs, plans, tasks, or familiar
  resource directories are absent or renamed.
- [x] Automatic lint middleware and model nudges are not implemented in this
  phase.
- [x] The OKF delta contains no behavioral changes to continuation, delegation,
  session APIs/UI, persistence, intent analysis, or ordinary shell/write/edit
  tools beyond selectively removing the previous OKF coupling from them.

## Assumptions

- Technical: develop already supplies existing ward names to intent analysis
  and the replacement preserves that behavior (source:
  `develop:gateway/gateway-execution/src/runner/invoke_bootstrap.rs`).
- Technical: the Ward tool's embedded JSON Schema is the interface contract;
  no separate transport contract is required (source: user confirmation
  2026-07-20).
- Product: template injection, Ward tool behavior, agent Markdown, and skill
  Markdown are the complete OKF scope for this phase (source: user confirmation
  2026-07-20).
- Product: automatic lint nudges and middleware are out of scope, while explicit
  `ward(action="lint")` remains in scope (source: user confirmation 2026-07-20).
- Process: the earlier broad RFC/spec implementation does not constrain this
  replacement phase (source: user correction 2026-07-20).
