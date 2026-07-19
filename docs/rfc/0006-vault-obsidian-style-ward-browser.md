# RFC-0006: Vault Obsidian-Style Ward Browser

- **Status:** Draft
- **Author:** phanijapps
- **Approver:** phanijapps
- **Date opened:** 2026-06-14
- **Date closed:**
- **Related:** `docs/specs/durable-ward-memory/spec.md`; `docs/specs/ward-side-panel/`; `apps/ui/ARCHITECTURE.md`; `gateway/src/http/ward_content.rs`; `gateway/src/http/ward_actions.rs`

## The ask

Approve a new top-level `Vault` tab that lets users browse the real ward
filesystem in an Obsidian-style tree, open a ward with a breadcrumb path such as
`Vault > Wards > stock-analysis`, and preview common safe file types in a
right-hand pane.

z-Bot already treats wards as durable user-owned workspaces, but the dashboard
mostly exposes derived memory views and native-folder open actions. Users need a
first-party way to inspect the ward contents without leaving the app. The
question is how much filesystem surface to expose, and how to keep it consistent
with the existing theme and Rust gateway boundaries.

Decisions requested:

1. Add a top-level `Vault` navigation item. Recommended: accept, because
   filesystem browsing is distinct from the existing Memory fact/wiki deck.
   Decide by 2026-06-14.
2. Make the top-level `Vault` link show the Vault root, with `Wards` as the
   first root entry that opens `Vault > Wards`. Recommended: accept, because it
   leaves room for other Vault settings/sections while keeping wards primary.
   Decide by 2026-06-14.
3. Add read-only, ward-relative filesystem APIs for the tree and file content.
   Recommended: accept, because the UI should not receive arbitrary host paths
   as its API contract even though this is a user-owned desktop vault. Decide by
   2026-06-14.
4. Ship read-only preview in V1 and leave editing as a follow-on design.
   Recommended: accept, because silent ward mutation is already out of bounds in
   durable ward memory specs. Decide by 2026-06-14.

## Problem & goals

The current app has multiple ward-aware surfaces, but none of them provides a
direct in-app view of the ward filesystem:

- Memory shows facts, wiki articles, procedures, and episodes grouped by ward.
- Research and artifact surfaces can open individual artifacts or native ward
  folders.
- The backend already knows the vault and ward paths through `VaultPaths`.

That leaves a gap for users who want to understand what is actually inside a
ward: `AGENTS.md`, `memory-bank/`, specs, reports, scripts, documents, and
generated outputs.

Goals:

- Add `Vault` as a primary app tab using the existing shell/navigation patterns.
- Default the Vault tab to the Vault root and expose `Wards` as the primary
  root entry.
- List real ward directories from the filesystem, including wards without memory
  facts.
- Show `Vault > Wards > <ward name>` after a ward is selected.
- Render an Obsidian-style expandable directory tree for the selected ward.
- Let the explorer sidebar collapse/expand so the preview pane can take the full
  workspace when the user is reading a file.
- Provide fuzzy file search for the selected ward from the explorer sidebar.
- Show directories and a focused allowlist of common user-facing files.
- Preview accepted files in a right-hand pane without requiring the native file
  browser.
- Keep all API paths ward-relative under the selected ward.
- Match the existing design-token and component-class system.

Non-goals:

- No automatic ward cleanup, restructuring, deletion, rename, or archival.
- No V1 inline editing, save, rename, drag/drop move, or new file creation.
- No replacement of Memory, Graph, or Observatory surfaces.
- No attempt to preview every binary, generated artifact, cache, or dependency
  directory.
- No new ward execution mechanism. `ward:{name}` delegation remains the runtime
  path.

## Proposal

Add a `Vault` route and page under the React app, likely
`apps/ui/src/features/vault/`, and add a `Vault` item to `navItems` in
`apps/ui/src/App.tsx`.

The top-level Vault route renders the Vault root:

```text
Vault
```

The first Vault root entry is `Wards`. Opening it renders:

```text
Vault > Wards
```

When a user clicks a ward, the page renders:

```text
Vault > Wards > <ward name>
```

The selected ward view uses split panes:

- Left pane: an expandable filesystem tree rooted at the selected
  `wards/<ward_id>/` only. Sibling wards are hidden until the user navigates
  back to `Vault > Wards`. The pane can be resized with a visible splitter and
  keyboard controls.
- Right pane: a read-only preview for the selected file, or an empty/metadata
  state when no previewable file is selected.

The tree should be lazy-loaded by directory path rather than eagerly walking the
entire ward. Live local wards contain nested project output and large hidden or
generated folders such as `.venv`, so eager recursion is the wrong default.

