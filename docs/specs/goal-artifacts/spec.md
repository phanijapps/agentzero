# Spec: Goal Artifacts

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** [`contracts/openapi/goal-artifacts.yaml`](../../../contracts/openapi/goal-artifacts.yaml)
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make Quick Chat show only durable, explicitly designated goal artifacts under a
clear **Deliverables** section. An agent marks a file as a goal artifact only
when it is a useful final output of the user's goal; its extension does not
decide. Consequently, a requested Python script or JSON export can be shown,
while an internal plan, specification, scratch file, or intermediate source
file remains hidden. Existing artifacts without this designation remain hidden.

## Boundaries

### Always do

- Persist an explicit `is_goal_artifact` boolean with each artifact declaration;
  its default is `false` for omitted declarations and legacy database rows.
- Carry the designation unchanged from the `respond` tool declaration through
  the execution event, artifact store, artifact-list endpoint, TypeScript
  transport, and Quick Chat filter.
- Tell agents to set the designation only for a final deliverable the user
  asked for or needs to use; do not infer it from filename, extension, path,
  label, or file contents.
- Limit the first goal-artifact presentation change to Quick Chat. Keep
  Research's artifact strip unfiltered and preserve the shared preview UX,
  while tightening the shared content request with session binding, path
  confinement, size limits, and script-disabled active-content previews.
- Keep all artifacts persisted and listable through the existing endpoint;
  Quick Chat requests its bounded, goal-only manifest while Research remains
  unfiltered.
- Render Research artifact chips only from persisted artifact-manifest records.
  A `respond` declaration's path is not an artifact ID and must never become a
  clickable fallback chip when persistence did not produce a manifest row.
- Canonicalize every declared artifact before persistence and accept only a
  regular, non-symlink file inside the active ward's vault root. A model
  declaration is untrusted data: it cannot authorize an absolute host path,
  traversal, a current-working-directory path, or a file outside that root.
- Revalidate the stored file's canonical ward-root confinement when serving its
  content, and require the existing content route to carry the matching session
  ID. `is_goal_artifact` changes presentation only; it never grants file-read
  authority.
- At serve time, open a candidate exactly once with no-follow semantics,
  validate that opened handle as a bounded regular file under the canonical
  ward root, then read from that same handle. On a platform without a safe
  no-follow open, reject the request rather than falling back to path-check
  then path-read behavior.
- Bound untrusted declaration data: at most 8 declarations per response, 24
  goal artifacts per session, a 1,024-character relative path, a
  160-character label, and a 5 MiB artifact size. The 24-goal-artifact limit
  is enforced atomically by the repository, and Quick Chat requests at most 24
  designated rows. Rejected excess or invalid declarations are skipped and
  logged without failing the completed response.
- Treat every artifact preview as untrusted data. HTML and SVG previews run in a
  sandbox without scripts, and the shared content endpoint rejects a file that
  exceeds 5 MiB at either persistence or serve time with a safe
  too-large-to-preview result. Direct HTML/HTM/SVG content responses are served as
  non-executable attachments with `X-Content-Type-Options: nosniff`; the
  slide-out may fetch their text into its script-disabled sandbox. Office
  documents (`.docx`, `.xlsx`, `.pptx`) are download-only: browser-side
  decompression is not used for untrusted artifact previews.

### Ask first

- Showing goal artifacts in Research or any other surface.
- Reclassifying existing rows, changing their retention, or deleting hidden
  artifacts.
- Introducing model-based value scoring or automatic file classification.
- Adding new artifact roles beyond this binary deliverable signal.
- Raising the declaration, session, path, or label budgets.
- Treating a caller-provided session ID as multi-user authorization. Any LAN
  or multi-user gateway exposure needs an authenticated session-access policy.

### Never do

- Never use a filename or file-extension blocklist/allowlist to decide whether
  an artifact has goal value.
- Never expose a working artifact solely because its type is markdown, Python,
  JSON, or another familiar file format.
- Never add a dependency, persistence table, service boundary, or new endpoint
  for this feature.
- Never return an artifact's resolved server path to a browser or trust a model
  declaration as permission to read a filesystem path.
- Never execute an artifact's HTML, SVG, script, or other file content as an
  instruction or as browser code.
- Never validate a filesystem pathname and then read that pathname in a later
  operation; the same no-follow opened handle must be validated and read.

## Testing Strategy

- **TDD — declaration and persistence:** Rust tests will prove omitted and
  explicit false declarations persist as non-goal artifacts, while an explicit
  true declaration survives the event-to-database-to-response path. Tests will
  prove path confinement, non-symlink checks, declaration/session budgets, and
  safe v22→v23 migration defaults.
