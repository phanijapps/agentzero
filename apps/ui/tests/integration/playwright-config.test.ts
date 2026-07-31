import { describe, expect, it } from 'vitest';

import playwrightConfig from '../../playwright.config';

declare global {
  const process: { env: Record<string, string | undefined> };
}

const EXPECTED_PROJECTS = [
  'required',
  'live-daemon',
  'provider-backed',
  'diagnostic',
] as const;
const EXPECTED_REQUIRED_FILES = [
  'navigation.spec.ts',
  'persistent-surfaces.spec.ts',
  'smoke.spec.ts',
];

function specFiles(): string[] {
  return Object.keys(import.meta.glob('../e2e/**/*.spec.ts')).map((path) =>
    path.replace('../e2e/', ''),
  );
}

function projectFiles(name: string): string[] {
  const project = playwrightConfig.projects?.find((candidate) => candidate.name === name);
  expect(project, `missing Playwright project: ${name}`).toBeDefined();
  expect(project?.testMatch, `${name} must use an explicit testMatch array`).toBeInstanceOf(Array);
  return (project?.testMatch as string[] | undefined) ?? [];
}

describe('Playwright E2E suite ownership', () => {
  // STUB: AC1 — every E2E spec belongs to exactly one named project.
  it('partitions every E2E spec exactly once', () => {
    expect(playwrightConfig.projects?.map((project) => project.name)).toEqual(EXPECTED_PROJECTS);

    const assigned = EXPECTED_PROJECTS.flatMap(projectFiles);
    expect(new Set(assigned).size).toBe(assigned.length);
    expect([...new Set(assigned)].sort()).toEqual(specFiles().sort());
  });

  // STUB: AC3/AC4 — the required lane remains the deliberate three-file set.
  it('keeps the required project fixed to the deterministic files', () => {
    expect(projectFiles('required').sort()).toEqual(EXPECTED_REQUIRED_FILES);
  });
});
