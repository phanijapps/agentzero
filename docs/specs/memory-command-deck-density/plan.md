# Plan: Memory Command Deck Density

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Tasks

### T1: Bound and label the Memory workbench

**Depends on:** none

**Touches:** `apps/ui/src/features/memory/command-deck/MemoryTab.tsx`,
`apps/ui/src/features/memory/command-deck/WardRail.tsx`,
`apps/ui/src/features/memory/command-deck/WriteRail.tsx`,
`apps/ui/src/features/memory/command-deck/MemoryTab.test.tsx`,
`apps/ui/src/styles/components.css`

**Tests:** a rendered deck has labelled scope, evidence, and curation regions;
the focused Memory suite, lint, and production build pass.

**Approach:** add gallery-aligned masthead and pane semantics, then constrain
the desktop grid and its scrollable children with `min-height: 0`. On narrow
viewports, restore a single natural document scroll while retaining a bounded
central evidence pane.

**Done when:** the central evidence list is the desktop scroll surface and no
existing memory action changes behavior.

## Rollout

Ship in the UI bundle. No migration, API deployment, or feature flag is
required. Rollback is a UI-only revert.

## Changelog

- 2026-07-15: completed T1 and verified the full focused Memory suite, lint,
  production build, desktop capture, and narrow-screen capture.
