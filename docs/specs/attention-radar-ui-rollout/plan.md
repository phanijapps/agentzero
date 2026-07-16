# Plan: Attention Radar UI Rollout

- **Status:** Executing
- **Spec:** [`spec.md`](spec.md)
- **Reference:** [`Mission Control Attention Radar mockup`](../../product/mission-control-attention-radar-mockup.html)
- **Gallery:** [`Attention Radar UI gallery`](../../product/attention-radar-ui-gallery.html)

## Design (LLD)

### Design decisions

- Keep the current flat top-bar navigation and canonical URLs. The visual system
  changes the shell's character and hierarchy, not the navigation contract.
- Establish Attention Radar as a layer over existing semantic tokens and BEM
  component classes. Page components compose workbench primitives; they do not
  add page-local palettes or one-off layout rules where a shared primitive is
  appropriate.
- Treat Mission Control as the reference composition, not a template to copy
  literally. A graph needs a dominant canvas, a file browser needs a preview,
  and a form needs a focused configuration deck; all retain the same signal,
  panel, and inspector vocabulary.
- Retain configurable accent color. Primary/active treatment derives from the
  existing semantic accent token; fixed cyan/violet in the gallery describes
  hierarchy, not a hard-coded production preference.
- Use independent scrolling only where a desktop workbench has more than one
  dense pane. Ensure the containing flex/grid chain has `min-height: 0`; on
  narrow screens revert to a single readable document flow.

### Component / module decomposition

Production implementation will introduce or consolidate these presentation-only
primitives under `apps/ui/src/styles/` and shared UI, then apply them in route
cohorts:

```text
Attention Radar shell
├── route masthead (eyebrow, title, status, local controls)
├── signal strip (optional, decision-oriented metrics)
├── workbench frame
│   ├── context rail / radar
│   ├── focus canvas
│   └── inspector / operations rail
├── panel header + panel body + pane-scroll
└── signal state (icon + text + semantic treatment)
```

The gallery is a non-runtime design artifact. It deliberately uses static
sample content and does not add a product route, component, API, or live event
transport.

### State & control flow

Each production route preserves its existing data hook and mutations. The
visual layer maps current state into one of five visible patterns: `loading`,
`empty`, `normal`, `selected/active`, and `attention/error`. Existing controls
remain connected to the same events. A rail selection may change local focus,
but no mock requires new server data.

### Behavior & rules

- A page has one primary task and one visual focus. Secondary information moves
  into a context rail or inspector rather than competing as another headline.
- A metric strip is permitted only when its values support an available action;
  configuration and onboarding should prefer progress and validation states.
- An inspector is selected by an existing item, not duplicated static content.
  When no selection exists it presents the route's meaningful empty state.
- Panel headings identify content and scope; tabs name a local view; route
  navigation remains in the global top bar.

### Quality attributes (NFRs)

- **Accessibility:** preserve accessible labels and keyboard paths; focus
  outline remains visible; semantic status never relies only on hue (spec AC 6).
- **Responsive behavior:** verify 1440px, 1024px, and 390px layouts, with no
  unintended horizontal overflow and no inaccessible pane content (spec AC 7).
- **Operability:** user-facing active, waiting, completed, and failed state
  labels remain readable in dense views (spec AC 6).

## Tasks

### T1: Publish the canonical visual contract and route matrix

**Depends on:** none

**Touches:** `docs/product/attention-radar-ui-gallery.html`, `docs/specs/attention-radar-ui-rollout/*.md`

**Tests:**
- Visual/manual QA of the gallery at desktop, tablet, and narrow widths against
  the shared rules and all eleven screen rows in `spec.md` (AC 1, AC 2).
- Visual/manual QA: every row in the spec's screen state and navigation matrix
  identifies preserved normal, selected/active, empty, loading, and error
  behavior before that route's production cohort starts (AC 3).
- Goal-based check: direct session URLs and every current legacy alias resolve
  to their existing target before and after its affected cohort (AC 3).
- Goal-based check: `git diff --check` has no whitespace errors.

**Approach:**
- Create one navigable static gallery with full-screen mockups for the
  standalone Commissioning screen and ten canonical in-shell routes.
- Include a route-to-composition matrix and visible shared visual rules so
  later implementers do not infer behavior from color and spacing alone.
- Add the screen state and navigation matrix to `spec.md`; update Quiet
  Instrument to name this spec as its successor visual contract while retaining
  its behavioral and accessibility boundaries.

