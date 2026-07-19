# RFC-0007: Ward-Specific Vault Explorer in Research

- **Status:** Open
- **Author:** phanijapps
- **Approver:** phanijapps
- **Date opened:** 2026-06-14
- **Date closed:**
- **Related:** [RFC-0006: Vault Obsidian-Style Ward Browser](0006-vault-obsidian-style-ward-browser.md); `docs/specs/vault-ward-browser/`; `docs/specs/durable-ward-memory/`; `apps/ui/src/features/research-v2/ResearchPage.tsx`; `apps/ui/src/features/vault/VaultPage.tsx`; `gateway/src/http/vault.rs`

## The ask

Approve a ward-scoped Vault explorer inside the Research page once a research
session has an active ward. The recommended V1 is a two-column research layout:
left column is a read-only file explorer/search scoped to the session ward, and
right column is the existing Research UI.

z-Bot already creates or selects a ward during research intent analysis, then
redirects the user to `/research/:sessionId`. The complication is that the user
cannot inspect the ward's files while reading the live research session without
leaving Research for `/vault?ward=<ward>`. The question is whether Research
should embed the same ward filesystem surface directly.

Decisions requested:

1. Add a Research-local ward explorer after `state.wardId` is known.
   Recommended: accept, because the ward is the research workspace and should be
   visible beside the session output. Default if no objection by 2026-06-21:
   embed it.
2. Use the existing `/api/vault/*` endpoints and typed transport calls as the
   first implementation path. Recommended: accept, because the Vault API already
   enforces local-only, ward-relative, read-only filesystem access. Default if
   no objection by 2026-06-21: reuse it, with permission for a small adapter or
   narrow new API only if extraction proves awkward.
3. Clicked files should open in a slide-out preview rather than replacing the
   research transcript. Recommended: accept, because Research remains the primary
   work surface and file inspection is contextual. Default if no objection by
   2026-06-21: slide-out preview.
4. Bind the explorer to the session/root ward, not arbitrary child-agent ward
   changes. Recommended: accept, because the existing Research state tracks the
   root/session ward. Default if no objection by 2026-06-21: one active ward per
   research session.

## Problem & goals

Research sessions and wards are linked, but their UI surfaces are separate:

- Research shows the live transcript, subagent turns, artifacts, intent, and the
  current ward chip.
- Vault shows the real ward filesystem tree, fuzzy file search, and read-only
  file previews.
- The ward chip currently navigates away to `/vault?ward=<ward_id>`.

This creates a context switch at the exact moment the ward becomes useful:
after intent analysis creates/selects a ward and the session redirects to
`/research/:sessionId`.

Goals:

- Journey 1: when new research starts, intent analysis creates/selects a ward,
  the URL redirects to `/research/:sessionId`, and the ward explorer appears as
  soon as the ward is known.
- Journey 2: when opening an existing research session with a ward, the explorer
  appears from snapshot-hydrated ward state.
- Show a two-column Research page after ward creation: ward file explorer/search
  on the left, existing Research UI on the right.
- Scope tree and fuzzy search strictly to the active ward.
- Reuse existing Vault allowlists, excludes, path validation, preview behavior,
  and local-only file access.
- Open clicked files in a contextual slide-out so the transcript stays visible.
- Keep the current top-level `/vault` route and ward chip navigation as a full
  Vault affordance.

Non-goals:

- No writable file editor in Research V1.
- No separate multi-ward research workspace in V1.
- No remote/LAN Vault access.
- No change to intent analysis ward selection, ward creation, or ward-agent
  routing behavior.
- No replacement of artifact chips or the existing `/vault` tab.

## Proposal

Add a reusable ward-scoped explorer component extracted from the existing Vault
page behavior, then render it inside Research when `state.wardId` is non-null.

Recommended component boundary:

```text
apps/ui/src/features/vault/
  WardVaultExplorer.tsx      # reusable scoped tree + fuzzy search
  VaultFilePreview.tsx       # reusable read-only preview renderer
  VaultFileSlideOut.tsx      # research contextual preview shell
  VaultPage.tsx              # route wrapper around reusable pieces
```

The exact names can change during implementation, but the important boundary is
that `VaultPage` should stop being the only owner of tree/search/preview logic.
It can remain the route-level page for `Vault > Wards > <ward>`, while Research
uses the ward-scoped explorer directly.

Research layout after ward creation:

```text
ResearchHeader
StatusPill

┌──────────────────────────────┬────────────────────────────────────────────┐
│ Ward Vault Explorer           │ Existing Research transcript/composer      │
│ - ward name                   │ - intent line                              │
│ - file fuzzy search           │ - session turns                            │
│ - expandable tree             │ - artifact strip                           │
│ - loading/error/empty states  │ - composer                                 │
└──────────────────────────────┴────────────────────────────────────────────┘

VaultFileSlideOut (only when a file is selected)
```

