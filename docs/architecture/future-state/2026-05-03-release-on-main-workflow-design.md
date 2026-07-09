# PR D — `release-on-main.yml` auto-version workflow

**Status:** Design (deferred per [versioning + rename plan](2026-05-03-versioning-and-rename-plan.md))
**Date:** 2026-05-03
**Owner:** phanijapps
**Target branch:** `develop` (workflow itself runs on `main`)

## Why this is deferred

The plan called for a GitHub Actions workflow that auto-bumps the workspace version every time something lands on `main`. We deliberately put it off so a few manual cuts could run first — to confirm the bump rhythm, the tag shape, and whether anything breaks with the new CalVer format. As of this doc, **PR B (#106)** has already cut `2026.5.3` manually, and PRs A1/A2/A3 + C are landing under that version. Once a couple more manual cuts happen and nothing has surprised, this is the next thing to ship.

## What it does

On every push to `main`:

1. Compute today's date as `YYYY.M.D` (no zero-padding — see plan doc for why).
2. Compare with the current `Cargo.toml [workspace.package].version`.
3. If they match — no-op (multiple merges to `main` on the same day shouldn't bump twice).
4. If they differ — update both `Cargo.toml` and `apps/ui/package.json`, regenerate `package-lock.json`, commit with `[skip ci]`, tag `vYYYY.M.D`, push with `--follow-tags`.

The `[skip ci]` flag is critical: without it, the bot's commit would re-trigger the workflow and infinite-loop.

## The workflow file

Path: `.github/workflows/release-on-main.yml`

```yaml
name: release-on-main

on:
  push:
    branches: [main]

# Avoid concurrent runs racing each other if two PRs land in quick succession.
# Group by branch; latest wins.
concurrency:
  group: release-on-main
  cancel-in-progress: false

jobs:
  release:
    runs-on: ubuntu-latest
    permissions:
      contents: write          # needed to push the bump commit + tag
    steps:
      - name: Checkout
        uses: actions/checkout@v4
        with:
          fetch-depth: 0
          token: ${{ secrets.GITHUB_TOKEN }}
          # Use a deploy key or PAT here if branch protection blocks
          # GITHUB_TOKEN-authored pushes — see "branch protection" below.

      - name: Compute today's CalVer
        id: ver
        run: |
          TODAY="$(date -u +%Y.%-m.%-d)"   # %-m / %-d strip leading zeros (GNU date on ubuntu-latest)
          CURRENT="$(awk -F\" '/^version[[:space:]]*=/ {print $2; exit}' Cargo.toml)"
          echo "today=$TODAY" >> "$GITHUB_OUTPUT"
          echo "current=$CURRENT" >> "$GITHUB_OUTPUT"
          if [ "$CURRENT" = "$TODAY" ]; then
            echo "skip=1" >> "$GITHUB_OUTPUT"
            echo "Already at $TODAY — nothing to bump."
          else
            echo "skip=0" >> "$GITHUB_OUTPUT"
            echo "Bumping $CURRENT → $TODAY"
          fi

      - name: Set up Node + npm (for package-lock regen)
        if: steps.ver.outputs.skip == '0'
        uses: actions/setup-node@v4
        with:
          node-version: '20'

      - name: Bump Cargo.toml + package.json
        if: steps.ver.outputs.skip == '0'
        env:
          TODAY: ${{ steps.ver.outputs.today }}
        run: |
          # Replace ONLY the first version = "..." line in Cargo.toml
          # (that's [workspace.package].version; per-crate overrides come
          # later and inherit via version.workspace = true anyway).
          awk -v ver="$TODAY" '
            !done && /^version[[:space:]]*=/ { sub(/"[^"]*"/, "\"" ver "\""); done=1 }
            { print }
          ' Cargo.toml > Cargo.toml.new && mv Cargo.toml.new Cargo.toml

          ( cd apps/ui && npm version --no-git-tag-version --allow-same-version "$TODAY" )

      - name: Verify the workspace still parses
        if: steps.ver.outputs.skip == '0'
        run: cargo check --workspace --quiet

      - name: Commit + tag + push
        if: steps.ver.outputs.skip == '0'
        env:
          TODAY: ${{ steps.ver.outputs.today }}
        run: |
          git config user.name  "github-actions[bot]"
          git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
          git add Cargo.toml apps/ui/package.json apps/ui/package-lock.json
          git commit -m "release: $TODAY [skip ci]"
          git tag -a "v$TODAY" -m "Release $TODAY"
          git push --follow-tags
```

## Edge cases handled in the spec above

| Case | Handling |
|---|---|
| Two merges to `main` on the same day | Second run sees `current == today`, sets `skip=1`, exits. No-op. |
| Bot's own bump commit re-triggering the workflow | `[skip ci]` in the commit message. GitHub's default workflow trigger respects this token. |
| `cargo check` fails after the bump | The job fails before push. Manual recovery: revert locally, push, investigate. Nothing reaches `main` mid-state. |
| Concurrent runs racing | `concurrency: release-on-main` serializes them. `cancel-in-progress: false` so we don't drop a bump mid-flight. |
| `npm version` rejecting same-version write | `--allow-same-version` flag prevents the bump-already-happened case from failing the run. (Belt-and-suspenders; the `skip` step usually catches it first.) |
| Detached HEAD / no branch context | Workflow runs on a refs/heads/main checkout — there's always a branch. Doesn't apply. |

## Edge cases NOT handled (deliberately)

- **Hand-bumped versions across day boundaries.** If someone manually merged a `2026.5.4` PR on the morning of `2026.5.5`, the workflow's `today=2026.5.5` won't match `current=2026.5.4` and will bump again. Outcome: tag for `2026.5.5` cut as well. That's correct — the day's first merge produces the day's tag.
- **Reverts.** A revert PR landing on main goes through the same flow; if the version doesn't already match today, it bumps. The tag captures whatever's on `main` at that moment, including the reverted state.
- **Merging a release commit from a feature branch.** The workflow runs and detects `current == today`, so it skips. The pre-existing tag stands.

## Branch protection considerations

If `main` requires PRs (protected branch with "require pull request reviews"):

- **Default**: `secrets.GITHUB_TOKEN` cannot push to a protected branch. Workflow will fail.
- **Fix**: either
  - Allow the `github-actions[bot]` to bypass branch protection in repo settings, OR
  - Use a Personal Access Token (PAT) with `contents:write` stored as a secret, set the `with: token:` to that secret.
- We're using `github-actions[bot]` for now; if branch protection bites, swap to a PAT and update the doc.

## After the bump — release notes

Two options for surfacing what's in each release. Pick one when this lands:

1. **GitHub Releases auto-draft** — extend the workflow with an extra step using `softprops/action-gh-release@v2`. The release body uses `${{ steps.ver.outputs.today }}` and pulls the changelog from `git log v<previous>..HEAD --oneline`. Free, no schema. Most useful.

2. **Manual `CHANGELOG.md`** — workflow leaves a stub entry; user fills in. More effort per cut, more curated output.

I'd start with (1). Add (2) only if the auto-generated body is too noisy.

## Failure modes & manual recovery

If the workflow fails halfway:

- **Failure before commit** (e.g., `cargo check` fails): nothing is pushed. Fix on a feature branch, re-merge.
- **Failure between commit and push** (network blip): the commit exists locally on the runner but isn't on `main`. The next run will re-bump from the actual `main` HEAD. No corruption.
- **Push succeeds, tag push fails**: very rare. Manual: `git tag -a v$TODAY -m "Release $TODAY" && git push origin v$TODAY` from any clone.

To temporarily disable the workflow without deleting the file: rename the file to `.disabled` or comment out the `on:` block. GitHub will skip parsing.

## Future enhancements (out of scope for the first cut of PR D)

- **Build verification on tag** — separate workflow on `push: tags` runs `cargo build --release && npm run build` and uploads artifacts. Useful when we ship binaries.
- **Crate publishing** — if any of these crates ever go to crates.io, add a `cargo publish` step gated on the tag.
- **Slack / Discord webhook** notifying release.
- **Rollback automation** — a workflow_dispatch with a target version that re-tags an old commit. Probably overkill until we ship to real users.

## When to land PR D

Recommended preconditions:
- At least 2-3 manual cuts have happened (`v2026.5.3` is `B`'s; whatever lands tomorrow + after) so the rhythm is observed.
- Branch protection on `main` is decided one way or the other (GITHUB_TOKEN bypass vs PAT).
- A decision on release notes format (option 1 vs 2 above).

Once those are settled, this is roughly a 1-PR job — drop the workflow file in `.github/workflows/`, push to develop, merge, watch the next push to main do its thing.

## Sequencing into the existing rename plan

Updates the [versioning + rename plan](2026-05-03-versioning-and-rename-plan.md) execution order:

| Step | Status |
|---|---|
| PR A1 (cargo bin rename) | merged |
| PR A2 (install scripts + version embed + branch suffix) | merged |
| PR A3 (UI version badge) | merged |
| PR B (manual bump to 2026.5.3) | merged |
| PR C (user copy + mDNS daemon rename) | merged |
| **PR D (this workflow)** | **deferred — ship after a few more manual cuts** |

## Test plan when implementing

- [ ] Land the workflow on develop.
- [ ] Push to main triggers the run.
- [ ] First run on a day-when-version-matches: `skip=1` step output, no commit, no tag.
- [ ] Manually advance the date (or wait until tomorrow): next push to main bumps + tags.
- [ ] Verify the `[skip ci]` flag prevents the workflow from re-triggering on its own commit.
- [ ] Tag push reaches origin and is visible via `git ls-remote --tags origin`.
- [ ] Rust binaries built from the bumped commit report the new version via `--version`.
- [ ] If branch protection is on, confirm the bot has permission to push or swap to PAT.

## Notes for the implementer

- The plan's original sketch (in `2026-05-03-versioning-and-rename-plan.md`) used `sed` to update `Cargo.toml`. I switched to `awk` here because `sed -i` portability between BSD (macOS) and GNU (ubuntu-latest) bites; `awk` is uniform.
- `apps/ui/package-lock.json` regenerates automatically via `npm version`. Add it to the commit explicitly so the lockfile stays in sync.
- The `concurrency` block matters more than it looks: GitHub Actions queues subsequent runs by default but can interleave them with simultaneous pushes. Serializing this one workflow is cheap insurance.
