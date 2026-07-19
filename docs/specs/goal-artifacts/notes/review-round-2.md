# Goal Artifacts — Review Round 2

## Blockers

**1. HTML artifact previews execute untrusted script.** `apps/ui/src/features/chat/ArtifactSlideOut.tsx:120`. Fix: remove `allow-scripts` from the iframe sandbox and test that model-produced HTML is rendered as non-executable data.

**2. Quick Chat fetches an unbounded artifact manifest and reads unbounded preview content.** `docs/specs/goal-artifacts/plan.md:85`. Fix: add a bounded `goal_artifacts_only=true&limit=24` manifest query for Quick Chat and reject artifacts larger than 5 MiB at persist and serve time.

**3. The shared content contract conflicts with its stated compatibility.** `docs/specs/goal-artifacts/spec.md:35`. Fix: retain Research's unfiltered strip and preview UX while explicitly tightening the shared content request's session binding and confinement.

**4. The session goal-artifact budget is race-prone.** `docs/specs/goal-artifacts/plan.md:138`. Fix: use a repository transaction that counts and inserts under the same lock/transaction; prove concurrent writes persist at most 24 goal artifacts.

## Concerns

**5. A session ID query is correlation, not authentication.** `docs/specs/goal-artifacts/plan.md:70`. Fix: describe the current single-owner limitation and defer multi-user authorization to an authenticated session-access policy.

**6. Content URL regression callers were omitted.** `docs/specs/goal-artifacts/plan.md:155`. Fix: include HTTP transport and Quick Chat E2E content URL tests in the task.
