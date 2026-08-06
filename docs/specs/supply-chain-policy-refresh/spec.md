# Spec: Supply-Chain Policy Refresh

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Discovery:** none
- **Contract:** none
- **Shape:** integration

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Repository maintainers receive a reproducible Rust dependency graph that passes
the existing supply-chain policy after registry and advisory-database drift,
without adding direct dependencies, suppressing active advisories, or changing
z-Bot runtime behavior. The already-delivered A2A initiative is also recorded
consistently as shipped in its spec, plan, and spec registry.

## Boundaries

### Always do

- Resolve affected transitive crates through the narrowest compatible lockfile
  updates and retain `Cargo.lock` as the reproducible source of versions.
- Remove advisory exceptions when their target is no longer in the resolved
  graph, and verify the complete graph with `cargo deny check`.
- Inspect inverse dependency paths and the final lockfile diff for every
  resolved crate changed by the refresh.

### Ask first

- Add or change a direct dependency, dependency feature, or source override.
- Ignore an active advisory, allow a yanked crate, or weaken advisory severity.
- Upgrade a parent dependency across an incompatible version boundary.

### Never do

- Add a patch source or fork solely to silence registry policy output.
- Suppress `RUSTSEC-2026-0097` or retain stale advisory exceptions after their
  dependencies are absent.
- Broaden the lockfile refresh beyond crates selected by the targeted Cargo
  resolution commands.

## Testing Strategy

- **Goal-based dependency checks:** the failing `cargo deny check` command is
  the regression oracle because the defect is resolved dependency and policy
  state rather than application logic.
- **Goal-based graph checks:** exact `cargo tree --invert` probes prove the
  vulnerable or yanked versions are absent and identify their replacements.
- **Goal-based integration gates:** formatting, workspace typechecking,
  Clippy, and workspace tests prove the compatible transitive refresh preserves
  build and runtime contracts.
- **Security review:** a supply-chain review checks provenance, update scope,
  policy strength, and unresolved advisories after the mechanical gates pass.

## Acceptance Criteria

- [x] The A2A federation spec, plan, and spec registry consistently record the
  merged initiative as `Shipped`, `Done`, and `Shipped`, respectively; AC1–AC16
  are checked and the unexecuted external-conformance AC17 has a durable backlog
  deferral rather than claimed evidence.
- [x] `cargo deny check advisories` exits successfully with `rand >= 0.8.6`,
  `spin >= 0.9.9`, and no resolved `core2 0.4.0`.
- [x] The lockfile resolves `bitstream-io >= 4.10.0` through its existing
  compatible requirement and contains no yanked `core2 0.4.0` or `spin 0.9.8`.
- [x] `deny.toml` contains no stale advisory exception for removed crates and
  introduces no new advisory ignore or policy relaxation.
- [x] `cargo deny check`, Rust formatting, workspace typechecking, Clippy, and
  workspace tests all exit successfully on the final diff.

## Assumptions

- Technical: the current failure is in transitive resolution and `deny.toml`,
  not direct manifests (source: `cargo deny check` and `cargo tree --invert`
  probes on `224e5eff`).
- Product: scope is A2A lifecycle closeout plus the reported dependency-policy
  repair (source: user confirmation 2026-08-06).
- Process: dependency-policy changes require full supply-chain verification and
  security review (source: `docs/specs/p0-supply-chain-gates/spec.md`).