**Done when:** the gallery is reviewable locally, every screen contract in
`spec.md` has one matching mock, and the successor/state/deep-link rules are
written before production cohort work starts.

### T2: Establish production Attention Radar primitives without changing behavior

**Status:** Complete (2026-07-15)

**Depends on:** T1

**Touches:** `apps/ui/src/styles/theme.css`, `apps/ui/src/styles/components.css`, `apps/ui/src/App.tsx`, `apps/ui/src/**/*.css`

**Tests:**
- Goal-based check: affected shared-component tests and `npm run build` pass
  (AC 4, AC 8).
- Visual/manual QA: top bar, route masthead, panel, selection, button, status,
  empty, and independent-scroll patterns match the gallery at three widths
  (AC 2, AC 5, AC 6, AC 7).

**Approach:**
- Evolve semantic theme tokens and add reusable BEM primitives for the
  workbench frame, rails, panel header/body, signal state, and local controls.
- Keep the existing global navigation order, accent picker, and route wiring.
- Remove only superseded visual CSS after confirming no unrelated route consumes
  it.

**Done when:** a route can compose the Attention Radar primitives without
inline visual decisions or a page-local palette.

### T3: Restyle conversation and operations workbenches

**Status:** Executing — Research complete; Quick Chat and Mission Control pending.

**Depends on:** T2, spec:mission-control-attention-radar/T3

**Touches:** `apps/ui/src/features/research-v2/*`, `apps/ui/src/features/chat-v2/*`, `apps/ui/src/features/mission-control/*`

**Tests:**
- TDD: preserve selected research session, intent visibility, active AG-UI
  surface, artifact strip/preview, and Quick Chat deliverable behavior in
  existing/updated Vitest tests (AC 4).
- TDD: Research without a bound ward does not mount or fetch the explorer;
  Research with a bound ward renders only that ward's filesystem, preserves
  file preview, and keeps artifact preview available (AC 4).
- Visual/manual QA: Research's filesystem/thread arrangement has no generic
  metric strip or duplicate Vault/deliverable control; Quick Chat's focus
  conversation and Mission Control's radar/inspector panes match their mocks
  at the defined breakpoints (AC 4–7).
- Goal-based check: focused tests and `npm run build` pass (AC 8).

**Approach:**
- Apply shared workbench primitives while preserving existing hooks, reducers,
  endpoints, and Mission Control's bounded data model.
- Start with the independent Research slice: when a session has a bound ward,
  use its existing `WardVaultExplorer` as the left context rail. Do not add a
  generic metric strip, a fake session radar, invented counters, a duplicate
  Vault action, or a second deliverables surface. Keep AG-UI plans and tool
  activity in the research focus canvas and preserve the one real artifact
  strip, its `Open artifact …` controls, and its preview path.
- Quick Chat follows the same shared vocabulary without turning a direct
  conversation into a dashboard. Mission Control remains gated on its bounded
  WebSocket state, reducer, and live/degraded states from the Attention Radar
  spec; this rollout does not create a second subscription or state path.

**Done when:** the three agent-facing surfaces use one status and inspector
vocabulary without a behavioral regression.

### T4: Restyle memory, graph, and vault workbenches

**Depends on:** T2

**Touches:** `apps/ui/src/features/memory/*`, `apps/ui/src/features/observatory/*`, `apps/ui/src/features/observatory-v2/*`, `apps/ui/src/features/vault/*`

**Tests:**
- TDD: preserve memory tab/scope selection, graph entity selection/zoom, and
  vault file selection/preview behavior (AC 4).
- Visual/manual QA: recall, graph, and preview canvases retain usable rails,
  inspector states, and narrow layouts (AC 4–7).
- Goal-based check: affected tests and `npm run build` pass (AC 8).

**Approach:**
- Give each knowledge surface its appropriate dominant canvas while unifying
  rail headers, search/filters, provenance, selection, and empty/error states.
- Preserve the separation between Observatory and Graph routes; they have
  different information jobs.

**Done when:** a user can recognize the shared workbench language without
losing existing semantic recall, graph, or file browsing capability.

### T5: Restyle management and configuration workbenches

**Depends on:** T2

**Touches:** `apps/ui/src/features/agent/*`, `apps/ui/src/features/integrations/*`, `apps/ui/src/features/settings/*`

