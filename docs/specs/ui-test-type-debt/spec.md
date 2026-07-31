# Spec: UI Test Type-Debt Cleanup

- **Status:** Implementing
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none
- **Shape:** ui

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

Mode: light (no risk trigger fired)

## Objective

Restore the UI's normal TypeScript/build gate so it type-checks production and
test sources together. Repair the current 36 errors across seven test files,
adopt ES2022 for the UI compiler/runtime contract, and remove the temporary
test-excluding build configuration without changing application behavior.

## Boundaries

### Always do

- Preserve the intent and observable assertions of every repaired test.
- Prefer complete, type-correct fixtures over casts or weakened types.
- Keep `npm run build`, lint, and the complete Vitest suite green.

### Ask first

- Change a production API type or application implementation to accommodate a
  stale test fixture.
- Remove, skip, or materially rewrite a test assertion.

### Never do

- Add a dependency, top-level module boundary, or replacement test framework.
- Use `any`, `@ts-ignore`, `@ts-expect-error`, or broader compiler exclusions
  to conceal a remaining error.
- Change runtime behavior as part of fixture cleanup.

## Testing Strategy

- **Goal-based compiler check:** `npx tsc --noEmit` is the primary contract;
  it must cover all files under `src/` and report zero errors.
- **Goal-based build check:** `npm run build` proves the normal build no longer
  depends on a test-excluding project configuration.
- **Behavior regression:** `npm test` and `npm run lint` prove fixture repairs
  preserve the existing UI tests and source-quality gate.

## Acceptance Criteria

- [x] `npx tsc --noEmit` reports zero errors while including test sources.
- [x] `tsconfig.json` targets and loads ES2022, and no separate build config
  excludes `*.test.*`, `*.spec.*`, or `tests/**` from type-checking.
- [x] `npm run build` uses the base TypeScript project and completes before the
  Vite production bundle.
- [x] The seven currently failing test files use current domain/transport
  types without suppression directives or production-code changes.
- [x] UI lint, the complete Vitest suite, and the production build pass.
- [x] TD-043 records the live 36-error/seven-file baseline and its completed
  resolution.

## Assumptions

- Technical: the current base-project check reports 36 errors across seven test
  files (source: `npx tsc --noEmit --pretty false`, 2026-07-31).
- Technical: production compilation passes because `tsconfig.build.json`
  excludes tests and `npm run build` selects that project (source:
  git revision 552891bf package/config snapshot).
- Technical: ES2022 support removes the nine `Array.prototype.at` library
  errors before fixture repair (source: local TypeScript 5.8.3 compiler probe).
- Process: specs use the repository's documented status and acceptance-criteria
  metadata contract (source: `docs/CONVENTIONS.md` section 4).
- Product: P5 repairs all current errors, adopts ES2022, removes the exclusion,
  and does not change production behavior (source: user confirmation
  2026-07-31).

## Verification Evidence

- `npx tsc --noEmit --pretty false`: zero errors with all `src/` test sources
  included.
- `npm run lint`: passed with zero errors and 21 pre-existing warnings.
- `npm test`: 117 files and 1,352 tests passed.
- `npm run build`: the unified `tsc && vite build` command passed.
- The scoped repository search found no TypeScript suppression directive or
  `tsconfig.build.json` reference in the changed UI configuration/tests.
