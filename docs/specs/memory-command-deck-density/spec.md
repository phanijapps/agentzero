# Spec: Memory Command Deck Density

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [Attention Radar UI gallery](../../product/attention-radar-ui-gallery.html#memory)
- **Brief:** user request, 2026-07-15
- **Contract:** none
- **Shape:** ui

> **Mode:** light (no risk trigger fired).

## Objective

Make `/memory` follow the gallery's three-pane recall command deck while
keeping large result sets compact: the evidence pane owns its own scroll
surface, and scope and curation remain available without extending the page.

## Boundaries

### Always do

- Preserve existing recall, filters, tabs, ward selection, writes, and deletes.
- Keep the desktop layout as scope, evidence, and curation panes.
- Make all controls reachable without horizontal overflow on narrow screens.

### Never do

- Change a memory query, storage provider, API contract, or persistence data.
- Add a virtualizer or a dependency for this layout-only correction.
- Duplicate the existing guarded write controls with a second composer.

## Testing Strategy

- **TDD:** render the Memory command-deck landmarks that correspond to its
  three operating panes.
- **Goal-based check:** focused Memory tests, UI lint, production build, and
  CSS diff check.
- **Visual/manual QA:** with a ward containing many records, the central
  evidence list scrolls independently on desktop while both rails stay within
  the viewport; a narrow viewport has no horizontal clipping.

## Acceptance Criteria

- [x] `/memory` exposes a labelled scope, evidence, and curation workbench
  beneath a durable-knowledge masthead.
- [x] On desktop, a large Facts, search, beliefs, or contradictions result set
  scrolls inside the evidence pane instead of making the page vertically long.
- [x] Ward scope and guarded curation controls have their own bounded,
  independently scrollable panes when their contents exceed the viewport.
- [x] Existing recall filters, content tabs, memory writes, and deletion flows
  retain their current behavior.
- [x] At narrow widths, the layout becomes a single accessible column without
  horizontal overflow or inaccessible controls.

## Assumptions

- Product: the Memory portion of the Attention Radar gallery is the visual
  contract, while existing live controls are retained where they already serve
  the corresponding job (source: `docs/product/attention-radar-ui-gallery.html#memory`).
- Technical: `.memory-deck__body` is already intended as the evidence scroll
  surface, but its grid parents do not constrain the deck's minimum height
  (source: `apps/ui/src/styles/components.css`).
- Process: this focused UI update keeps its paired spec and criteria current
  with implementation (source: `docs/CONVENTIONS.md` § Spec metadata contract).

## Changelog

- 2026-07-15: shipped a labelled, gallery-aligned Memory workbench with
  independently bounded scope, evidence, and curation scroll surfaces; the
  narrow search controls now wrap rather than clip.
