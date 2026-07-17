# Plan review — round 2

## Blockers

**1. Keep crate-private service tests in the crate.** `plan.md:T2-T4` declares `pub(crate)` service operations but assigns tests to integration-test crates, which cannot call them. Fix: move gate and operation tests to source `#[cfg(test)]` modules; retain external integration tests only for public bootstrap/store behavior.

## Concerns

**2. Use a pre-facade compatibility fixture.** `plan.md:T5` seeds data after the bootstrap change and therefore cannot prove pre-adoption compatibility. Fix: add a versioned, synthetic baseline `engram_data.db` fixture captured through the pre-T1 adapter implementation, with a manifest/checksum, then reopen it only through the new facade path.

**3. Limit migration operations to non-writing actions.** `spec.md` protects migration policy/data but `plan.md:T3` names broad migration actions. Fix: permit inspection and dry-run only; exclude apply/import, automatic migration, mode mutation, and write-capable wrappers.

## Security review

Clean — ready to commit.
