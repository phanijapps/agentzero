# Plan: P0 Supply-Chain Gates

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Update the smallest set of resolved dependencies needed to clear both RustSec
findings. For Engram licensing, first consume upstream manifest metadata if the
current upstream revision supplies it; otherwise add crate-specific,
content-hashed MIT clarifications from Engram's existing license file. Permit
the OSI-approved Zlib license only for the exact resolved `foldhash` and
`zlib-rs` versions. Verify that no advisory ignore or unlicensed bypass was
introduced.

## Constraints

- Preserve the existing Engram source and feature selection.
- Keep changes confined to dependency resolution, license policy, this spec,
  and the spec registry.
- Do not address the separately reported yanked crates in this change.

## Construction tests

**Integration tests:** run both `cargo deny` gates followed by the repository's
Rust format, check, Clippy, and test gates.

**Manual verification:** inspect the lockfile diff and `cargo tree` inverse
paths for the updated crates to ensure the change did not add an unintended
direct dependency. Record the locked Engram revision and upstream license path,
compute the license file's SHA-256 digest, and confirm both `cargo deny`
clarifications use the tool-computed content hash for that same file.

## Design (LLD)

### Design decisions

- Prefer an upstream Engram license metadata correction over a local
  clarification; fall back to a narrow hash-bound clarification only when
  upstream still omits the member metadata. Traces to: AC3, AC4.
- Use exact-version, crate-specific Zlib exceptions for `foldhash` and
  `zlib-rs`; the license is OSI-approved, but another dependency declaring it
  must still trigger review. Traces to: AC3.

### Failure, edge cases & resilience

- If the latest Engram revision expands the graph or fails workspace gates,
  retain the current Engram revision and use hash-bound clarifications.
- If either patched crate cannot resolve without a broader incompatible
  upgrade, stop rather than ignoring its advisory.

### Dependencies & integration

- Cargo resolves `anyhow` directly from the workspace range and
  `crossbeam-epoch` transitively through active Engram/FastEmbed paths.
- `cargo deny` is the authoritative advisory and license-policy gate.

## Tasks

### T1: Advisory and license gates pass without policy bypasses

**Depends on:** none

**Touches:** `Cargo.lock`, `deny.toml`, `stores/zbot-engram-adapter/Cargo.toml`,
`docs/specs/p0-supply-chain-gates/**`, `docs/specs/README.md`

**Tests:**

- Goal-based: `cargo deny check advisories` exits zero and AC1/AC2 hold.
- Goal-based: `cargo deny check licenses` exits zero and AC3/AC4 hold.
- Evidence-based: record Engram's locked revision, upstream `LICENSE` path and
  SHA-256, and the matching `cargo deny` content hash used by both
  clarifications for AC4.
- Goal-based: `cargo fmt --all -- --check`, `cargo check --workspace
  --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, and
  `cargo test --workspace` exit zero for AC5.

**Approach:**

- Resolve patched versions with the narrowest Cargo updates.
- Refresh Engram only if its current upstream manifests truthfully repair the
  missing license metadata without widening the change unacceptably.
- Add only evidence-backed license policy entries or clarifications.

**Done when:** every acceptance criterion is checked and the complete gate set
passes on the final diff.

## Rollout

The change ships atomically as a lockfile and policy update. Reverting the
commit restores the prior graph; there is no data migration or runtime flag.

## Risks

- Updating Engram from a moving branch may pull unrelated behavior.
- A license clarification can become stale when the dependency revision moves;
  its content hash must fail closed and force re-verification.
- Lockfile-only transitive updates can expose compatibility changes not covered
  by compilation, so the full workspace test suite remains required.

## Changelog

- 2026-07-30: initial plan.
- 2026-07-30: completed narrow advisory updates, scoped license policy, and all
  supply-chain and workspace verification gates.
