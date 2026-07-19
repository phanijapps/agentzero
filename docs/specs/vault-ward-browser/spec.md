# Spec: Vault Ward Browser

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** RFC-0006: Vault Obsidian-Style Ward Browser; [`durable-ward-memory`](../durable-ward-memory/spec.md)

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Add a top-level `Vault` tab where the local owner can browse real ward
directories in an Obsidian-style tree, open the Vault root from the top-level
navigation, drill into `Vault > Wards`, see breadcrumbs such as
`Vault > Wards > <ward name>`, and preview common accepted files in a read-only
right pane without exposing raw ward filesystem APIs to remote/LAN clients.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off
before proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Keep Vault browsing read-only in V1.
- Use ward-relative paths in API payloads and never return absolute host paths.
- Enforce the same backend allowlist and exclude policy for tree listings and
  direct file reads, and fuzzy file search.
- Treat `/api/vault/*` as a same-machine owner surface with remote/LAN clients
  denied in V1.
- Match existing React route, transport, theme token, and component patterns.

### Ask first

- Adding authenticated remote/LAN Vault access.
- Adding inline edit, create, rename, delete, move, or drag/drop behavior.
- Adding new third-party preview/parser dependencies.
- Changing the RFC-0006 V1 extension allowlist, exclude list, or file-size caps.
- Showing hidden files by default or adding user-configurable Vault filters.

### Never do

- Never let the browser UI read arbitrary host filesystem paths directly; V1
  reads are allowed only through bounded read-only `/api/vault/*` endpoints.
- Never write, mutate, delete, rename, archive, or restructure ward files from
  the Vault UI in V1.
- Never expose `.env`, `.env.*`, `*.env`, excluded config YAMLs, dependency/cache
  directories, hidden entries, or non-allowlisted files through tree or file
  endpoints.
- Never allow `ward_id` or `path` traversal to escape the canonical wards
  directory or selected ward root.
- Never list reserved/internal ward-root entries such as `_archive`,
  `_curator_backups`, `.usage.json`, hidden entries, or non-directory entries as
  normal selectable wards.
- Never execute ward `.html` scripts in V1; rendered HTML previews must stay in
  a sandboxed iframe without script permissions.

## Testing Strategy

- Backend path, allowlist, exclude, size, parser-limit, local-only, and response
  shape behavior: **TDD**. These rules are compressible invariants and should be
  covered by focused Rust HTTP/handler tests.
- Frontend transport and state behavior: **TDD**. Request paths, selected ward
  state, file metadata, and error states can be asserted with Vitest and mocked
  transport calls.
- User-visible Vault navigation, breadcrumbs, collapsible explorer behavior,
  fuzzy file search, tree expansion, file selection, and read-only preview
  rendering: **Visual / manual QA plus component tests**.
  Automated tests should simulate user gestures and assert visible text/states;
  a final manual smoke should run in the browser because this is a new primary
  app tab.
- Build integration: **Goal-based check**. `cargo check --workspace` and the UI
  test/build commands verify the changed Rust and TypeScript surfaces compile.

## Acceptance Criteria

- [x] The app includes a top-level `Vault` navigation item and route using the
  existing shell/nav style.
- [x] Opening `Vault` from the top-level navigation shows the Vault root. The
  root exposes `Wards`; opening `Wards` shows breadcrumb `Vault > Wards` and a
  list of active ward directories from the filesystem, including wards without
  memory facts.
- [x] The ward list excludes reserved/internal ward-root entries such as
  `_archive`, `_curator_backups`, `.usage.json`, hidden entries, and
  non-directory entries.
- [x] Selecting a ward shows breadcrumb `Vault > Wards > <ward name>` and a
  split-pane layout with a directory tree on the left and preview/metadata pane
  on the right. Before a file is selected, the right pane shows a `Ward content`
  landing state for the selected ward.
- [x] The selected-ward explorer is scoped to the selected ward only. Sibling
  wards are not shown in ward content mode; users return to `Vault > Wards` to
  choose another ward.
- [x] The explorer sidebar can be collapsed and expanded from the Vault header
  without changing the selected ward, selected file, or preview state.
- [x] The explorer/file pane can be resized with a visible splitter and keyboard
  controls while keeping the preview pane usable.
- [x] Vault scrollable panes use theme-consistent thin scrollbars rather than
  browser-default scrollbars.
- [x] A selected ward exposes in-sidebar fuzzy file search. Search results come
  from `/api/vault/wards/:ward_id/search?q=<query>&limit=<n>`, include only
  visible allowlisted files, use the same exclude policy as tree/file endpoints,
  return ward-relative paths, and can be clicked to open the read-only preview.
