## Blockers

**1. Legacy marker parsing is now gated behind `present_surface`.** `runtime/agent-runtime/src/executor.rs:1199`. The surface guard also enclosed unrelated legacy marker parsing. Fix: restrict only work-surface markers and preserve the general marker path.

## Concerns

**2. Plan status uses a non-convention value.** `docs/specs/automatic-work-surfaces/plan.md:4`. `Complete` is not a supported plan status. Fix: use `Done`.
