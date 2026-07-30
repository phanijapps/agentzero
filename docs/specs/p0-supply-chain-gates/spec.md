# Spec: P0 Supply-Chain Gates

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none
- **Shape:** integration

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Restore the repository's Rust supply-chain gates after newly published
advisories and incomplete dependency license metadata made `cargo deny` fail.
The resolved dependency graph must contain patched `anyhow` and
`crossbeam-epoch` releases, and every production dependency must have a
truthful license determination accepted by the repository policy.

## Boundaries

### Always do

- Keep `Cargo.lock` as the reproducible source of resolved dependency versions.
- Verify advisories and licenses with `cargo deny` against the resulting lockfile.
- Base every license clarification on an upstream license file and its exact hash.

### Ask first

- Adding a new direct runtime dependency.
- Ignoring a RustSec advisory or allowing an unlicensed dependency.
- Repinning Engram to a non-`main` source or changing its public feature set.

### Never do

- Suppress either P0 advisory in `deny.toml`.
- Use a blanket license exception or claim a license without upstream evidence.
- Broaden this change into unrelated yanked-crate or duplicate-version cleanup.

## Testing Strategy

- **Goal-based checks:** `cargo deny check advisories` proves the vulnerable
  versions are absent, while `cargo deny check licenses` proves every resolved
  license is supported by manifest metadata or a hashed clarification.
- **Goal-based integration gates:** formatting, workspace typechecking, Clippy,
  and workspace tests prove the lockfile or Engram update does not break zbot.
- **Security review:** a supply-chain review checks pinning, provenance,
  transitive impact, and whether repository policy was weakened to obtain a
  green result.

## Acceptance Criteria

- [x] `cargo deny check advisories` exits successfully with neither
      RUSTSEC-2026-0190 nor RUSTSEC-2026-0204 present.
- [x] The lockfile resolves `anyhow >= 1.0.103` and
      `crossbeam-epoch >= 0.9.20`.
- [x] `cargo deny check licenses` exits successfully without ignoring
      unlicensed Engram crates or weakening checks globally.
- [x] Every Engram license determination is backed by upstream MIT metadata or
      a content-hashed upstream MIT license file.
- [x] Rust formatting, workspace typechecking, Clippy, and workspace tests pass.

## Assumptions

- Technical: the current `develop` lockfile reproduces two advisory failures
  and four license-policy failures (source: `cargo deny check advisories` and
  `cargo deny check licenses` on `105fb956`).
- Technical: Engram declares `license = "MIT"` at its workspace root and ships
  a root `LICENSE`, while the two reported member manifests omit inherited
  license metadata (source: checked-out Engram revision `0149f9d3`).
- Product: the requested P0 scope is limited to restoring advisory and license
  gates, not general dependency modernization (source: user confirmation
  2026-07-30).
- Process: dependency and lockfile changes require vulnerability audit and
  security review (source: `docs/architecture/security.md`).
