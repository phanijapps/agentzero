# Plan: OKF Mini-Obsidian UI

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Extend the Phase 1 browser in vertical slices: editable document workspace,
link/backlink navigation, graph neighborhood, then concept refinement views.
Reuse the existing Vault/Memory feature boundary and shared components. Keep all
mutations behind the foundation client so UI state cannot bypass validation,
confinement, or concurrency rules.

## Constraints

- Depends on the OKF Ward Foundation being shipped and contract-compatible.
- Do not recreate wiki-specific stores, routes, terminology, or templates.
- Follow semantic CSS/design tokens and preserve keyboard/non-visual navigation.
- Archived runs are immutable; absolute paths are never accepted or displayed.

## Construction tests

- Build API client mocks for every consumed operation/error before components.
- Add interaction tests with same-stem concept fixtures and nested book/chapter
  and valuation/refinement examples.
- Add accessibility assertions and sanitized Markdown fixtures early.

## Design (LLD)

- Maintain one normalized selection model keyed by root-relative concept path.
- Use server ETags as the editor version; keep draft, saved, validating,
  conflicted, and invalid states explicit in a reducer/state machine.
- Render graph data as an enhancement over an accessible backlink/forward-link
  list, with server-bounded nodes/edges.
- Model refinement tabs from companion resources rather than parsing arbitrary
  directory names in components.
- Add semantic component classes/modifiers to `components.css` and tokens only
  when a genuinely new design decision is required.

## Tasks

1. Extend the typed OKF API client and query cache for concept mutations,
   backlinks, graph, validation, and refinement resources.
2. Refactor Phase 1 selection state so a concept and same-stem companion appear
   as one tree node with stable deep links.
3. Add Markdown source editor, sanitized preview, metadata controls, dirty-state
   guard, ETag save, inline validation, and conflict reload/diff flow.
4. Add create/rename/delete dialogs with collision, affected-link, and immutable
   history handling.
5. Add backlink/forward-link inspector and accessible bounded graph neighborhood
   with unresolved-link states.
6. Add spec, plan, active tasks, task status, and archived-run inspector views.
7. Add search/filter chips for type, tag, path, ward, and refinement status;
   retain provenance in results.
8. Add keyboard shortcuts, focus management, live announcements, graph list
   alternative, reduced-motion behavior, and responsive panes.
9. Complete component, browser, accessibility, and security tests; update
   `apps/ui/ARCHITECTURE.md` and user documentation.

## Rollout

Enable after foundation telemetry shows stable validation/search/reindex behavior.
Use a temporary Phase 2 UI feature flag only for release safety; remove it once
the authoring workspace is the supported ward UI. Server APIs remain authoritative
throughout, so disabling the UI does not alter ward contents.

## Risks

- Editing complexity expands into an IDE: keep the editor Markdown-focused and
  reject plugin/WYSIWYG scope.
- Graph rendering becomes slow or inaccessible: request bounded neighborhoods
  and retain equivalent lists.
- Lost updates: require ETags and explicit conflict resolution.
- Concept/companion confusion: centralize pairing in the API/client selection
  model and test nested examples.
- Unsafe preview: use the established sanitizer and hostile Markdown fixtures.

## Changelog

- 2026-07-19: Initial Phase 2 plan branched from accepted RFC-0015.
