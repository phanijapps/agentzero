# T3 Research implementation review — normalized record

## Blockers

**1. Collapsed narrow explorer clips the focus flow.** `apps/ui/src/features/research-v2/research.css:282` — The collapsed explorer returned to a clipped grid while the page shell disallows overflow, so a long thread could become unreachable. Fix: keep the collapsed narrow layout in normal vertical flow and verify the expand control, thread, artifacts, and composer remain reachable.
