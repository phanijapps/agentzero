# Spec: Attention Radar UI Rollout

- **Status:** Implementing
- **Owner:** zbot maintainers
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none
- **Shape:** ui

> **Spec contract:** this document defines what "done" means. The implementing PR must match this spec, or update it.

## Objective

Turn the Mission Control Attention Radar visual language into z-Bot's coherent
desktop workbench system. A user moving among commissioning, Research, Quick
Chat, Mission Control, Agents, Memory, Vault, Observatory, Graph,
Integrations, and Settings will recognize the same dark operational canvas,
high-signal type hierarchy, status vocabulary, bounded panels, and focused
work area—while each page keeps the workflow it already owns. The static
[mock gallery](../../product/attention-radar-ui-gallery.html) is the approved
visual contract before production routes are restyled.

This refines the active [Quiet Instrument UI](../quiet-instrument-ui/spec.md)
work: where its earlier visual direction conflicts with the user-approved
Attention Radar reference, Attention Radar wins. Its preservation of behavior,
routes, accessibility, and public contracts remains in force.

## Boundaries

### Always do

- Preserve every current route, route alias, transport call, session state,
  mutation, keyboard behavior, and accessible name while changing presentation.
- Use one semantic token layer and shared BEM-style workbench primitives for
  chrome, panels, status, controls, empty states, and independent scroll areas.
- Implement a route only after its gallery mock and screen-state checklist have
  been reviewed; retain the same information and action affordances that exist
  on that route today.
- Make the desktop, tablet, and narrow-screen layouts usable: content columns
  stack deliberately, each desktop pane owns its own vertical scrolling, and
  there is no accidental horizontal page overflow.

### Ask first

- Adding a UI dependency, web font delivery dependency, new route, or a change
  to the top-level navigation information architecture.
- Changing a user workflow, exposing a new agent capability, or altering a
  transport/API contract to make a mock look possible.
- Replacing a currently working graph, artifact, editor, or virtualization
  interaction rather than reskinning and composing it.

### Never do

- Do not modify gateway APIs, persistence, session/goal lifecycle, agent
  execution, memory semantics, or tool permissions for this visual rollout.
- Do not create a second component library, page-local token system, or new
  top-level application shell.
- Do not turn operational telemetry into decorative noise: color, glow, motion,
  or a badge must communicate real state and must retain a text or icon cue.
- Do not use a static mock as evidence that a live data feed or action exists.

## Testing Strategy

- **Visual/manual QA:** every route's normal, empty, loading, error, and
  active/selected state will be exercised at desktop (1440px), tablet (1024px),
  and narrow (390px) widths. The result is judged against the gallery because
  composition, readable hierarchy, and pane ownership are visual outcomes.
- **TDD:** retain or add targeted React/Vitest tests when restructuring can
  change a route's selected item, tab, session, artifact, or form behavior.
  Those controls have compressible state invariants independent of styling.
- **Goal-based checks:** run affected UI tests, `npm run build`, and the scoped
  lint command for each route cohort. These prove the existing route contracts
  still compile and render through the same application shell.

## Acceptance Criteria

- [x] Given a reviewer opens the static gallery, when they select a canonical
  z-Bot surface, they can inspect a Attention Radar mock for Commissioning,
  Research, Quick Chat, Mission Control, Agents, Memory, Vault, Observatory,
  Graph, Integrations, and Settings.
- [x] Given a reviewer compares a mock to the reference, when they inspect the
  shared chrome, panel, type, control, status, and scrollbar treatments, the
  gallery makes the shared visual vocabulary explicit rather than relying on a
  route-by-route interpretation.
- [x] Given an implementer begins a route cohort, when they consult the screen
  state and navigation matrix below, they can identify that route's normal,
  selected/active, empty/loading/error, direct-link, and legacy-alias behavior
  that the visual change must preserve.
- [ ] Given any canonical production route, when it renders, the user sees the
  shared Attention Radar workbench vocabulary while retaining that route's
  existing actions, information, route, and transport behavior.
- [ ] Given a desktop workbench route with several dense regions, when a pane's
  content exceeds its height, that pane scrolls independently and controls are
  not duplicated between its collapsed and expanded states.
- [ ] Given a normal, selected/active, empty, loading, or error state, when it
  is visible on a restyled route, the state is legible from text, iconography,
  and semantic status treatment without relying on color alone.
- [ ] Given 1440px, 1024px, and 390px viewports, when a restyled route loads,
  the primary task remains readable and operable without horizontal page
  overflow or hidden critical actions.
- [ ] Given each implementation cohort, when its targeted tests and
  `npm run build` run, the UI compiles and existing behavior remains valid.

## Assumptions

- Technical: the application exposes ten canonical in-shell routes plus a
  standalone Commissioning route; aliases redirect and do not need separate
  visual designs (source: `apps/ui/src/App.tsx`).
