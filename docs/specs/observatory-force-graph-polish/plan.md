# Plan: Observatory Force-Graph Polish

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Enhance the existing SVG graph in place. Add semantic SVG layers for a subtle ambient treatment and connection focus, then use scoped Observatory CSS for the premium canvas, node, edge, legend, and control states. Keep the D3 simulation parameters, transport data, route, and existing interactions intact.

## Tasks

### T1: Make the D3 graph read as an intentional Observatory surface

**Depends on:** none

**Touches:** `apps/ui/src/features/observatory/GraphCanvas.tsx`, `apps/ui/src/styles/components.css`, `apps/ui/src/features/observatory/graph-hooks.test.ts`

**Tests:**
- Visual / manual QA (AC1–AC4): run the UI and inspect the live `/observatory` graph while selecting, searching, dragging, panning, and zooming.
- Goal-based check (AC4): `npm run lint`, `npm run build`, and the Observatory graph tests pass.

**Approach:**
- Add non-data SVG decoration and edge/node classes that let CSS establish hierarchy without changing the simulation or event contract.
- Add scoped styles for the canvas, graph affordances, legend, and controls using the existing design tokens and the established dark-glass aesthetic.
- Update focused graph tests only if DOM-level behavior changes require it.

**Done when:** The real Observatory presents a readable layered force graph and all existing interaction paths continue to work.

## Declined additions

- New graph-rendering dependency — D3 already supplies the required simulation and SVG surface.
- A new graph data mode or API — the request is visual polish, not a semantic-model change.
- A separate component layer — the change remains local to the existing graph renderer and styles.
