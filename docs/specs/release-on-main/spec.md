# Spec: Release On Main

- **Status:** Done for now
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** `docs/architecture/future-state/2026-05-03-release-on-main-workflow-design.md`; `docs/architecture/future-state/2026-05-03-versioning-and-rename-plan.md`; existing `.github/workflows/release.yml`; existing `scripts/release.sh`

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Automate the daily CalVer release bump when changes land on `main`, while
keeping `scripts/release.sh` as the manual release path. A push to `main` should
either no-op when `[workspace.package].version` already equals today's
`YYYY.M.D`, or create exactly one release bump commit, annotated `vYYYY.M.D`
tag, and dispatch the existing release artifact workflow for that tag.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off
before proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Keep `scripts/release.sh` working as the manual fallback.
- Use the existing `vYYYY.M.D` CalVer tag scheme with no zero padding.
- Update root `Cargo.toml`, `apps/ui/package.json`, and
  `apps/ui/package-lock.json` together.
- Add `workflow_dispatch` so the workflow can be tested manually before relying
  on automatic `push` behavior.
- Serialize runs with a workflow-level concurrency group so two pushes to
  `main` cannot race the same date tag.
- Fail before pushing if verification fails after the version bump.
- Dispatch the existing `.github/workflows/release.yml` after the release tag is
  pushed, because `GITHUB_TOKEN`-authored pushes do not recursively trigger
  push workflows.

### Ask first

- Replacing `scripts/release.sh` instead of keeping it as fallback.
- Changing the CalVer scheme or allowing multiple normal releases per day.
- Using a PAT secret instead of `GITHUB_TOKEN` for bot pushes.
- Letting the workflow bypass protected branch policy.
- Creating GitHub Releases directly from this workflow instead of relying on
  the existing tag-triggered `.github/workflows/release.yml`.
- Running heavyweight full-workspace tests in this workflow.
- Introducing a PAT or GitHub App token instead of the default `GITHUB_TOKEN`.

### Never do

- Never create a release commit without a matching annotated tag.
- Never push a tag when the version files were not updated to the same version.
- Never silently ignore a failed commit, tag, or push.
- Never add `[skip ci]`, `[ci skip]`, `[no ci]`, `[skip actions]`, or
  `[actions skip]` to the release commit; those tokens can skip push-triggered
  workflows.
- Never use `sed -i` for cross-platform version editing in repository scripts.
- Never mutate unrelated files such as installer scripts, release packaging, or
  security workflows in this feature.
- Never weaken or remove the existing tag-triggered release workflow.

## Testing Strategy

- Version computation and file mutation: **TDD / script-level checks**. The
  logic is deterministic enough to test with temporary fixture files or a
  dry-run workflow step.
- Workflow shape: **goal-based check**. Static validation can prove the workflow
  contains the right triggers, permissions, concurrency, no-op branch, bump
  branch, commit, tag, and push steps.
- End-to-end workflow behavior: **manual QA through `workflow_dispatch`**. A
  GitHub runner is the only realistic place to validate bot push permissions,
  tag creation, and release workflow triggering.
- Existing release path preservation: **goal-based check**. `scripts/release.sh
  --dry-run` must still run and `.github/workflows/release.yml` must still be
  tag-triggered.

## Acceptance Criteria

- [ ] `.github/workflows/release-on-main.yml` exists.
- [ ] The workflow supports `workflow_dispatch`.
- [ ] The workflow supports `push` to `main`.
- [ ] The workflow computes today's version in UTC as `YYYY.M.D` with no zero
  padding and derives the tag as `vYYYY.M.D`.
- [ ] If root `Cargo.toml` already equals today's version, the workflow exits
  successfully without committing or tagging.
- [ ] If root `Cargo.toml` differs from today's version, the workflow updates
  root `Cargo.toml`, `apps/ui/package.json`, and `apps/ui/package-lock.json` to
  the same plain version.
- [ ] The workflow verifies the bumped repository before commit with a
  lightweight parse/build gate appropriate for a release-bump commit.
- [ ] The workflow commits with message `release: YYYY.M.D`.
- [ ] The workflow creates an annotated `vYYYY.M.D` tag.
- [ ] The workflow pushes the release commit and tag, then dispatches
  `.github/workflows/release.yml` with input `version=vYYYY.M.D`.
- [ ] Concurrent runs cannot create duplicate same-day release commits or tags.
- [ ] `scripts/release.sh --dry-run` still works after this feature lands.
- [ ] Existing `.github/workflows/release.yml` remains tag-triggered and is not
  weakened by this feature.
- [ ] Branch-protection failure mode is documented in the plan with the required
  remediation choices: bot bypass or PAT secret.

## Assumptions

- Technical: existing release artifact publication is tag-triggered by
  `.github/workflows/release.yml` on `v*` tags and already supports
  `workflow_dispatch` with a version input (source:
  `.github/workflows/release.yml`).
- Technical: manual release cutting already exists in `scripts/release.sh` and
  updates `Cargo.toml`, `apps/ui/package.json`, `apps/ui/package-lock.json`,
  `CHANGELOG.md`, commits, tags, and pushes (source: `scripts/release.sh`).
- Technical: the deferred future-state design already defines the desired
  no-op-on-same-day, bump-on-new-day, commit, tag, and release-dispatch
  behavior; implementation deliberately differs from its `[skip ci]`
  recommendation because GitHub skip tokens affect push workflows
  (source:
  `docs/architecture/future-state/2026-05-03-release-on-main-workflow-design.md`;
  GitHub Docs, "Skipping workflow runs").
- Technical: `GITHUB_TOKEN`-authored pushes do not create new workflow runs,
  except for `workflow_dispatch` and `repository_dispatch`; this workflow must
  explicitly dispatch the existing release workflow after pushing the tag
  (source: GitHub Docs, "Triggering a workflow").
- Product: versioning remains the existing `vYYYY.M.D` CalVer scheme rather
  than a new versioning project (source:
  `docs/architecture/future-state/2026-05-03-versioning-and-rename-plan.md`).
- Process: branch protection on `main` may prevent `GITHUB_TOKEN` from pushing;
  the first implementation must make that failure visible and document either
  bot bypass or PAT as the remediation (source:
  `docs/architecture/future-state/2026-05-03-release-on-main-workflow-design.md`).