- **TDD — Quick Chat presentation:** focused Vitest tests will prove only
  `isGoalArtifact: true` cards render, regardless of extension; a requested
  `.py` or `.json` marked true renders, while a `.md` plan marked false does
  not. A hook test proves the bounded goal-only query is used instead of an
  unbounded session manifest; shared preview tests prove active-content scripts
  are disabled, direct HTML/SVG content is non-executable, and oversized content gets
  a safe unavailable state with no broken preview or download action.
- **Visual/manual QA:** complete one Quick Chat turn that creates a deliverable
  and an internal working file; only the deliverable appears in Deliverables
  and opens in the existing preview.
- **Goal-based checks:** validate the OpenAPI contract, run the focused Rust and UI
  tests, UI lint/build, Rust formatting/clippy, and affected workspace check.

## Acceptance Criteria

- [ ] Given an artifact declaration without `is_goal_artifact`, when it is
  persisted, the stored and returned value is `false`.
- [ ] Given an artifact declaration explicitly marked `is_goal_artifact: true`,
  when the producing Quick Chat turn completes, its card appears in **Deliverables**
  and opens with the existing artifact preview.
- [ ] Given an artifact marked false, when its producing Quick Chat turn completes,
  it is not shown in **Deliverables**, regardless of its filename, label, or
  extension.
- [ ] A requested `.py`, `.json`, plan, or other file format marked true is
  shown; the same format marked false is hidden.
- [ ] Existing artifact rows and clients that omit the new field are treated as
  non-goal artifacts without failing session hydration or artifact listing.
- [ ] A declaration for `/etc/hosts`, `../../outside`, a symlink escaping the
  active ward, or a session without an active ward is not persisted or
  previewable; an artifact ID from a different session cannot be served through
  the current session's content request.
- [ ] A response with more than 8 declarations, a session with 24 designated
  goal artifacts, or overlong label/path metadata cannot create more
  deliverables, including concurrent responses; Quick Chat communicates its
  24-deliverable session limit.
- [ ] Artifact list responses omit resolved `filePath` metadata while still
  providing all preview-safe display metadata and `isGoalArtifact`.
- [ ] A 5 MiB-plus artifact is never persisted or previewed; a pre-existing
  oversized row receives a safe too-large-to-preview response, and an HTML or
  SVG artifact preview cannot execute script or load as executable content through a
  direct content URL.
- [ ] Artifact serving validates and reads the same no-follow file handle, so
  a symlink/path replacement after validation cannot redirect the content read.
- [ ] Research remains unfiltered and its previews continue to work through
  the shared session-bound, confinement-checked content request.
- [x] Given a `respond` declaration whose path has no persisted artifact
  manifest row, Research does not render a broken clickable artifact chip.
- [ ] The goal-artifact manifest contract, focused regression tests, manual
  Quick Chat journey, and stated quality gates pass.

## Assumptions

- Technical: Quick Chat currently fetches every artifact for its reserved
  session and renders all of them in one card list (source:
  `apps/ui/src/features/chat-v2/useQuickChat.ts`,
  `apps/ui/src/features/chat-v2/QuickChat.tsx`).
- Technical: the existing `ArtifactDeclaration` and persisted `Artifact` have
  no semantic visibility field; the SQLite runtime schema has a versioned
  migration path (source: `runtime/agent-primitives/src/event.rs`,
  `services/execution-state/src/types.rs`,
  `stores/zbot-runtime-sqlite/src/schema.rs`).
- Technical: the existing list endpoint transports artifact metadata through
  `ArtifactResponse` and the TypeScript `Artifact` shape (source:
  `gateway/src/http/artifacts.rs`,
  `apps/ui/src/services/transport/types.ts`).
- Product: only Quick Chat should surface designated goal artifacts; Research
  remains unfiltered, apart from shared preview safety hardening (source: user
  confirmation 2026-07-14; security review 2026-07-14).
- Product: deliverable value is explicit agent designation, not a filename,
  extension, or automatic classifier; legacy unclassified artifacts remain
  hidden (source: user confirmation 2026-07-14).
- Process: this mixed UI/data/interface change uses an OpenAPI contract; no
  matching contract-authoring skill is installed, so the contract is authored
  directly (source: `docs/CONVENTIONS.md`, available skills roster).
- Security: the current gateway is a local single-owner application; the
  required `session_id` content parameter is a correlation/ownership binding,
  not authenticated multi-user authorization (source: current local deployment
  model in `AGENTS.md`; security review 2026-07-14).
- Deployment: required content-route session binding is a coordinated UI and
  gateway release; a stale UI is not supported against the updated gateway
  (source: spec decision after review 2026-07-14).
