# Plan: Vault Ward Browser

- **Spec:** [`spec.md`](spec.md)
- **Status:** Complete

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially
> (a different approach, not just a re-ordering), note why in the changelog
> at the bottom.

## Approach

Implement the backend first so the UI has a stable contract: add a new
filesystem-backed `gateway/src/http/vault.rs` module with shared policy helpers
for active ward listing, local-only access checks, ward/path canonicalization,
extension visibility, content-read allowlists, excludes, and size/parser caps.
Then add typed UI transport methods and a `features/vault/` page that follows
the existing route/nav/theme patterns, lazy-loads tree nodes, and reuses
Markdown/text/Office preview code where practical. Finish with targeted Rust
and Vitest coverage plus a manual browser smoke for the new tab.

## Constraints

- RFC-0006 requires a top-level Vault tab, `/vault` root state, `Vault > Wards`
  drill-in, ward-relative APIs, read-only V1 preview, shared allow/exclude
  enforcement, active ward filtering, local-only `/api/vault/*`, and explicit
  caps.
- `durable-ward-memory` says wards remain durable source workspaces and should
  not be silently rewritten or replaced by summaries.
- No new top-level dependency or remote access design should be introduced
  without human sign-off.

## Construction tests

**Integration tests:** backend HTTP tests covering `/api/vault/wards`,
`/api/vault/wards/:ward_id/tree`, and `/api/vault/wards/:ward_id/file` with
filesystem fixtures; UI component/transport tests covering visible navigation,
breadcrumbs, collapsible explorer state, fuzzy search, tree selection, previews,
and error states.

**Manual verification:** run the app locally, open `/vault`, select a ward,
expand at least one directory, preview one Markdown/text/code file, confirm
excluded files are absent, and confirm `.doc`/`.ppt` show non-previewable
metadata state if fixture files are present.

**Recorded result:** 2026-06-14 browser smoke passed against the Vite app at
`http://127.0.0.1:3001/vault` using Playwright-routed Vault API fixtures:
opened `/vault`, selected `smoke-ward`, expanded `docs`, previewed
`docs/readme.md`, and captured `/tmp/agentzero-vault-smoke.png`.

**Recorded result:** 2026-06-14 browser smoke passed against the existing Vite
app on port 3000 after the mini-IDE visual pass: `/vault` rendered the framed
explorer/preview workspace at desktop and mobile sizes without visible overlap.

## Tasks

### T1: Backend Vault policy helpers enforce the filesystem contract

**Depends on:** none

**Mode:** TDD, with Rust unit tests in the new backend Vault module.

**Tests:**
- AC5-AC7, AC9-AC12: Rust unit tests prove extension allow/deny, direct-read
  allowlist, case-insensitive excludes, reserved ward-root filtering, `ward_id`
  validation, canonical path escape rejection, and size/parser cap decisions.

**Approach:**
- Add `gateway/src/http/vault.rs` with small pure helpers for policy and path
  resolution.
- Keep policy data local to the module unless another existing HTTP module
  needs it.
- Use existing `ErrorBody`-style JSON error shapes where practical.

**Done when:** helper tests fail without the policy code and pass with it.

### T2: Backend Vault endpoints return ward-relative tree and file payloads

**Depends on:** T1

**Mode:** TDD, with Rust HTTP/handler tests for the new routes.

**Tests:**
- AC2-AC12: HTTP/handler tests prove active ward listing, internal ward
  filtering, lazy directory listing, content reads, metadata-only legacy Office
  behavior, stable denied responses, truncated listings, local-only access, and
  no absolute host paths.

**Approach:**
- Register `GET /api/vault/wards`,
  `GET /api/vault/wards/:ward_id/tree`,
  `GET /api/vault/wards/:ward_id/search`, and
  `GET /api/vault/wards/:ward_id/file` in `gateway/src/http/mod.rs`.
- Read directory children only for the requested ward-relative directory.
- Return JSON for text-like content and metadata states; return bytes/content
  type only for `.docx` and `.pptx` compressed content after the 15 MiB backend
  input cap.
- Implement local-only request validation using Axum request connection info or
  the nearest existing gateway mechanism; do not rely on CORS alone.
- Implement fuzzy search as bounded recursive metadata discovery: visible files
  only, same exclude policy as tree/file, default 30 results, max 50 results,
  scan cap 20,000 entries, ward-relative paths only, and `truncated: true` when
  caps are hit.

**Done when:** targeted Rust tests for the new endpoints pass.

### T3: UI transport exposes typed Vault API calls

**Depends on:** T2

**Mode:** TDD, with Vitest transport tests.

**Tests:**
- AC2-AC12: Vitest tests for request URLs, path encoding, response typing, and
  error handling in the existing transport test style.

**Approach:**
- Extend `apps/ui/src/services/transport/types.ts` and `http.ts` with Vault
  ward, tree node, search result, and file content/metadata types.
- Keep API paths ward-relative and percent-encode user-controlled path segments
  or query values.

**Done when:** transport tests for Vault calls pass.

### T4: Shared Office preview parser enforces V1 parser limits

**Depends on:** T3

**Mode:** TDD, with Vitest parser tests around the shared Office preview helper.

**Tests:**
- AC9: parser tests prove `.docx`/`.pptx` previews fail with a typed
  preview-unavailable error when zip entry count, uncompressed XML bytes,
  `.pptx` slide count, or extracted text character caps are exceeded.
- AC8-AC9: existing valid `.docx` and `.pptx` fixture tests still produce
  text-oriented previews under the caps.

**Approach:**
- Move or extend the existing `apps/ui/src/features/chat/officePreview.ts`
  helper in place so both artifact previews and Vault previews use bounded
  parsing.
