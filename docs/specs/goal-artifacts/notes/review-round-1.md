# Goal Artifacts — Review Round 1

## Blockers

**1. Artifact declarations can disclose arbitrary host files.** `docs/specs/goal-artifacts/spec.md:27`. Fix: constrain declaration and serving to canonical, regular non-symlink files inside the active ward root; bind content serving to its owner session and revalidate at read time.

**2. Goal deliverables are unbounded.** `docs/specs/goal-artifacts/plan.md:69`. Fix: enforce per-response, per-session, and metadata-size limits; make the Quick Chat display budget visible.

## Concerns

**3. The artifact propagation chain is inaccurate.** `docs/specs/goal-artifacts/plan.md:87`. Fix: name `StreamEvent::ActionRespond` and `handle_artifact_declarations`; do not place artifacts on `GatewayEvent::Respond`.

**4. Contract testing names no viable validator.** `docs/specs/goal-artifacts/plan.md:136`. Fix: use the workspace `serde_yaml` shape-test pattern rather than adding a JSON Schema validator.

**5. The turn lifecycle wording is inaccurate.** `docs/specs/goal-artifacts/spec.md:77`. Fix: state that Quick Chat refreshes when the producing turn completes, not when the reserved session completes.

**6. The artifact manifest leaks resolved server paths.** `contracts/jsonschema/goal-artifact-manifest.schema.json:17`. Fix: replace the response contract with a REST contract that omits `filePath` and documents the session-bound content read.
