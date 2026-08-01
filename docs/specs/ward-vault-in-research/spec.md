# Spec: Ward Vault In Research

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [RFC-0007: Ward-Specific Vault Explorer in Research](../../rfc/0007-ward-specific-vault-in-research.md); [`vault-ward-browser`](../vault-ward-browser/spec.md)

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

When a Research session has an active ward, show that ward's filesystem beside
the existing Research transcript/composer so the local owner can inspect files
without leaving `/research/:sessionId`. The left column is a read-only,
ward-scoped Vault explorer with fuzzy search; clicking a previewable file opens
a contextual slide-out preview while the Research UI remains visible.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off before
proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Scope the embedded explorer and search to `ResearchSessionState.wardId` only.
- Reuse the existing `/api/vault/*` transport methods, Vault allow/exclude
  policy, shared Markdown renderer, and Office preview helper before adding any
  new API or parser behavior.
- Keep the existing Research transcript, artifact strip, composer, stop action,
  and ward-chip navigation behavior available.
- Preserve the top-level `/vault` route as the full Vault browsing surface.
- Make the explorer collapsible on narrow or crowded layouts.

### Ask first

- Adding a backend endpoint beyond a narrow adapter that delegates to the same
  Vault policy.
- Making Research Vault files editable, writable, renameable, deletable, or
  draggable.
- Changing the Vault extension allowlist, exclude list, local-only policy, file
  size caps, or HTML sandbox posture.
- Supporting multiple simultaneous ward explorers for child-agent wards.

### Never do

- Never show sibling wards inside the Research embedded explorer.
- Never read ward files through raw filesystem paths or expose absolute host
  paths in the UI.
- Never bypass existing Vault path validation, hidden-file filtering, env/config
  excludes, dependency/cache excludes, or previewability checks.
- Never replace or hide the Research transcript/composer when a file is opened.
- Never add a new top-level UI dependency or new preview framework for V1.

## Testing Strategy

- Reusable Vault explorer state and preview behavior: **TDD** with Vitest
  component tests. Tree loading, ward scoping, fuzzy search, file selection,
  non-previewable files, Markdown rendering, HTML iframe sandboxing, and Office
  handoff are observable with mocked transport calls.
- Research integration behavior: **TDD plus visual/manual QA**. Tests should
  render Research with ward state, assert the left explorer appears, verify
  search/tree calls use only that ward id, and verify clicked files open a
  slide-out preview without removing transcript/composer content.
- Layout, collapse, and slide-out ergonomics: **Visual / manual QA plus
  targeted component assertions**. The observable contract is that the explorer
  can be collapsed/expanded and the Research page remains usable at desktop and
  mobile widths.
- Build integration: **Goal-based check**. `npm --prefix apps/ui run build`
  proves TypeScript and Vite accept the extracted component boundaries.

## Acceptance Criteria

- [x] A Research session with `wardId` and `wardName` renders a two-column body:
  left ward Vault explorer/search, right existing Research UI.
- [x] A Research session without a ward keeps the current single-column
  experience and does not call Vault tree/search/file APIs.
- [x] Opening an existing `/research/:sessionId` whose snapshot contains a ward
  renders the same ward-scoped explorer after snapshot hydration.
- [x] When live `ward_changed` sets the session ward during new research, the
  embedded explorer appears and loads only that ward root.
- [x] The embedded explorer tree never lists sibling wards; it shows only the
  selected ward root and its child directories/files.
- [x] Fuzzy file search in Research calls
  `searchVaultFiles(<session ward id>, query, 30)` and renders only results from
  that ward.
- [x] Clicking a previewable tree node or search result opens a Research-local
  slide-out preview and calls `getVaultFile(<session ward id>, path)`.
- [x] Markdown previews use the shared Markdown renderer; text/code render as
  escaped read-only source; `.html` renders in a sandboxed preview iframe;
  `.docx` and `.pptx` use the existing Office preview helper.
- [x] Non-previewable `.doc` and `.ppt` entries show metadata/non-previewable
  state and do not call `getVaultFile`.
- [x] Closing the file preview returns focus to Research with transcript,
  artifact strip, and composer still visible.
- [x] The embedded explorer can be collapsed and expanded without losing the
  selected ward, tree state, search query, or selected file.
- [x] The top-level ward chip still navigates to `/vault?ward=<wardId>`.
- [x] Existing `/vault` behavior and tests continue to pass after extracting
  reusable Vault explorer/preview components.
- [x] `docs/specs/README.md` lists Ward Vault In Research as an active spec.

## Assumptions

- Technical: UI is React 19 + TypeScript + Vite with Vitest and Playwright
  available (source: `apps/ui/package.json`).
- Technical: Research already has sticky `wardId` / `wardName` state and
  ward-chip navigation (source:
  `apps/ui/src/features/research-v2/ResearchPage.tsx`;
  `apps/ui/src/features/research-v2/types.ts`;
  `apps/ui/src/features/research-v2/session-snapshot.ts`).
- Technical: existing Vault transport methods already cover tree, search, and
  file preview calls (source: `apps/ui/src/services/transport/http.ts`;
  `apps/ui/src/services/transport/interface.ts`).
- Technical: Vault preview already uses the shared Markdown renderer and Office
  preview helper (source: `apps/ui/src/features/vault/VaultPage.tsx`;
  `apps/ui/src/features/shared/markdown/Markdown.tsx`).
- Process: Vault browser spec is shipped and should be the main implementation
  precedent (source: `docs/specs/vault-ward-browser/spec.md`).
- Process: repo has no `docs/CONVENTIONS.md` or `docs/CHARTER.md`; this spec
  follows existing `docs/specs/*` shape and RFC-0007 (source: repository read
  2026-06-14).
- Product: it is okay to start implementation before RFC PR #214 is merged,
  using this implementation branch as dependent work (source: user confirmation
  2026-06-14).
- Product: V1 is read-only: left ward-scoped explorer/search inside Research,
  clicked files open in a slide-out preview, no editing (source: user
  confirmation 2026-06-14).
- Product: reuse existing `/api/vault/*` first; add a narrow new API only if
  code verification shows reuse causes awkward duplication or incorrect UX
  (source: user confirmation 2026-06-14).
