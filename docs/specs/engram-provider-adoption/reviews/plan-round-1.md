# Plan review — round 1

## Blockers

**1. Enforce trusted-root confinement.** `spec.md:22` and `plan.md:T1` preserve validation without requiring `AdapterConfig::to_engram_config` / `resolve_engram_path`, canonical root confinement, symlink rejection, or adversarial path tests. Fix: make that existing path the only bootstrap route and add traversal, absolute-outside-root, symlinked-parent, and symlinked-file tests.

**2. Do not expose raw export/import.** `spec.md` and `plan.md:T5` expose executable portability handles without a confined source/destination, generated names, size budget, overwrite rule, or symlink policy. Fix: defer export/import to a dedicated portability spec with a restricted wrapper.

**3. Redact operation failures.** `plan.md` only maps absent-handle errors. Provider operation errors could leak paths, SQL, or storage details. Fix: require a stable redacted adapter operation-error mapper, internal-only diagnostics, no retries, and tests for open/capability/service failures.

**4. Enable the integration SQLite feature.** `plan.md:T1` removes `engram-conformance` but does not enable `engram-integration`'s `sqlite` feature, which currently powers `EngramProvider::open`. Fix: add the existing dependency feature explicitly and verify the feature tree.

**5. Keep raw Engram recall distinct from policy-safe z-Bot recall.** `plan.md:T2-T3` risks treating the provider's raw recall handle as `AdapterFeature::Recall`, contradicting the existing unified-recall policy boundary. Fix: add a separately named adapter-only capability and prove existing `Recall` readiness/reporting stays unchanged.

## Concerns

**6. Gate capability and handle together.** Capability translation currently names a helper that sees only the upstream report. Fix: use a per-service gate that receives the provider and add report/handle disagreement tests.

**7. Prove data compatibility.** Fresh-root tests do not prove existing `engram_data.db` compatibility. Fix: seed representative facts, graph records, and sidecars through the old adapter path, reopen through facade bootstrap, inspect reads and `PRAGMA integrity_check`, and confirm no new SQLite files.

**8. Do not require an unspecified deterministic partial batch.** A normal SQLite batch may be complete. Fix: require forwarding of actual guarantee and step outcomes; move deterministic partial-failure proof upstream unless Engram supplies a stable fixture.

**9. Make the Engram revision risk explicit.** RFC-0012 expects a pinned compatible upstream. Fix: add an enforceable revision mechanism, or record this as an unsatisfied upstream prerequisite rather than implying it is completed.

## Nits

**10. Use one draft status vocabulary.** `spec.md` uses `Draft` while `plan.md` uses `Drafting`. Fix: use `Draft` consistently.