**Tests:**
- TDD: preserve tabs, provider editing, tool-server/plugin actions, agent and
  schedule selection, validation, and save paths (AC 4).
- Visual/manual QA: roster/detail/posture panels make ownership, connectivity,
  validation, and destructive actions explicit (AC 4–7).
- Goal-based check: affected tests and `npm run build` pass (AC 8).

**Approach:**
- Convert dense card grids and stacked settings into focused workbenches while
  retaining every current input, action, and detail slide-over.
- Use posture rails for permission/connection/validation data rather than
inventing telemetry APIs.

**Done when:** configuration routes feel operationally coherent and all
existing administrative actions remain reachable.

### T6: Align standalone Commissioning with the workbench language

**Depends on:** T2

**Touches:** `apps/ui/src/features/commissioning/*`

**Tests:**
- TDD: existing required focus/domain, provider/model/API key, and personal
  profile validation remain unchanged (AC 4).
- Visual/manual QA: each onboarding step presents the correct focused decision,
  progress state, local-data reassurance, and narrow-screen order (AC 4–7).
- Goal-based check: commissioning tests and `npm run build` pass (AC 8).

**Approach:**
- Apply the same panel/type/signal vocabulary without imposing in-shell
  navigation on the standalone first-run flow.
- Keep the existing autonomous-agent safety warning and local-data explanation
  adjacent to the user decisions they inform.

**Done when:** first-run users see a calm, consistent introduction without any
loss of commissioning validation or data handling.

### T7: Run cross-route regression and close the visual rollout

**Depends on:** T3, T4, T5, T6

**Touches:** `apps/ui/src/**/*.test.tsx`, `apps/ui/src/**/*.css`, `docs/specs/attention-radar-ui-rollout/*`

**Tests:**
- Goal-based check: `npm test -- --run`, `npm run build`, and scoped lint
  outcomes are recorded; any pre-existing lint failures are isolated from this
  change (AC 8).
- Visual/manual QA: traverse every canonical route and its normal, selected,
  empty, loading, and error states at 1440px, 1024px, and 390px (AC 4–7).
- Goal-based check: navigate `/research/:sessionId` and each legacy alias named
  in the screen state and navigation matrix; verify its current redirect and
  direct-session behavior remains intact (AC 3).
- Manual keyboard check: navigate global and local controls, then focus each
  independently scrollable pane (AC 5, AC 6).

**Approach:**
- Compare the implemented system to the gallery, fix route drift, and update
the gallery/spec when an implementation constraint changes the intended design.
- Remove migration CSS only after a route-by-route consumer audit.

**Done when:** all unchecked acceptance criteria are checked with recorded
verification and the spec is ready to move to Shipped.

## Rollout

Delivery is route-cohort based, beginning with shared primitives. Each cohort
is independently reversible because it changes only client presentation; a
rollback reverts the affected CSS/component composition without data migration
or service sequencing. No external system, schema, event, or provider change is
required. The gallery remains the design reference until the production visual
QA pass replaces it as current reality.

## Risks

- A global token adjustment can unintentionally change a legacy route. Mitigate
with a shared-primitive baseline and targeted route tests before removing CSS.
- Dense multi-pane layouts can trap content when an ancestor lacks `min-height:
0`. Mitigate with the desktop/narrow viewport matrix and explicit pane-scroll
testing.
- A beautiful static workbench can imply capabilities the backend does not
provide. Mitigate by labeling mock data as illustrative and preserving existing
transport-driven states in implementation.
- The active Quiet Instrument spec overlaps this work. Mitigate by treating
this spec as the visual decision authority and updating both documents if
implementation reveals a material conflict.

## Changelog

- 2026-07-15: Initial design plan and mock-gallery contract authored.
- 2026-07-15: Completed T1 visual contract; started T2 with shared semantic
  tokens, shell styling, and reusable Attention Radar workbench primitives.
- 2026-07-15: Completed T2: production theme and workbench primitives now
  preserve the configurable accent, use responsive normal document flow below
  desktop, and pass build, test, viewport, and follow-up review checks.
- 2026-07-15: Revised the Research visual contract: remove the generic metric
  strip and use the real bound-ward filesystem explorer as its left rail.
- 2026-07-15: Completed the Research slice of T3: a bound ward is the sole
  left context rail, the live thread remains the focus pane, and narrow
  collapsed-explorer flow keeps the thread, artifacts, and composer reachable.
