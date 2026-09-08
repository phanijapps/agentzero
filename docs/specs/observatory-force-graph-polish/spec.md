# Spec: Observatory Force-Graph Polish

- **Status:** Shipped
- **Owner:** maintainer
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Discovery:** none
- **Contract:** none
- **Shape:** ui

> **Mode:** light (no risk trigger fired)

## Objective

An operator exploring the Observatory can read the D3 knowledge network as a deliberate, premium instrument: nodes, edges, controls, and context remain clear against a layered dark canvas while the existing graph data, selection, search, drag, and zoom behavior remain unchanged.

## Boundaries

### Always do

- Preserve existing D3 graph data, simulation, selection, search, drag, and zoom behavior.
- Use the existing UI tokens and local Observatory renderer/styles for visual polish.

### Ask first

- Change graph semantics, API payloads, route structure, or force-simulation behavior.

### Never do

- Add a graph-rendering dependency, a new module boundary, or a replacement visualization mode for this polish work.

## Testing Strategy

- **AC1–AC3:** Visual / manual QA in the live `/observatory` route, because the visual hierarchy and interaction affordances are user-visible outcomes.
- **AC4:** Goal-based checks through the existing Observatory tests, UI lint, and production build, backed by the same live-route interaction exercise.

## Acceptance Criteria

- [x] Given graph data, when the Observatory opens, the canvas shows a layered dark-space treatment with a restrained network glow and a readable in-canvas context label.
- [x] Given a node is selected or highlighted by search, when the graph updates, its visual emphasis remains distinct while unrelated nodes and edges recede without obscuring the selected result.
- [x] Given an operator uses the legend and zoom controls, when they hover or focus either surface, controls remain legible and visibly interactive at normal desktop sizes.
- [x] Given the graph renders, when built and exercised locally, existing entity selection, search highlighting, panning, zooming, and drag behavior still work.

## Assumptions

- Technical: The `/observatory` route renders `GraphCanvas`, which owns D3-force simulation, zoom, and SVG rendering (source: `apps/ui/src/features/observatory/GraphCanvas.tsx`).
- Technical: The UI uses React, D3, and shared CSS; `d3-force`, `d3-selection`, and `d3-zoom` are already installed (source: `apps/ui/package.json`).
- Product: The desired direction is dark, layered, restrained, and premium rather than a new data visualization mode (source: user confirmation 2026-08-26).
- Process: The mandatory base-freshness script is absent; the user explicitly approved proceeding without it (source: user confirmation 2026-08-26).
- Process: The experience-design pack is not installed; design intent is grounded in the existing Graph-v2 dark glass visual language (source: available skills roster; `apps/ui/src/features/observatory-v2/ObservatoryV2Page.tsx`).
