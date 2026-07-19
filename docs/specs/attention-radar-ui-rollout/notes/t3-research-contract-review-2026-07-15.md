# T3 Research contract review — 2026-07-15

## Blockers

- `docs/product/attention-radar-ui-gallery.html:298` — The Research masthead
  introduces a `Vault` button even though ward context is embedded in
  `WardVaultExplorer`; this implies a duplicate action. **Fix:** remove it.
- `docs/product/attention-radar-ui-gallery.html:301` — A separate,
  non-interactive Goal deliverables rail duplicates the existing interactive
  artifact strip and preview path. **Fix:** represent the existing artifact
  strip once, in its current functional position.

## Concerns

- `docs/specs/attention-radar-ui-rollout/plan.md:141-148` — The Research
  checks did not make the central ward and artifact behavior explicit.
  **Fix:** name the no-ward/no-fetch, ward scope, Vault preview, artifact
  preview, and AG-UI preservation checks.
- `docs/product/attention-radar-ui-gallery.html:301` — The mock exposed
  unsupported counters and activity numbers. **Fix:** remove them rather than
  fabricate a source or empty-state rule.
