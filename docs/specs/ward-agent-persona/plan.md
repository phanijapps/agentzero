# Plan: Ward Agent Persona

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Expand the existing template-driven Ward scaffold rather than adding another
agent representation. Generate a readable identity from the already-validated
ward ID, write a complete but layout-neutral persona contract to `AGENTS.md`,
and verify that Ward synthesis continues to embed that user-owned doctrine.

Tempted to add a persona-generation service; declining because deterministic
scaffolding plus user-owned evolution is sufficient. Tempted to add purpose and
persona arguments to the Ward tool; declining because that would widen its
public interface before the lifecycle is settled.

## Tasks

### T1: New wards receive a complete persistent-agent doctrine

**Depends on:** none

**Tests:**
- Extend `scaffolds_generic_agent_instructions_without_assuming_directory_roles`
  to assert every persona section and a readable ward-specific identity.
- Assert the scaffold still contains no concrete `src/`, `data/`, `reports/`,
  `output/`, spec, plan, or task placement rules.

**Approach:**
- Add a small Ward-ID display-name formatter beside the existing scaffold logic.
- Replace the minimal `AGENTS.md` body with the complete persona contract.

**Done when:** focused `gateway-services` Ward-layout tests pass.

### T2: Synthesized Ward agents retain the complete doctrine

**Depends on:** T1

**Tests:**
- Extend Ward instruction-composition coverage with persona and
  self-maintenance sections.
- Preserve the existing empty-doctrine behavior.
- Use a temporary scaffolded ward as the reproducible fixture; do not depend on
  developer-local vault content.

**Approach:**
- Keep `AGENTS.md` as the sole persistent Ward doctrine; avoid a parallel agent
  config or persistence model.

**Done when:** focused `gateway-execution` Ward-agent tests pass.

### T3: The explicitly selected live generated scaffold exercises the new behavior

**Depends on:** T1, T2

**Tests:**
- Goal-based check only: if the active `financial-analysis/AGENTS.md` still
  exactly matches the known generated scaffold, replace it and verify the new
  section headings. Repository correctness remains covered by T1 and T2.

**Approach:**
- Replace only the known generated scaffold in the active test ward; do not
  rewrite arbitrary customized ward files.

**Done when:** the active Ward-agent doctrine is populated and repository gates pass.
