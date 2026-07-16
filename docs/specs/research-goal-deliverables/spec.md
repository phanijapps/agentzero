# Spec: Research Goal Deliverables

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none
- **Shape:** ui

> **Mode:** light (no risk trigger fired).

## Objective

Make the Research attachment strip show only durable artifacts explicitly
declared as user-facing goal deliverables. Internal plans, Python scripts,
JSON, and other working files remain available in the ward explorer, while a
requested final file of any type remains visible when marked as a goal
artifact.

## Boundaries

### Always do

- Request the existing bounded goal-artifact manifest for every Research
  snapshot and cache-miss lookup.
- Defensively discard artifact rows not explicitly marked `isGoalArtifact`.
- Keep the existing artifact preview and ward-file explorer behavior unchanged.

### Ask first

- Changing the goal-artifact declaration, persistence, or API contract.
- Adding a second classification mechanism based on filenames, extensions, or
  content inspection.

### Never do

- Delete or mutate existing artifact rows or ward files.
- Infer deliverable value from a `.py`, `.json`, `.md`, or any other file type.
- Add a dependency, module boundary, or new top-level directory.

## Testing Strategy

- **TDD:** Research snapshot construction and its cache-miss artifact lookup
  verify that a mixed manifest exposes only explicitly designated artifacts.
- **Goal-based check:** lint, TypeScript production build, and the focused
  Research test area confirm the transport calls and UI compile together.
- **Visual/manual QA:** complete a Research task that writes one working file
  and one declared deliverable; only the deliverable chip appears before and
  after a reload.

## Acceptance Criteria

- [x] Given a Research session whose artifact manifest contains both designated
  and undesignated rows, when its snapshot loads or refreshes, the attachment
  strip shows only the explicitly designated rows.
- [x] Given a requested final `.py`, `.json`, or other file marked
  `isGoalArtifact: true`, it remains available from the Research attachment
  strip; the same file type marked false is hidden.
- [x] Research requests the existing bounded goal-artifact manifest and also
  filters returned rows locally, so an incorrect or older server response
  cannot expose working files.
- [x] The ward explorer and artifact-preview behavior remain available for the
  deliverable records without changing or deleting any session files.

## Assumptions

- Technical: the established artifact transport already supports a bounded
  `goalArtifactsOnly` query and supplies `isGoalArtifact` per manifest row
  (source: `apps/ui/src/services/transport/types.ts`,
  `apps/ui/src/services/transport/http.ts`).
- Technical: Research currently maps every artifact manifest row into its
  visible strip and refresh cache (source:
  `apps/ui/src/features/research-v2/session-snapshot.ts`,
  `apps/ui/src/features/research-v2/useResearchSession.ts`).
- Product: Research attachments should show only explicit goal deliverables;
  legacy/unmarked rows are hidden but remain in the ward explorer (source: user
  confirmation 2026-07-15).
- Process: this focused UI update must keep its paired spec and criteria current
  with implementation (source: `docs/CONVENTIONS.md` § Spec metadata contract).