### File visibility

Directories are visible unless excluded. Files are visible only when their
extension is accepted for browsing or preview.

V1 accepted file extensions:

| Extension | V1 behavior |
| --- | --- |
| `.md` | render with the shared Markdown renderer |
| `.txt` | plain text preview |
| `.yaml`, `.yml` | plain text/code preview, except excluded config files |
| `.py`, `.js`, `.ts`, `.tsx`, `.css`, `.json`, `.toml` | read-only code/text preview |
| `.html` | rendered in a sandboxed iframe without script permissions |
| `.docx` | text-oriented Office Open XML preview using existing artifact preview helpers |
| `.pptx` | text-oriented Office Open XML preview using existing artifact preview helpers |
| `.doc`, `.ppt` | visible as non-previewable files, but V1 uses tree metadata/open externally only unless a converter is added |

The V1 extension visibility allowlist applies to tree listings. The V1 content
read allowlist for `/file?path=...` is narrower: `.md`, `.txt`, `.yaml`,
`.yml`, `.py`, `.js`, `.ts`, `.tsx`, `.html`, `.css`, `.json`, `.toml`, `.docx`,
and `.pptx`. Legacy `.doc` and `.ppt` files are tree-visible but must not return
raw content in V1; selecting them should use the tree metadata and offer
open-externally affordances only. A file that is not in the visibility allowlist
must not be returned by the tree. A file that is not in the content read
allowlist must not be fetched through `/file?path=...` in V1. Non-accepted files
should return the same stable `403 Forbidden` or `404 Not Found` shape used for
excluded paths.

Default excludes for both tree listings and direct file reads:

- Dependency/cache directories from this fixed V1 set: `.venv/`, `venv/`,
  `node_modules/`, `__pycache__/`, `.git/`, `target/`, `dist/`, `build/`,
  `.next/`, and `.cache/`.
- `.env`, `.env.*`, `*.env`.
- `config.yaml`, `config.yml`, `settings.yaml`, `settings.yml`,
  `secrets.yaml`, `secrets.yml`, `credentials.yaml`, and `credentials.yml`.
- Hidden files and hidden directories by default, except explicitly allowed
  project files such as `.gitignore` only if a future setting opts in.

The shared backend matcher must case-fold extensions, basenames, and path
components before applying both allowlist and exclude checks. Mixed-case forms
such as `.ENV`, `Secrets.yaml`, `CONFIG.YML`, or `.Venv/` should behave the same
as their lowercase equivalents in both the tree and direct file endpoints.

The user owns the vault filesystem, so this is not a multi-tenant secrecy
boundary. These excludes are still useful because secrets and configuration
files are easy to expose accidentally and rarely useful in a broad browsing
default. The exclude policy must live in shared backend code used by both the
tree endpoint and the file endpoint, alongside the extension allowlist; a user
should not be able to hide `.env` in the tree but still fetch it directly with a
crafted `/file?path=.env` request. Excluded paths should return a stable `403
Forbidden` or `404 Not Found` response, with the implementation choosing one
behavior and testing it. Any additional exclude patterns are deferred to the
follow-on `docs/specs/vault-ward-browser/` spec or a later settings design.

### Backend

Add a filesystem-backed HTTP module, separate from the existing memory
aggregator:

```text
GET /api/vault/wards
GET /api/vault/wards/:ward_id/tree?path=<ward-relative-dir>
GET /api/vault/wards/:ward_id/search?q=<fuzzy-query>&limit=<n>
GET /api/vault/wards/:ward_id/file?path=<ward-relative-file>
```

V1 access contract: `/api/vault/*` is a same-machine owner surface, not a LAN
file-sharing API. The handler must allow loopback or otherwise trusted
same-device requests from the local dashboard, and must deny remote/LAN clients
with a stable `403 Forbidden` response when the request cannot be proven local.
The daemon's ability to bind `0.0.0.0` for other dashboard features must not
implicitly expose raw ward file reads. CORS or origin checks are not sufficient
as the only control. If remote Vault access is desired later, it needs an
explicit authenticated access design and a separate spec/RFC amendment.

The existing `GET /api/wards` can remain memory-count based for Memory. The new
Vault API should list active ward directories under `VaultPaths::wards_dir()`.
It must filter reserved/internal entries at the ward root, including `_archive`,
`_curator_backups`, `.usage.json`, hidden entries, and any non-directory entry.
Archived wards can get a later explicit view if users need them, but they should
not appear as normal selectable wards in V1.

Response fields should use ward-relative paths:

```json
{
  "ward_id": "stock-analysis",
  "path": "reports/valuation.md",
  "name": "valuation.md",
  "kind": "file",
  "extension": "md",
  "size": 12345,
  "modified_at": "2026-06-14T12:00:00Z",
  "previewable": true
}
```

