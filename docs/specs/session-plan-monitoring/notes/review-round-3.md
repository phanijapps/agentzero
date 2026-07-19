# Plan review — round 3

**1. The server-issued order counter lacks restart durability.** `docs/specs/session-plan-monitoring/plan.md:78-89`. The session-registry counter resets if the gateway restarts while `session_plans.source_event_sequence` persists. New valid updates can then have lower sequences than the stored row and be rejected indefinitely. Fix: allocate the sequence durably and atomically, or rehydrate the registry counter from the persisted maximum before accepting events. Add a restart/recovery test proving the first post-restart valid update replaces the prior snapshot.

**2. The proposed sequence has no durable allocation/recovery protocol.** `docs/specs/session-plan-monitoring/plan.md:80-89`. The in-memory counter has no atomic allocation across concurrent contexts or restart recovery. Fix: persist and atomically allocate the per-session next sequence, initialize it from existing snapshot state during migration, and add restart plus concurrent-allocation tests.
