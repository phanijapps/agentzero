# Spec: YFinance Market Analysis Skill Consolidation

- **Status:** Shipped

Mode: light (content/template consolidation; no runtime code path changed)

## Objective

Consolidate the six bundled yfinance skills into a single primary skill named
`yfinance-market-analysis`, while keeping the old `yf-*` skill IDs available as
compatibility wrappers during the transition.

## Acceptance Criteria

- [x] A bundled `gateway/templates/skills/yfinance-market-analysis/SKILL.md`
  exists with frontmatter name `yfinance-market-analysis`.
- [x] The new skill covers data collection, technical signals, fundamentals,
  catalysts, options, and portfolio risk workflows.
- [x] The detailed reference files from the old yfinance skills are available
  under the new skill.
- [x] The old `yf-*` skill folders remain loadable but direct agents to prefer
  `yfinance-market-analysis`.
- [x] Prompt examples stop recommending multiple separate yfinance skills for
  new work.
- [x] Skill validation and targeted repository checks pass.

## Tasks

1. Add the aggregate `yfinance-market-analysis` skill and references.
2. Replace old `yf-*` skill bodies with legacy wrapper instructions.
3. Update prompt examples and seed comments that teach old skill names.
4. Run validation, targeted tests, and typecheck.
