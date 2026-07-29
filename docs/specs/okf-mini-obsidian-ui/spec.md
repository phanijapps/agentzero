# Spec: OKF Mini-Obsidian UI

- **Status:** Archived
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [RFC-0015](../../rfc/0015-okf-aligned-ward-layout-and-capabilities.md); [OKF Ward Foundation](../okf-ward-foundation/spec.md)
- **Brief:** none
- **Contract:** [`contracts/openapi/okf-wards.yaml`](../../../contracts/openapi/okf-wards.yaml) (consumer)
- **Shape:** ui

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Evolve the retained Phase 1 OKF browser into a focused “mini Obsidian” for wards:
users can navigate concepts and links, edit validated Markdown, inspect graph
relationships, filter knowledge, and maintain a concept’s current spec, plan,
tasks, and archived runs without leaving z-Bot.

This UI consumes the foundation API and filesystem semantics. It does not invent
a second content model, expose arbitrary host filesystem editing, or recreate
the retired dedicated wiki.

## Experience

The primary workspace uses a three-pane layout: a ward/concept tree, a Markdown
editor or preview, and a contextual inspector. The inspector switches among
metadata/validation, backlinks, graph neighborhood, and refinement state.
Search and structured filters remain globally available. Selecting a concept
file and its same-stem companion presents one conceptual object, with children
and refinement artifacts grouped beneath it.

Editing is source-oriented Markdown with preview; Phase 2 need not provide a
WYSIWYG editor. Save uses the loaded ETag, shows validation errors inline, and
offers an explicit reload/diff path after a conflict. The UI never silently
overwrites a newer version.

## Boundaries

### Always do

- Use `contracts/openapi/okf-wards.yaml`; display root-relative paths only.
- Preserve the canonical concept/companion distinction while presenting them as
  a single navigable concept.
- Support create/edit/rename for valid concept documents, metadata/tags, child
  concepts, and refinement artifacts through server-owned operations.
- Show unsaved state, save progress, validation diagnostics, stale-ETag
  conflicts, and reindex status accessibly.
- Provide backlinks and a bounded local graph neighborhood; graph nodes navigate
  to concepts and distinguish unresolved links.
- Provide dedicated spec/plan/task views inside the selected concept companion,
  including task status updates and read-only archived-run browsing.
- Confirm destructive delete/rename operations with their affected relative
  paths and link impacts.
- Bind rename/delete confirmation to the server's short-lived impact token and
  abort on any path, companion-tree, or inbound-link change. A confirmed concept
  deletion atomically removes the concept and complete same-stem companion,
  including children, active refinement, and archived runs physically beneath
  that companion; the UI lists all of them before confirmation. Linked resource
  targets and ward-level `src/`, `data/`, `reports/`, `output/`, and
  `references/` content are retained. Archived files remain individually
  immutable and are removable only through this ancestor-level destructive act.
- Follow `apps/ui/ARCHITECTURE.md`, semantic CSS classes, design tokens, keyboard
  access, and responsive degradation.

### Ask first

- Adding collaborative/multi-user editing, real-time remote sync, plugins, or a
  general-purpose filesystem editor.
- Adding WYSIWYG/blocks, a custom Markdown dialect, or graph-wide bulk mutation.
- Changing API or canonical OKF semantics owned by the foundation spec.

### Never do

- Reintroduce wiki routes, wiki DTO names, promotion flows, or numbered wiki
  templates.
- Write files directly from the browser or accept absolute paths.
- Autosave over validation errors or concurrency conflicts.
- Allow editing immutable `history/runs/<run-id>/` content.
- Render raw HTML/scripts from Markdown without the existing safe sanitization
  boundary.

## Testing Strategy

- Component-test tree pairing, tabs, diagnostics, dirty state, conflict UI,
  backlinks, graph selection, refinement/task views, and archived read-only mode.
- API-mock test every foundation response/error used by the UI, especially 409,
  412, 422, 413, and reindex progress/failure.
- Browser-test keyboard navigation, concept creation/edit/save/reload, nested
  navigation, search/filter, broken links, rename/delete impact confirmation,
  and task updates.
- Accessibility-test labels, focus order, dialogs, status announcements, graph
  alternatives, contrast, reduced motion, and narrow viewport behavior.
- Security-test sanitized preview and ensure absolute paths/server internals
  never appear in DOM, logs, or errors.

## Acceptance Criteria

- [ ] Users can navigate every conforming ward/concept while a same-stem file and
      directory render as one concept with grouped children/refinement state.
- [ ] Users can create and edit conforming concept Markdown and metadata; invalid
      saves remain unsaved and show path/line-addressable diagnostics.
- [ ] Concurrent edits produce a conflict experience with reload/diff choices
      and never overwrite the newer document.
- [ ] Search supports text, ward, path, type, tag, and refinement-status filters
      with match provenance.
- [ ] Backlinks and a bounded graph neighborhood navigate correctly and visibly
      distinguish unresolved references.
- [ ] Concept spec, plan, task list, task status, and immutable prior runs are
      usable from the inspector.
- [ ] Rename/delete previews affected paths and inbound links and requires
      confirmation; history cannot be edited.
- [ ] Markdown preview is sanitized and no API/UI response exposes host paths.
- [ ] All interactions have keyboard-accessible non-graph equivalents and meet
      the project’s accessibility and semantic-style conventions.
- [ ] Existing UI tests plus new component/browser/accessibility suites pass.

## Assumptions

| Assumption | Type | Resolution |
| --- | --- | --- |
| The existing UI and reusable components remain product surfaces. | Product | Confirmed by user, 2026-07-19. |
| “Mini Obsidian” means linked Markdown navigation/editing, not plugin parity or a full Obsidian clone. | Product | Derived from the accepted RFC scope and confirmed Phase 2 direction. |
| Subdomain and concept are interchangeable terms. | Product | Confirmed by user, 2026-07-19. |
| Repeatable specs/plans/tasks live in the selected concept companion and may be updated to the newest run. | Product | Confirmed by user, 2026-07-19. |
| Foundation APIs and Phase 1 browser ship before this spec. | Process | Accepted split requested by user, 2026-07-19. |
| The UI consumes, but does not own, the OpenAPI contract. | Technical | Contract ownership assigned to the foundation spec. |
| UI styling follows semantic classes and tokens. | Technical | Verified in `apps/ui/ARCHITECTURE.md`. |
