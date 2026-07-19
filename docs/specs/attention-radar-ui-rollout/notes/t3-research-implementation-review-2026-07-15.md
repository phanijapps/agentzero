# T3 Research implementation review — 2026-07-15

## Blockers

- `apps/ui/src/features/research-v2/research.css:282-296` — At narrow widths,
  the collapsed ward explorer switched back to a clipped grid while the page
  shell disallows overflow. **Fix:** retain normal vertical flow and verify a
  collapsed explorer leaves the thread, artifacts, and composer reachable.
