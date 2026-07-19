# Plan: Research Goal Deliverables

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Use the existing goal-artifacts query rather than adding a new API or artifact
role. Snapshot hydration, root-completion refresh cache warming, and the
Research preview cache-miss path all request the bounded goal manifest. The
snapshot additionally filters on `isGoalArtifact === true` so the attachment
strip fails closed if a server returns an unfiltered legacy response.

## Constraints

`docs/specs/goal-artifacts/spec.md` is frozen history and intentionally remains
unchanged even though it limited the original presentation filter to Quick
Chat. This newer spec supersedes that Research-presentation boundary after the
user confirmation recorded in `spec.md`.

## Tasks

### T1: Research exposes only explicit goal deliverables

**Depends on:** none

**Touches:** `apps/ui/src/features/research-v2/session-snapshot.ts`, `apps/ui/src/features/research-v2/useResearchSession.ts`, `apps/ui/src/features/research-v2/ResearchPage.tsx`, `apps/ui/src/features/research-v2/artifact-poll.ts`, `apps/ui/src/features/research-v2/*.test.ts*`

**Tests:**

- TDD: a snapshot built from a mixed artifact manifest contains only the row
  marked `isGoalArtifact: true`, while a marked Python deliverable remains.
- TDD: snapshot and Research fallback calls pass `{ goalArtifactsOnly: true,
  limit: 24 }` and state cannot include an undesignated row (AC1-3).
- Goal-based: focused Research tests, UI lint, and production build pass.

**Approach:**

- Define the bounded goal-artifact query once in the Research feature.
- Apply it to every manifest read that can populate or resolve the strip.
- Preserve the existing slide-out and ward-explorer APIs; they receive only the
  selected persisted deliverable record.

**Done when:** a mixed persisted manifest leaves only explicit deliverables in
Research state and the focused regression tests are green.

## Rollout

Ship with the UI bundle. No migration, backend deployment, or feature flag is
needed because the existing goal-artifact API is already available. Rollback is
reverting the Research-only query and defensive filter.

## Risks

Legacy artifacts lack an explicit designation and will disappear from Research
attachments; this is intentional and they remain in the ward explorer.

## Changelog

- 2026-07-15: initial light-mode plan after user confirmed that unmarked
  Research attachments should be hidden.
- 2026-07-15: shipped bounded goal-manifest reads and a defensive client-side
  designation filter across hydration, live refresh caching, and preview
  cache misses.