- Technical: React 19, Tailwind 4, Radix primitives, Lucide icons, and a
  semantic CSS-token system are established and sufficient for the rollout
  (source: `apps/ui/package.json`, `apps/ui/src/styles/theme.css`, and
  `apps/ui/ARCHITECTURE.md`).
- Product: the Mission Control Attention Radar mock is the art-direction
  reference and all distinct user-facing surfaces receive mocks before
  production implementation (source: user confirmation 2026-07-15).
- Process: this remains a living spec with a plan and acceptance criteria while
  implementation is active (source: `docs/CONVENTIONS.md` §4).

## Screen contract

| Surface | Primary job | Attention Radar composition |
| --- | --- | --- |
| Commissioning | Personalize a safe, useful first agent | guided progress rail + one focused decision deck + local-data reassurance |
| Research | Run and supervise a multi-step agent goal | ward filesystem rail when a ward is bound + active research thread/AG-UI focus + real goal activity and deliverables; no generic metric strip |
| Quick Chat | Start or continue a direct conversation | quiet conversation canvas + contextual rail + valuable goal deliverables |
| Mission Control | Notice and inspect what needs attention | ranked mission radar + focused inspector + live operations/posture |
| Agents | Configure agents, skills, and schedules | catalog/filters + roster or tabular workbench + selected-agent inspector |
| Memory | Recall and curate durable knowledge | recall command deck + evidence results + scope/write or provenance rail |
| Vault | Browse ward files safely | tree rail + document preview + file metadata/action rail |
| Observatory | Inspect learning and belief relationships | graph canvas + health/filter rail + selected-entity inspector |
| Graph | Navigate ontology/taxonomy hierarchy | hierarchy rail + graph/tree canvas + classification inspector |
| Integrations | Trust and manage external capability | connection roster + integration details + permission/posture rail |
| Settings | Configure providers and agent behavior | settings rail + focused configuration deck + validation/posture rail |

## Shared visual rules

- **Canvas and panels:** deep ink canvas, hairline borders, modest rounded
  panels, and one restrained color wash only behind the current focus—not
  decoration on every card.
- **Type:** compact high-contrast page title; monospace uppercase eyebrow,
  timestamp, count, and operational metadata; readable body copy at normal
  size. Font delivery remains unchanged unless separately approved.
- **Signal:** cyan/primary denotes current work or healthy live activity;
  green success, amber attention, and red failure keep their semantic meaning.
  Every colored status has a word label.
- **Controls:** compact outline controls for secondary actions; a single clear
  primary action per workbench; tabs control a local view, not global routing.
- **Density:** a route can be calm, but it should not leave users hunting for
  the relevant signal. Metric strips appear only when they change the decision
  a user can make.
- **Scrolling:** the application shell does not scroll dense desktop views;
  their explicit content panes do. At tablet/narrow widths, panes become a
  deliberate vertical sequence with normal page scrolling.

## Screen state and navigation matrix

The gallery shows one representative productive state per route. Before a
cohort is implemented, its implementer records a visual/manual verification
against every state named below. “Preserve” means the existing hook, reducer,
mutation, screen-reader name, and route behavior stay intact; it does not mean
the old visual hierarchy stays intact.

| Surface | Productive and selected state | Empty, loading, and error states to preserve | Direct route and alias proof |
| --- | --- | --- | --- |
| Commissioning | current step, chosen focus/provider/interests, invalid required input | local diagnosis loading/failure; submission failure | `/commission`; `/setup` redirects to it |
| Research | current session, bound-ward filesystem, intent analysis, active plan/AG-UI, artifact, selected session | no sessions; no bound ward; initial/session snapshot loading; request/stream error | `/research`, `/research/:sessionId`; `/research-v2` and `/research-v2/:sessionId` redirect correctly |
| Quick Chat | messages, activity chip, ward context, goal deliverable | new empty conversation; send/stream error | `/chat`; `/chat-v2` redirects correctly |
| Mission Control | ranked mission, selected inspector, live/degraded status | no sessions; bounded-summary loading/failure; selected-detail loading/failure | `/mission-control`; `/dashboard` and `/logs` redirect correctly |
| Agents | selected My Agents/Skills/Schedules tab, selected agent/schedule | empty collection; load/action failure | `/agents`; `/skills` and `/hooks` retain their tab redirect |
| Memory | selected scope/subtab, recall result and write rail | no ward/content/results; recall/write failure | `/memory` |
| Vault | selected ward/file and file preview | no wards; no file selected; tree/preview loading and failure | `/vault` |
| Observatory | selected entity, filters, graph controls | graph loading; no graph data; graph/entity error | `/observatory` |
| Graph | selected hierarchy/classification, graph/tree focus | hierarchy/trace loading; no data; trace error | `/observatory-v2` |
| Integrations | selected Tool Servers/Plugins tab and integration detail | empty catalog; connection/test/action failure | `/integrations`; `/connectors` and `/mcps` retain their redirect/tab behavior |
| Settings | selected configuration area, changed validation state, save outcome | no providers; settings loading; validation/save failure | `/settings`; `/providers` redirects correctly |