The file endpoint should return content and metadata for text-like files, and
raw bytes with a content type for Office Open XML previews when the UI needs an
`ArrayBuffer`. V1 should cap text/code/markdown file reads at 2 MiB and Office
Open XML reads at 15 MiB compressed input. Office Open XML preview parsing must
also cap uncompressed XML work: at most 256 zip entries, 25 MiB total
uncompressed XML, 300 slides for `.pptx`, and 200,000 extracted text characters.
Directory listings should cap returned children at 1,000 entries per directory
and return a `truncated: true` flag when the cap is hit. Oversized files,
parser-limit hits, and truncated directories need clear UI states.

The search endpoint should fuzzy-match visible allowlisted files by ward-relative
path and basename, not file contents. It must reuse the same hidden/env/config,
dependency/cache, symlink, and extension filters as the tree endpoint. V1 should
default to 30 results, cap requested results at 50, scan no more than 20,000
filesystem entries per request, and return `truncated: true` when the scan cap
or result cap is hit.

Even though vault content is user-owned, the handler should still bound both the
ward id and requested file path. The backend must treat `ward_id` as untrusted:
resolve the candidate ward root from `VaultPaths::wards_dir()`, reject ids with
path separators or dot segments, canonicalize the candidate root, and assert the
canonical root remains an actual child directory of the canonical wards
directory. After the ward root is proven safe, normalize and canonicalize the
requested relative path, reject paths that escape that ward root, and apply the
shared exclude policy before reading or returning metadata. This keeps the API
contract bounded and makes tests simple.

Backend tests should cover allowed local requests, denied remote/non-loopback
requests, traversal attempts, excluded paths, mixed-case excludes, non-allowlisted
extensions, oversized files, Office Open XML parser-limit hits, and truncated
directories.

### Frontend

Use the existing shell and theme conventions:

- `VaultPage` owns route-level state.
- `WardList` lists filesystem wards.
- `WardTree` renders the expandable directory tree.
- `VaultSearchBox` searches visible files in the selected ward and opens matches
  through the same preview flow as tree clicks.
- `FilePreviewPane` renders selected file content.
- CSS classes live in feature CSS or component CSS and use tokens from
  `theme.css`.
- Use `lucide-react` icons for folder, file, markdown/text/code/document, and
  chevron disclosure controls.

The preview pane should reuse existing rendering pieces where practical:

- Shared Markdown renderer for `.md`.
- Existing artifact preview logic for `.docx` and `.pptx`.
- Existing code/text preview styling from artifact slide-outs where practical.
- `.html` preview in a sandboxed iframe without script permissions.

The left explorer should be collapsible from the Vault header/sidebar so the
preview pane can use the full workspace. Collapsing the explorer must not clear
the selected ward, search query, selected file, or preview state.

### Editing follow-on

Editing is deliberately not part of V1. The RFC reserves a follow-on design for
an explicit edit mode with:

- read-only default,
- visible dirty state,
- save/revert controls,
- conflict detection when a file changes on disk,
- extension and size allowlists,
- and no writes to excluded secret/config paths.

## Options considered

Axis: where the ward filesystem is surfaced. These options are collectively
exhaustive for V1 because the system can either not expose it, expose it through
an existing surface, shell out to the OS, or add a first-party Vault surface.

| Option | Trade-off |
| --- | --- |
| Do nothing | No implementation cost, but users still cannot inspect ward files in-app. |
| Keep only native folder open | Uses the existing `/api/wards/:ward_id/open` behavior, but context switches out of the dashboard and does not support preview, breadcrumbs, or route actions. |
| Extend Memory | Reuses a ward rail, but conflates durable memory facts/wiki with raw filesystem content and makes Memory harder to reason about. |
| Add top-level Vault tab | Clear information architecture, matches the user's mental model, and leaves Memory focused on indexed knowledge. This is the recommended option. |

Axis: backend path contract. These options are collectively exhaustive because
the UI can receive absolute paths, opaque file ids, or ward-relative paths.

| Option | Trade-off |
| --- | --- |
| Absolute host paths | Simple and acceptable for a user-owned desktop vault, but leaks implementation details and makes path traversal tests harder. |
| Opaque ids only | Strong encapsulation, but requires indexing state for ordinary filesystem browsing and adds complexity before there is a need. |
| Ward-relative paths | Bounded, testable, and consistent with durable ward memory route-hint direction. This is the recommended option. |

Axis: mutability. These options exhaust the V1 choice because the page can be
read-only, fully editable, or explicitly staged for later edit mode.