Journey 1:

1. User starts research from `/research`.
2. `sendMessage()` subscribes, invokes the root agent, and the session id lands.
3. Existing URL sync redirects to `/research/:sessionId`.
4. Intent analysis / ward tool produces `ward_changed`.
5. Research reducer stores `state.wardId` and `state.wardName`.
6. Research renders the left explorer for that ward and loads the ward root tree.
7. User searches or expands directories; clicking a file opens the slide-out.

Journey 2:

1. User opens `/research/:sessionId`.
2. `snapshotSession()` calls `/api/sessions/:id/state`.
3. The snapshot maps `state.ward.name` into `wardId` and `wardName`.
4. Research renders the same left explorer and slide-out behavior.

API approach:

- Use existing transport methods first:
  - `listVaultWards()`
  - `getVaultTree(wardId, path)`
  - `searchVaultFiles(wardId, query, limit)`
  - `getVaultFile(wardId, path)`
- Do not add a new backend endpoint for V1 unless extraction reveals a concrete
  need. Acceptable small additions include:
  - a typed response adapter for a Research-specific slide-out if current Vault
    file payloads are too route-coupled;
  - an optional `GET /api/vault/wards/:ward_id/root` convenience endpoint if the
    UI otherwise has to duplicate too much `list + select` setup.
- Any new endpoint must preserve RFC-0006 rules: local-only, read-only,
  ward-relative paths, same allowlist/exclude policy, and no absolute host paths.

File click behavior:

- Tree/search result click calls `getVaultFile`.
- `.md` uses the shared Markdown renderer.
- text/code/YAML/JSON/TOML render as escaped read-only source.
- `.html` renders in the same sandbox posture as Vault.
- `.docx` / `.pptx` use the existing Office preview helper.
- `.doc` / `.ppt` remain metadata/non-previewable with an open-ward-folder
  affordance.

State rules:

- The explorer is visible only when `state.wardId` is set.
- The explorer resets when the session ward changes.
- The explorer should not show sibling wards.
- Search is scoped to the selected ward only.
- Existing top-level Vault navigation remains available for full-page browsing.

## Options considered

Axis: where to surface ward files during research. These options are
collectively exhaustive for V1 because Research can either not show files,
link away, embed a scoped explorer, or become a full Vault/Research hybrid.

| Option | Trade-off |
| --- | --- |
| Do nothing | No implementation cost, but users still lose the ward context during live research. |
| Keep only the `/vault?ward=` link | Reuses shipped Vault completely, but forces a route change away from the transcript. |
| Embed a scoped Research explorer | Keeps files and transcript together while reusing the Vault backend. Recommended. |
| Merge Research and Vault fully | Powerful long term, but too broad for V1 and risks muddying both surfaces. |

Axis: API contract. These options are collectively exhaustive because Research
can reuse the existing Vault API, add a thin adapter, or add a separate research
filesystem API.

| Option | Trade-off |
| --- | --- |
| Reuse `/api/vault/*` directly | Lowest backend risk and matches shipped constraints. Recommended default. |
| Add a thin adapter endpoint | Acceptable if it removes frontend duplication without changing security/path semantics. |
| Add separate Research file APIs | More explicit, but duplicates policy and increases test surface. Avoid unless reuse fails. |

Axis: preview placement. These options exhaust V1 because file content can
replace the transcript, appear inline, open in contextual overlay, or navigate
to Vault.

| Option | Trade-off |
| --- | --- |
| Replace Research content | Simple, but hides the live transcript and composer. |
| Inline preview in the left column | Keeps context, but the explorer column becomes too cramped for real documents. |
| Slide-out preview | Matches existing artifact behavior and preserves the transcript. Recommended. |
| Navigate to Vault | Already exists, but is the context switch this RFC is trying to remove. |

Axis: ward binding. These options are collectively exhaustive for current
Research state because there is one session/root ward, no ward, or potentially
multiple child-agent wards.

| Option | Trade-off |
| --- | --- |
| No ward -> no explorer | Correct for landing/simple sessions without ward state. |
| Session/root ward -> one explorer | Matches current state and reopen behavior. Recommended. |
| Multi-ward child-agent explorer | Potential future need, but current state does not track this as active UI context. |

## Risks & what would make this wrong

- `VaultPage` extraction could become larger than expected because it currently
  owns route params, tree state, search state, resizing, preview loading, and
  preview rendering. Mitigation: extract only the ward-scoped explorer first and
  leave route breadcrumbs/root navigation in `VaultPage`.
