# Plan: UI Test Type-Debt Cleanup

- **Spec:** [`spec.md`](spec.md)
- **Status:** Executing

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially,
> note why in the changelog at the bottom.

## Approach

Update the base UI compiler target/library to ES2022 and make the package build
invoke that project directly. Repair only the seven fixtures currently named
by the compiler, using the exported transport/domain types as the contract,
then delete `tsconfig.build.json`. Verify the unified compiler gate first, then
lint, Vitest, and the production bundle.

Files expected to change are `apps/ui/tsconfig.json`, `apps/ui/package.json`,
`apps/ui/tsconfig.build.json`, the seven compiler-reported `*.test.*` files,
and the P5 spec/debt registry documents. No production TypeScript source,
public interface, dependency, or test assertion behavior changes.

Tempted to introduce fixture factories; declining because the seven local
fixtures can be corrected directly without a new abstraction. Tempted to
loosen transport types or add suppression directives; declining because the
compiler errors are the debt this change exists to expose and repair.

## Constraints

- Use the existing TypeScript 5.8, Vitest, ESLint, and Vite toolchain.
- Preserve the transport/domain types as the source of truth.
- Do not modify runtime source files to make tests compile.

## Construction tests

- `npx tsc --noEmit --pretty false` reports zero errors across `src/`.
- `npm run lint`, `npm test`, and `npm run build` pass.
- A repository check finds no `tsconfig.build.json` reference and no new
  suppression directive in the touched tests.

## Design (LLD)

### Design decisions

- The base TypeScript project becomes the single compiler contract for both
  production and test sources. Traces to AC1-AC3.
- Fixture objects are repaired at their definition sites against current
  exported types. Traces to AC4-AC5.

### Quality attributes (NFRs)

- Type safety: zero base-project errors and no suppression directives.
- Maintainability: one TypeScript project drives editor, test, and build
  checking rather than allowing their accepted source sets to drift.

## Tasks

### T1: The unified UI compiler, tests, and build are green

**Depends on:** none

**Touches:** apps/ui/tsconfig.json, apps/ui/tsconfig.build.json,
apps/ui/package.json, apps/ui/src/features/logs/log-hooks-extended.test.ts,
apps/ui/src/features/logs/useSessionTrace.test.ts,
apps/ui/src/features/memory/command-deck/SearchResults.test.tsx,
apps/ui/src/features/research-v2/IntentInfoButton.test.tsx,
apps/ui/src/features/research-v2/useResearchSession.test.ts,
apps/ui/src/services/transport/http.embeddings.test.ts,
apps/ui/src/services/transport/http.ws.test.ts, docs/tech-debt.md,
docs/specs/ui-test-type-debt/**, docs/specs/README.md

**Verification mode:** goal-based check

**Tests:**

- No TDD stub (configuration and fixture-conformance task).
- Run the compiler, suppression search, lint, full Vitest suite, and build
  commands in `Construction tests` (AC1-AC5).
- Verify TD-043 describes the measured baseline and completed result (AC6).

**Approach:**

- Set the base compiler target/library to ES2022 and point `npm run build` at
  the base project.
- Repair each compiler-reported fixture from its current exported type; remove
  unused imports and resolve definite-assignment errors without changing test
  intent.
- Delete the obsolete exclusion config and update TD-043 after all gates pass.

**Done when:** the base compiler, lint, complete Vitest suite, and build pass,
and no test source is excluded from the build's TypeScript check.

## Rollout

One reversible pull request. No deployment, data, API, or infrastructure
sequencing is required.

## Risks

- A cast could make the compiler green while leaving a stale fixture; direct
  typed fixture construction and the suppression search mitigate this.
- Raising the target assumes supported desktop/web runtimes implement ES2022;
  the existing Vite production target remains the bundling compatibility
  boundary.

## Changelog

- 2026-07-31: Initial light-mode plan based on the measured 36-error,
  seven-file compiler baseline.