- [x] The tree lazy-loads ward-relative directories and lists directories plus
  only the V1 visible extensions: `.md`, `.txt`, `.yaml`, `.yml`, `.py`, `.js`,
  `.ts`, `.tsx`, `.html`, `.css`, `.json`, `.toml`, `.docx`, `.pptx`, `.doc`,
  and `.ppt`.
- [x] Tree and direct file endpoints both exclude `.venv/`, `venv/`,
  `node_modules/`, `__pycache__/`, `.git/`, `target/`, `dist/`, `build/`,
  `.next/`, `.cache/`, `.env`, `.env.*`, `*.env`, `config.yaml`, `config.yml`,
  `settings.yaml`, `settings.yml`, `secrets.yaml`, `secrets.yml`,
  `credentials.yaml`, `credentials.yml`, and hidden entries using
  case-insensitive matching.
- [x] Fuzzy search excludes the same hidden, env, config, dependency/cache,
  symlink, and non-allowlisted files as the tree endpoint.
- [x] Direct file reads return content only for `.md`, `.txt`, `.yaml`, `.yml`,
  `.py`, `.js`, `.ts`, `.tsx`, `.html`, `.css`, `.json`, `.toml`, `.docx`, and
  `.pptx`; `.doc` and `.ppt` are tree-visible metadata/open-externally items
  only.
- [x] Markdown renders through the shared Markdown renderer; text/code/YAML/JSON
  render as escaped read-only source; `.html` renders in a sandboxed iframe
  without scripts; `.docx` and `.pptx` use text-oriented Office Open XML
  previews; `.doc` and `.ppt` show a
  non-previewable metadata state with an `Open ward folder` affordance that uses
  the existing ward-open endpoint rather than exposing an absolute file path.
- [x] Backend file reads cap text/code/markdown at 2 MiB and Office Open XML
  compressed input at 15 MiB; the shared Office preview parser caps parsing at
  256 zip entries, 25 MiB total uncompressed XML, 300 `.pptx` slides, and
  200,000 extracted text characters, with clear oversized/parser-limit UI
  states.
- [x] Directory listings cap returned children at 1,000 entries and expose a
  `truncated: true` state that the UI renders clearly.
- [x] Fuzzy search caps requested results to 50, defaults to 30, scans at most
  20,000 filesystem entries per request, and returns `truncated: true` when the
  scan or result cap is hit.
- [x] `/api/vault/*` accepts same-machine local dashboard requests and denies
  remote/non-loopback requests with a stable `403 Forbidden` response in V1.
- [x] API responses use ward-relative paths, validate `ward_id`, canonicalize
  roots, reject path traversal, and never expose absolute host paths.
- [x] Existing Memory, Research, Observatory, Settings, and ward open-folder
  behavior continue to work without API contract changes.
- [x] `docs/specs/README.md` lists Vault Ward Browser as an active spec.

## Assumptions

- Technical: UI routes and top-level navigation are wired through
  `apps/ui/src/App.tsx` and its `navItems` array (source:
  `apps/ui/src/App.tsx`).
- Technical: gateway ward APIs already live under `gateway/src/http/`, and
  bounded vault path resolution is available through
  `VaultPaths::wards_dir()` and `VaultPaths::ward_dir()` (source:
  `gateway/src/http/mod.rs`; `gateway/gateway-services/src/paths.rs`).
- Technical: reusable Markdown, text/code, and Office preview code exists in
  the artifact slide-out area (source:
  `apps/ui/src/features/chat/ArtifactSlideOut.tsx`;
  `apps/ui/src/features/chat/officePreview.ts`;
  `apps/ui/src/features/shared/markdown/Markdown.tsx`).
- Technical: daemon host binding defaults to `0.0.0.0`, so raw ward file APIs
  need explicit local-only access control instead of inheriting general
  dashboard reachability (source: `apps/daemon/src/main.rs`).
- Process: no local `docs/CONVENTIONS.md` exists; this spec follows existing
  `docs/specs/*` shape and RFC-0006 (source: repository read, 2026-06-14).
- Product: V1 implements RFC-0006 as read-only browsing/preview only, with
  editing deferred (source: user confirmation 2026-06-14).
- Product: V1 `/api/vault/*` is local-only even if the rest of the dashboard is
  LAN-reachable (source: user confirmation 2026-06-14).
- Product: `.doc` and `.ppt` are visible as non-previewable metadata/open-
  externally items, not content-read files (source: user confirmation
  2026-06-14).
- Process: user confirmed proceeding with the Vault implementation on the new
  branch on 2026-06-14; V1 uses `jszip` as the bounded Office Open XML parser
  dependency so `.docx` and `.pptx` previews do not require a custom ZIP
  implementation.
