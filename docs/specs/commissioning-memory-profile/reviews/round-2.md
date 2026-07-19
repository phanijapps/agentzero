## Concerns

**1. Activation conflicts still masquerade as another pending restart in the recovery UI.** `apps/ui/src/features/commissioning/CommissioningScreen.tsx:223`. The backend returns `memory_profile_conflict` with `restartRequired: false`, but the activation check tells every non-complete result to restart again. Fix: branch on the conflict recovery code, render a finite actionable conflict message, and add a component test.