- The two-column layout could crowd the existing Research transcript on small
  screens. Mitigation: collapse the explorer by default below a breakpoint or
  expose an explicit panel toggle.
- A slide-out preview could duplicate artifact slide-out behavior. Mitigation:
  reuse preview/rendering helpers where possible; keep `ArtifactSlideOut`
  artifact-specific.
- The explorer could appear too late in Journey 1 if `ward_changed` lands after
  a delay. Mitigation: render a reserved "Waiting for ward" state during
  `intentAnalyzing` and replace it when `wardId` lands.
- A new endpoint could accidentally fork Vault security policy. Mitigation: only
  allow a small adapter when direct reuse fails, and require tests proving the
  same path/exclude/local-only rules.
- Users may expect editing because the pane feels like a mini IDE. Mitigation:
  label V1 as preview/read-only and keep editing out of scope.

Key assumptions:

- `state.wardId` is the correct V1 binding for the Research explorer.
- `/api/vault/*` remains the canonical file tree/search/read boundary.
- The file preview surface can be extracted without changing backend payloads.
- One active ward per research session is acceptable for V1.

This RFC would be wrong if Research sessions commonly use multiple wards in one
turn and users need to switch between them, or if preview-in-slide-out proves
too disconnected from the explorer compared with an inline editor pane.

## Evidence & prior art

Spike / de-risk result:

- Codemem confirmed `ResearchSessionState` already has sticky `wardId` and
  `wardName` fields in `apps/ui/src/features/research-v2/types.ts`.
- Codemem confirmed `snapshotSession()` already calls
  `transport.getSessionState(sessionId)` and maps `ward.name` into `wardId` and
  `wardName` in `apps/ui/src/features/research-v2/session-snapshot.ts`.
- Direct code inspection confirmed live `ward_changed` maps to `WARD_CHANGED`
  in `event-map.ts`, and the reducer stores `wardId` / `wardName`.
- Direct code inspection confirmed the backend persists ward changes through
  `stream_event_processor.rs`, so reopened sessions can recover the ward.
- Codemem and direct reads confirmed transport already exposes the Vault API
  methods in `apps/ui/src/services/transport/http.ts` and
  `interface.ts`.
- Focused tests passed on 2026-06-14:
  `npm --prefix apps/ui run test -- src/services/transport/http.class.test.ts src/features/research-v2/event-map.test.ts src/features/research-v2/ResearchPage.test.tsx src/features/vault/VaultPage.test.tsx`
  with 117 passing tests. The run emitted existing React `act(...)` and MSW
  artifact-fetch warnings in Research tests, but no failures.

Repo precedent:

- [RFC-0006](0006-vault-obsidian-style-ward-browser.md) proposed the shape of a
  read-only Vault browser with ward-relative APIs, fuzzy file search, and safe
  preview behavior.
- `docs/specs/vault-ward-browser/spec.md` records the shipped constraints:
  read-only V1, local-only `/api/vault/*`, path validation, excludes, file
  allowlist, and search limits.
- `docs/specs/durable-ward-memory/spec.md` treats wards as durable executable
  source workspaces and asks future UI route actions to preserve ward/file
  pointers.
- `apps/ui/src/features/research-v2/ResearchPage.tsx` already shows the active
  ward chip and navigates it to `/vault?ward=<wardId>`.

External prior art:

- VS Code's Explorer is a file/folder view beside editor content; its UI guide
  describes browsing project files in the Explorer and opening content in editor
  regions. This supports the two-column explorer/work-surface model:
  <https://code.visualstudio.com/docs/editing/userinterface>.
- VS Code supports filtering/fuzzy matching inside tree views, which supports a
  search control directly in the explorer column:
  <https://code.visualstudio.com/docs/editing/userinterface>.
- VS Code preview mode opens a selected file in a reusable preview tab, which
  supports non-destructive click-to-preview rather than route navigation:
  <https://code.visualstudio.com/docs/editing/userinterface>.
- Obsidian community discussion includes a three-pane file explorer with note
  previews, supporting the requested Obsidian-style "folder/list/content"
  workflow: <https://forum.obsidian.md/t/plugin-for-3-pane-file-explorer-with-note-previews/40361>.

## Open questions

1. Should the Research explorer be expanded by default on desktop once a ward
   exists? Recommended default: yes on desktop, collapsed/toggleable on narrow
   screens. Owner: phanijapps. Decide-by: implementation spec.
## Follow-on artifacts

- Spec: `docs/specs/ward-vault-in-research/`
- Possible UI architecture update if Vault preview/explorer components become
  reusable shared feature components.