| Option | Trade-off |
| --- | --- |
| Read-only V1 | Lowest risk and matches preview-first needs. This is the recommended option. |
| Editable V1 | Useful, but creates conflict handling, secret/config write blocking, and accidental mutation risks immediately. |
| No preview, tree only | Easier, but misses the main workflow: click a file and inspect it. |

## Risks & what would make this wrong

- The tree can become slow on large wards. Mitigation: lazy-load directories,
  cap directory entries, cap fuzzy-search scans, and exclude common
  generated/dependency folders.
- Secret or tweakable config files can appear in the tree. Mitigation: default
  excludes for `.env*`, `*.env`, `.venv/`, and tweakable YAML config names.
- The UI can drift from the existing app theme. Mitigation: use `theme.css`
  tokens, existing component class conventions, and lucide icons.
- Preview can imply full-fidelity Office rendering. Mitigation: label `.docx`
  and `.pptx` previews as text-oriented; keep `.doc` and `.ppt` as metadata/open
  externally in V1.
- Path handling can accidentally escape a ward root. Mitigation: use
  ward-relative inputs, canonicalize against `VaultPaths::ward_dir`, reject
  escapes, and test traversal attempts.
- Raw file APIs can expose ward content to another device if treated like a
  normal LAN dashboard endpoint. Mitigation: make `/api/vault/*` local-only in V1
  and require explicit authenticated design before remote access.

This RFC would be wrong if users mainly want write/edit workflows in the first
slice, or if real wards are small enough that a native folder button already
solves the problem. Current user direction asks for an Obsidian-style in-app
tree and preview, so read-only Vault browsing is the right first slice.

## Evidence & prior art

Repo evidence:

- `docs/specs/durable-ward-memory/spec.md` says ward directories are durable
  source workspaces and should not be replaced by summaries.
- `docs/specs/durable-ward-memory/spec.md` lists future UI route actions such
  as open ward, open artifact, inspect source, and resume execution.
- `apps/ui/ARCHITECTURE.md` requires theme changes to use design tokens and
  existing component class conventions.
- `apps/ui/ARCHITECTURE.md` documents Memory as a command deck over ward facts,
  wiki, procedures, and episodes, not raw filesystem browsing.
- `gateway/src/http/ward_content.rs` already exposes ward memory aggregation
  through `/api/wards/:ward_id/content`.
- `gateway/src/http/ward_actions.rs` already resolves a selected ward through
  `VaultPaths::ward_dir` for native folder opening.
- `apps/ui/src/features/chat/ArtifactSlideOut.tsx` already previews markdown,
  text, code, sandboxed HTML, `.docx`, `.xlsx`, and `.pptx` artifact content.
- A read-only local probe on 2026-06-14 confirmed live wards under
  `~/Documents/zbot/wards` contain nested directories and mixed `.md`, `.py`,
  `.yaml`, generated output, and hidden/dependency content.

External prior art:

- [Obsidian File explorer](https://obsidian.md/help/plugins/file-explorer)
  provides a core vault file/folder browsing model for notes and accepted file
  formats.
- [Visual Studio Code User Interface](https://code.visualstudio.com/docs/getstarted/userinterface)
  documents the Explorer as a file/folder tree with filtering support.
- [Visual Studio Code Code Navigation](https://code.visualstudio.com/docs/editing/editingevolved)
  documents breadcrumbs as a navigable path affordance.
- [OWASP Path Traversal](https://owasp.org/www-community/attacks/Path_Traversal)
  documents the general risk of user-controlled paths reaching unintended files.
- [PortSwigger Web Security Academy](https://portswigger.net/web-security/file-path-traversal)
  recommends validating inputs, canonicalizing paths, and checking resolved
  paths stay under the expected base directory.

Spike / de-risk result:

- Codemem and grep found existing route-hint and ward-relative path precedent,
  but no raw ward filesystem tree endpoint.
- The UI already has a top-level nav shell, shared markdown renderer, lucide
  icons, and artifact preview code that can be reused.
- The backend already has `VaultPaths::wards_dir()` and `VaultPaths::ward_dir()`
  as the correct root for a bounded ward filesystem API.
- Live wards are nested and can include large generated folders, so the tree
  must be lazy and filtered by default.

## Open questions

None for V1. Editing, hidden-file opt-in, and richer Office/PDF conversion
should be handled by follow-on specs after read-only Vault browsing lands.

## Follow-on artifacts

- Spec: `docs/specs/vault-ward-browser/`
- UI architecture update: `apps/ui/ARCHITECTURE.md`
- Optional ADR if the project wants to record the new `/api/vault/*`
  filesystem API boundary as a stable backend convention.
