# Plan: Supply-Chain Policy Refresh

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

First close the A2A documentation state against the merged implementation.
Then use targeted Cargo updates for the exact failing transitive packages:
move `bitstream-io` to its compatible release that removes yanked `core2`,
move `spin` to its compatible non-yanked release, and move `rand 0.8` to the
patched release named by RustSec. Remove only advisory exceptions that become
unmatched, inspect the resulting graph and diff, then run supply-chain and full
workspace gates.

## Constraints

- Preserve every direct dependency declaration and feature selection.
- Preserve all matched `deny.toml` advisory exceptions, license rules, bans,
  and sources.
- Keep the lockfile change limited to packages selected by the three targeted
  Cargo updates and their resolver-required replacements.
- Do not change A2A code or public contracts during documentation closeout.

## Construction tests

**Integration tests:** `cargo deny check` followed by the complete Rust
format, workspace check, Clippy, and workspace test sequence.

**Manual verification:** inspect `git diff -- Cargo.lock deny.toml`; verify
`cargo tree -i core2@0.4.0`, `cargo tree -i spin@0.9.8`, and
`cargo tree -i rand@0.8.5` fail because those versions are absent; verify
`cargo tree -i bitstream-io@4.10.0`, `cargo tree -i spin@0.9.9`, and
`cargo tree -i rand@0.8.6` succeed and show transitive paths only.

## Design (LLD)

### Dependencies & integration

Cargo's existing semver requirements remain authoritative. Targeted
`cargo update -p` commands select compatible registry releases without editing
workspace manifests or introducing source overrides. Traces to: AC2, AC3.

### Failure, edge cases & resilience

If a yanked crate has no compatible replacement, stop rather than adding a
policy exception or patch source. If a targeted update expands beyond its
expected replacement set, inspect and either justify each resolver-required
change or revert that update. Traces to: AC2–AC4.

## Tasks

### T1: A2A lifecycle metadata matches the merged implementation

**Depends on:** none

**Touches:** `docs/specs/a2a-federation-discovery/{spec,plan,verification}.md`,
`docs/specs/README.md`, `docs/backlog.md`, `workspace.toml`

**Tests:**

- Goal-based: spec status is `Shipped`, plan status is `Done`, registry status
  is `Shipped`, AC1–AC16 are checked, and AC17 names the matching durable
  backlog deferral (AC1).

**Approach:**

- Check the delivered A2A acceptance criteria, align the three lifecycle status
  surfaces with merged PR #246, and record the unexecuted external-conformance
  journey without claiming it passed.

**Done when:** all A2A lifecycle status probes agree and no implementation file
is changed.

### T2: The dependency graph contains patched, non-yanked compatible releases

**Depends on:** none

**Touches:** `Cargo.lock`, `deny.toml`

**Tests:**

- Goal-based: the pre-fix `cargo deny check` reproduction fails on
  `RUSTSEC-2026-0097`, yanked `core2 0.4.0` and `spin 0.9.8`, and unmatched
  `RUSTSEC-2026-0235`; the final command exits zero with no unmatched advisory
  exceptions (AC2–AC5).
- Goal-based: `cargo tree -i core2@0.4.0`, `cargo tree -i spin@0.9.8`,
  and `cargo tree -i rand@0.8.5` fail because none remains resolved;
  `cargo tree -i bitstream-io@4.10.0`, `cargo tree -i spin@0.9.9`, and
  `cargo tree -i rand@0.8.6` succeed with transitive inverse paths (AC2, AC3).

**Approach:**

- Run targeted compatible updates for `bitstream-io`, `spin`, and `rand 0.8`.
- Remove only stale advisory ignores and inspect every lockfile package changed
  by Cargo.

**Done when:** `cargo deny check` exits zero without a direct-manifest change or
policy bypass.

### T3: Full workspace gates remain green

**Depends on:** T1, T2

**Touches:** none

**Tests:**

- Goal-based: `cargo fmt --all -- --check`, `cargo check --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`, and
  `cargo test --workspace` exit zero (AC5).

**Approach:**

- Run gates in order after the final diff is stable; fix only failures caused
  by this diff.

**Done when:** the complete gate sequence exits zero.

## Rollout

The change ships atomically as documentation, lockfile, and policy metadata.
Reverting the commit restores the prior graph; there is no data migration,
runtime flag, infrastructure change, or irreversible operation.

## Risks

- Registry releases can change resolved transitive packages beyond the named
  crate; the lockfile diff must be reviewed package by package.
- Advisory database drift can reveal a newer issue between local verification
  and CI; rerun `cargo deny check` immediately before publishing.
- Documentation closeout can overclaim delivery if acceptance criteria are
  checked without matching merged evidence; PR #246 and its green workspace
  gates are the evidence baseline.

## Changelog

- 2026-08-06: initial maintenance plan after PR #247 exposed dependency-policy
  drift on the merged A2A dependency graph.
