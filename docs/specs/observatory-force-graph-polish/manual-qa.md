# Manual QA: Observatory Force-Graph Polish

- **Route:** `http://127.0.0.1:3002/observatory`
- **Environment:** local Vite development server against the active local daemon
- **Scope:** visual rendering and existing graph interactions for the D3 Observatory only

## Observed result

The live graph rendered 200 entities and 500 relationships on the layered dark canvas. The in-canvas network context, ambient grid and glow, glass treatment, legend, and zoom controls were visible and readable. Selecting a rendered node opened its entity detail panel; entering `agent` in the search field applied highlighting; and the Zoom in control accepted interaction.

## Session boundary

This session verifies the visual rendering and the existing data-backed graph view. It does not exercise alternate agent filters, empty/error states, or the separate `/observatory-v2` 3D route.
