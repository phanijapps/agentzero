# Plan: Ward Vault In Research

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially
> (a different approach, not just a re-ordering), note why in the changelog at
> the bottom.

## Approach

Extract the existing route-owned Vault tree/search/preview behavior into
reusable ward-scoped components, then render those components from both
`VaultPage` and `ResearchPage`. The backend and transport contract should stay
unchanged unless extraction exposes a concrete gap. Research integration should
add a left explorer column only after `state.wardId` is known, while selected
files open in a Research-local slide-out that reuses the same preview renderer.

Tempted to add a new Research-specific filesystem API; declining unless the
existing `/api/vault/*` methods cannot express the required scoped explorer.
Tempted to add an editor-pane abstraction; declining because V1 is read-only
preview. Tempted to rewrite Vault styling; declining because the user explicitly
wants theme/style consistency, not a new visual system.

## Constraints

- RFC-0007 recommends a Research-local, ward-scoped explorer after `wardId`,
  existing `/api/vault/*` first, slide-out preview, and one active session/root
  ward for V1.
- `vault-ward-browser` is shipped and defines the read-only Vault API policy,
  extension allowlist, excludes, preview behavior, local-only access, and
  existing UI expectations.
- `apps/ui/AGENTS.md` requires UI work to follow existing feature-module,
  transport, React, TypeScript, Vite, and theme-token patterns.

## Construction tests

**Integration tests:** focused Vitest coverage for extracted Vault components,
existing `/vault` page behavior, and Research page integration.

**Manual verification:** run the UI locally, open an existing research session
with a ward, verify the left explorer loads only that ward, search returns
files from that ward, clicking Markdown/HTML opens a slide-out, the explorer
collapses/expands, and the ward chip still opens `/vault?ward=<wardId>`.

## Tasks

### T1: Reusable Vault components preserve existing Vault behavior

**Depends on:** none

**Mode:** TDD.

**Tests:**
- AC5, AC6, AC8, AC9, AC13: existing `VaultPage.test.tsx` still proves tree
  expansion, ward-scoped search, Markdown/HTML/Office previews,
  non-previewable `.doc` / `.ppt`, collapse, resize, and route behavior.
- AC13: new or updated component tests prove `VaultPage` delegates to the
  reusable ward explorer without changing visible `/vault` output.

**Approach:**
- Extract stateful ward-scoped explorer behavior from
  `apps/ui/src/features/vault/VaultPage.tsx` into reusable components under
  `apps/ui/src/features/vault/`.
- Keep route breadcrumbs, Vault root/Wards navigation, and ward list ownership
  in `VaultPage`.
- Keep preview rendering in a reusable component so Research and Vault use the
  same Markdown, HTML, text/code, and Office behavior.

**Done when:** the Vault test file passes without reducing existing assertions.

### T2: Research renders the ward-scoped explorer only after ward state exists

**Depends on:** T1

**Mode:** TDD plus visual/manual QA.

**Tests:**
- AC1, AC2, AC3, AC4, AC5: Research component tests render no explorer without
  `wardId`, render the explorer with `wardId`/`wardName`, call
  `getVaultTree(wardId, "")`, and do not call Vault APIs for wardless sessions.
- AC3: `session-snapshot.test.ts` or `useResearchSession.test.ts` proves an
  opened existing session whose `/api/sessions/:id/state` snapshot includes a
  ward hydrates `state.wardId` / `state.wardName`; the Research component test
  then proves that hydrated state renders the embedded explorer.
- AC4: `event-map.test.ts`, `reducer.test.ts`, and the existing full-flow
  Research hook/integration tests prove live `ward_changed` events update
  `state.wardId` / `state.wardName`; the Research component test then proves
  the newly-known ward renders and loads the embedded explorer.
- AC12: Research component test proves clicking the ward chip still navigates
  to `/vault?ward=<wardId>`.

**Approach:**
- Import the reusable explorer into
  `apps/ui/src/features/research-v2/ResearchPage.tsx`.
- Add a two-column Research body when `state.wardId` exists; keep the existing
  single-column body otherwise.
- Add layout CSS in the existing Research/Vault style areas using current
  theme tokens and responsive breakpoints.

**Done when:** Research tests prove the ward and no-ward branches.

### T3: Research file clicks open a contextual Vault file slide-out

**Depends on:** T1, T2

**Mode:** TDD plus visual/manual QA.

**Tests:**
- AC7, AC8, AC9, AC10: Research tests click a tree file and a search result,
  assert `getVaultFile(wardId, path)`, assert visible Markdown/HTML preview
  content in a slide-out, assert `.doc` / `.ppt` do not call `getVaultFile`,
  and assert closing the preview leaves transcript/composer content visible.
- AC6, AC7: Research tests type into the embedded fuzzy search, assert
  `searchVaultFiles(<session ward id>, query, 30)`, render search results, and
  click a result that opens the same slide-out preview path as a tree click.

**Approach:**
- Add a `VaultFileSlideOut` shell that wraps the reusable preview renderer with
  Research-appropriate open/close behavior.
- Let the reusable explorer own tree/search state and report selected files
  through the same selection path used by Vault.
- Avoid coupling to artifact-specific `ArtifactSlideOut` data shapes.

**Done when:** Research file-preview tests pass.

### T4: Explorer collapse/expand and responsive behavior are stable in Research

**Depends on:** T2, T3

**Mode:** Visual / manual QA plus targeted component tests.

**Tests:**
- AC11: component tests collapse and expand the Research explorer and assert
  ward, tree, search, and selected-file state remain available.
- AC1, AC10: component tests assert the right Research column still contains
  transcript/composer content while preview is open.

**Approach:**
- Reuse the Vault explorer collapse affordance where practical.
- Keep responsive CSS conservative: desktop two columns, narrow screens
  toggleable/collapsed explorer above or before the Research column.

**Done when:** tests pass and manual browser smoke shows no obvious overlap at
desktop and mobile widths.

### T5: Documentation, gates, and work-loop review are complete

**Depends on:** T1-T4

**Mode:** Goal-based check.

**Tests:**
- AC14: `docs/specs/README.md` contains the active spec entry.
- Goal checks: `npm --prefix apps/ui run test -- src/features/vault/VaultPage.test.tsx src/features/research-v2/ResearchPage.test.tsx`, `npm --prefix apps/ui run build`, and `git diff --check`.

**Approach:**
- Update the spec status as implementation starts/finishes.
- Run the work-loop gates and adversarial implementation review.
- Fix review findings before publishing the implementation PR.

**Done when:** documented gates pass and the implementation review is clean or
all accepted findings are fixed.

## Rollout

Ship in the normal UI bundle. No feature flag is planned because the explorer
is only visible after a Research session has a ward and reuses existing
read-only Vault APIs.

## Risks

- Extracting `VaultPage` may accidentally change existing `/vault` behavior.
  Mitigation: keep `VaultPage.test.tsx` green and avoid route-level rewrites.
- Research layout may crowd the transcript on small screens. Mitigation:
  collapse/toggle the explorer responsively and verify manually.
- Sharing preview rendering could couple Research to Vault route state.
  Mitigation: keep preview props data-oriented and route-free.
- Existing Research tests already emit some `act(...)` warnings. Mitigation:
  add assertions on visible behavior and do not treat unrelated warning cleanup
  as part of this feature.

## Changelog

- 2026-06-14: initial plan.
- 2026-06-14: implemented via reusable Vault explorer/preview extraction and
  Research page integration.
