# Goal Artifacts — Review Round 3

## Blockers

**1. Direct HTML artifact content remains executable.** `gateway/src/http/artifacts.rs:87`. Fix: serve HTML/HTM as a non-executable attachment with `nosniff`, while the slide-out renders fetched text in a script-disabled sandbox.

**2. Serving has a check/read race.** `gateway/src/http/artifacts.rs:87`. Fix: open once with no-follow semantics, validate the opened handle, then read that exact handle; reject on unsupported platforms rather than use a path check followed by a path read.

**3. Required session binding breaks stale UI bundles.** `docs/specs/goal-artifacts/plan.md:261`. Fix: require a coordinated gateway/UI release instead of claiming old-client compatibility.

## Concerns

**4. Oversized artifact UI behavior lacks a complete 413 state.** `docs/specs/goal-artifacts/plan.md:181`. Fix: test one preview-unavailable state for every preview type and suppress downloads that would also return 413.

**5. List limit behavior conflicts with its contract.** `docs/specs/goal-artifacts/plan.md:181`. Fix: reject invalid limits with 400 and document both list-limit and missing-session 400 responses.
