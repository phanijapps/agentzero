# Plan: Release On Main

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done for now

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially
> (a different approach, not just a re-ordering), note why in the changelog
> at the bottom.

## Approach

Add a small release automation workflow that owns the version-bump commit, tag,
and dispatch of the existing release workflow. The existing
`.github/workflows/release.yml` remains responsible for building artifacts and
creating the GitHub Release. Keep the workflow conservative: manual trigger
first-class, automatic trigger on `main`, no-op on same-day versions,
verification before commit, annotated tag, explicit release workflow dispatch,
and explicit branch-protection failure notes.

## Constraints

- Follow `docs/architecture/future-state/2026-05-03-release-on-main-workflow-design.md`.
- Inherit `vYYYY.M.D` from
  `docs/architecture/future-state/2026-05-03-versioning-and-rename-plan.md`.
- Do not replace or weaken `scripts/release.sh`.
- Do not replace or weaken `.github/workflows/release.yml`.
- Use `GITHUB_TOKEN` by default; use `workflow_dispatch` to trigger
  `.github/workflows/release.yml` because `GITHUB_TOKEN`-authored pushes do not
  recursively trigger push workflows.
- Switch to PAT or GitHub App token only after human approval if branch
  protection blocks bot pushes.

## Construction tests

**Integration tests:** none beyond per-task checks; GitHub Actions behavior is
validated through `workflow_dispatch`.

**Manual verification:**
- Run the workflow manually on a branch or controlled test path and confirm the
  same-version no-op path.
- Run the workflow manually when the version differs and confirm the release
  commit plus tag are produced.
- Confirm the workflow dispatches `.github/workflows/release.yml` with the new
  tag as the `version` input.

## Tasks

### T1: Add the release-on-main workflow

**Depends on:** none

**Touches:** `.github/workflows/release-on-main.yml`

**Tests:**
- Verify the workflow file contains both `workflow_dispatch` and `push` to
  `main` triggers.
- Verify the workflow declares `permissions.contents: write`.
- Verify the workflow declares `permissions.actions: write`.
- Verify the workflow has a `release-on-main` concurrency group with
  `cancel-in-progress: false`.
- Verify the workflow computes `TODAY` as `YYYY.M.D` and `TAG` as
  `vYYYY.M.D`.

**Approach:**
- Create `.github/workflows/release-on-main.yml`.
- Use `actions/checkout@v4` with `fetch-depth: 0`.
- Use Node 20 only on the bump path so `npm version` can update package files.
- Compute current version from `[workspace.package]` in root `Cargo.toml`.
- Expose `skip`, `today`, and `tag` through `$GITHUB_OUTPUT`.

**Done when:** the workflow is present and statically matches the trigger,
permission, concurrency, and version-computation contract.

### T2: Implement no-op and bump paths

**Depends on:** T1

**Touches:** `.github/workflows/release-on-main.yml`

**Tests:**
- Same-day path: if `CURRENT == TODAY`, every mutation, commit, tag, and push
  step is skipped.
- Bump path: root `Cargo.toml`, `apps/ui/package.json`, and
  `apps/ui/package-lock.json` are staged together.
- Commit message is exactly `release: YYYY.M.D`.
- Tag command creates annotated `vYYYY.M.D`.
- Release dispatch calls `.github/workflows/release.yml` with
  `version=vYYYY.M.D`.

**Approach:**
- Use a portable `awk` script to update only the `[workspace.package]` version
  in `Cargo.toml`.
- Use `npm --prefix apps/ui version --no-git-tag-version --allow-same-version
  "$TODAY"` for UI package and lockfile updates.
- Add a verification step after mutation and before commit. Keep it lightweight:
  `cargo metadata --no-deps` plus a package-lock consistency check is preferred
  over full `cargo check --workspace` for this narrow release-bump workflow.
- Configure `github-actions[bot]` identity for the commit.
- Push the branch and tag after the commit is created.
- Dispatch `release.yml` via the Actions workflow dispatch API after the tag is
  pushed.

**Done when:** the workflow has separate guarded no-op and bump paths and cannot
dispatch release packaging without first committing the version-file changes and
pushing the annotated tag.

### T3: Preserve existing manual and tag-triggered release paths

**Depends on:** T1

**Touches:** `scripts/release.sh`, `.github/workflows/release.yml`

**Tests:**
- Run `scripts/release.sh --dry-run`.
- Confirm `.github/workflows/release.yml` still has `on.push.tags: ['v*']` and
  `workflow_dispatch` with a required `version` input.
- Confirm the new workflow does not edit `.github/workflows/release.yml`.

**Approach:**
- Do not edit `scripts/release.sh` unless a compatibility issue is discovered.
- Do not edit `.github/workflows/release.yml`.
- If a compatibility issue is discovered, update the spec before changing
  either existing release path.

**Done when:** manual dry-run still works and the existing release workflow
still responds to pushed `v*` tags.

### T4: Document operation and branch-protection failure modes

**Depends on:** T1-T3

**Touches:** `docs/specs/release-on-main/plan.md`,
`docs/specs/release-on-main/spec.md`, optionally `docs/publishing.md`

**Tests:**
- Plan documents what happens when `GITHUB_TOKEN` cannot push to protected
  `main`.
- Plan names the two approved remediations: allow bot bypass or approve a PAT
  secret.
- Publishing docs, if touched, point to both manual and automatic release paths.

**Approach:**
- Keep this spec and plan as the primary implementation record.
- Add a short note to publishing docs only if the current publishing docs would
  mislead an operator after this workflow lands.

**Done when:** a failed protected-branch push has a documented operator path
instead of looking like an unexplained workflow bug.

### T5: Manual GitHub workflow verification

**Depends on:** T1-T4

**Touches:** none expected

**Tests:**
- Trigger `workflow_dispatch` and confirm same-version no-op.
- Trigger a controlled bump run and confirm the release commit and annotated tag.
- Confirm the run dispatches `.github/workflows/release.yml`.
- If protected branch blocks the push, record the failure and choose one of the
  documented remediations before retrying.

**Approach:**
- Run this only after the PR lands or from a test branch adjusted to avoid
  touching protected `main`.
- Do not fake success locally; the important behavior is GitHub permission and
  tag-trigger interaction.

**Done when:** the first successful GitHub run produces either a clean no-op or
  a valid release bump plus tag plus release-workflow dispatch, and any
  branch-protection decision is recorded.

## Rollout

Ship as one PR. The workflow includes `workflow_dispatch` and `push` to `main`
from the start, but the manual path is the first verification route. If
branch-protection blocks the workflow-authored push, leave the workflow in
place, document the failure, and choose bot bypass or PAT in a follow-up PR.

## Risks

- Protected `main` may reject `GITHUB_TOKEN` pushes. Mitigation: document the
  failure and require human approval for bot bypass or PAT.
- A release commit can trigger this workflow again. Mitigation: keep the
  workflow no-op-on-current-version so re-entry cannot create a second same-day
  bump.
- A tag may be pushed while artifact release dispatch or artifact release fails.
  Mitigation: rerun/fix `.github/workflows/release.yml` rather than expanding
  this feature's scope.
- `cargo check --workspace` may be too slow for a release-bump-only workflow.
  Mitigation: use `cargo metadata --no-deps` unless the spec is updated to
  require a heavier gate.

## Changelog

- 2026-06-03: initial plan.