- Avoid adding a new preview framework or dependency; use existing `jszip`
  metadata and reads.
- Expose typed parser-limit failures so Vault can render the required state.

**Done when:** Office preview parser tests fail without the limits and pass with
them.

### T5: Vault UI route, navigation, ward list, and breadcrumbs render correctly

**Depends on:** T3

**Mode:** Visual/manual QA plus component tests, with Vitest asserting visible
route, breadcrumb, and split-pane output.

**Tests:**
- AC1-AC4: component tests simulate opening Vault at the root, drilling into
  `Vault > Wards`, selecting a ward, and seeing
  `Vault > Wards > <ward name>` with a split-pane layout.

**Approach:**
- Add `apps/ui/src/features/vault/` with `VaultPage`, `WardList`, `WardTree`,
  `FilePreviewPane`, and feature CSS following existing theme token patterns.
- Add a `Vault` nav item and route in `apps/ui/src/App.tsx`.
- Use lucide icons for folder/file/document/chevron affordances.

**Done when:** Vault route/nav tests pass and existing App tests are updated.

### T6: Vault tree lazy-load and read-only previews match V1 behavior

**Depends on:** T4, T5

**Mode:** Visual/manual QA plus component tests, with Vitest asserting visible
tree and preview states.

**Tests:**
- AC5-AC10: component tests simulate directory expansion, file selection,
  Markdown previews, text/code source previews, sandboxed HTML iframe preview,
  Office preview handoff, legacy `.doc`/`.ppt` metadata states,
  oversized/parser-limit states, and truncated directory state.

**Approach:**
- Implement lazy tree expansion keyed by ward-relative directory paths.
- Reuse shared Markdown and artifact preview helpers where practical.
- Render `.html` in a sandboxed iframe without script permissions.
- Render `.doc` and `.ppt` as non-previewable metadata with an `Open ward
  folder` action that calls the existing ward-open endpoint, not a file-specific
  absolute-path open.
- Provide clear empty, loading, error, non-previewable, oversized, and truncated
  states without adding edit controls.

**Done when:** Vault UI preview tests pass.

### T6b: Vault explorer collapse and fuzzy file search

**Depends on:** T2, T3, T5

**Mode:** TDD plus visual/manual QA.

**Tests:**
- Component tests prove the Vault explorer can collapse and expand while the
  page remains usable.
- Component tests select a ward, type a fuzzy search query, render backend
  results, click a result, and open the file preview.
- Transport tests prove `searchVaultFiles()` encodes ward id, query, and limit.
- Backend tests prove search finds nested visible files and excludes hidden,
  env, config, non-visible, and symlinked paths.

**Approach:**
- Add a header/sidebar toggle that hides the explorer and lets the preview pane
  consume the workspace; keep state in `VaultPage`.
- Add selected-ward search UI inside the explorer, with debounced backend
  search, stale-response guards, result truncation copy, and clickable file
  results.
- Reuse `FileIcon` and `selectFile()` so search results follow the same preview
  path as tree selections.

**Done when:** backend, transport, and Vault component search/collapse tests
pass, and browser smoke confirms desktop/mobile layout.

### T7: Gates, manual smoke, and docs index are complete

**Depends on:** T1-T6

**Mode:** Goal-based check plus manual QA, using the named commands and recorded
manual smoke result.

**Tests:**
- AC13: run existing UI regression tests for neighboring surfaces:
  `npm --prefix apps/ui run test -- src/App.test.tsx
  src/features/memory/command-deck/MemoryTab.test.tsx
  src/features/research-v2/ResearchPage.test.tsx
  src/features/observatory/ObservatoryPage.test.tsx
  src/features/settings/WebSettingsPanel.test.tsx`.
- AC13: run transport regression tests for ward APIs, including an added
  `openWard` assertion if absent:
  `npm --prefix apps/ui run test -- src/services/transport/http.class.test.ts`.
- AC13: run the new backend Vault route tests plus any added ward-open
  regression in `gateway/tests/` or `gateway/src/http/ward_actions.rs`.
- AC14 plus build health: run `cargo check --workspace`,
  `npm --prefix apps/ui run test -- <changed Vault test files>`, and
  `npm --prefix apps/ui run build`.
- Manual smoke records opening Vault, selecting a ward, expanding a directory,
  fuzzy-searching a file, collapsing/expanding the explorer, and previewing a
  file.

**Approach:**
- Update `docs/specs/README.md` to list Vault Ward Browser.
- Run mechanical gates and fix failures in scope.
- Run adversarial reviewer before final status.

**Done when:** gates pass or any environmental failures are documented with the
exact command/output, reviewer is clean, and the manual smoke result is recorded.

## Rollout

Ship as a normal dashboard feature without a feature flag. V1 is read-only and
local-only, so rollback is removal of the route/nav item and `/api/vault/*`
routes.

## Risks

- Local-only access may require plumbing peer address extraction through the
  existing Axum router.
- Office preview helpers currently live near chat artifacts; reuse must stay
  small and avoid a broad preview-system refactor.
- The current worktree contains unrelated uncommitted changes; implementation
  must avoid reverting or reshaping those files except where the Vault feature
  directly requires edits.

## Changelog

- 2026-06-14: initial plan from RFC-0006 and confirmed product assumptions.
- 2026-06-14: recorded `jszip` as the approved bounded Office Open XML parser
  dependency and added browser smoke evidence for the Vault route.
- 2026-06-14: marked V1 implementation complete after backend, UI, build,
  regression, browser-smoke, and adversarial-review gates.
- 2026-06-14: added collapsible explorer and bounded fuzzy file search as a
  shipped enhancement, with backend route, transport, UI tests, and updated
  docs.
