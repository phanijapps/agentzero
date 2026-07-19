## Blockers

**1. Approved-only resume boundary.** `docs/specs/autonomy-ledger/spec.md:80`. A high-confidence active match or review request could start execution without server-verifying `approved`. Fix: require an explicit user-selected item, verify `state == approved` after loading it, and keep review read-only.

**2. Trusted packet handoff.** `docs/specs/autonomy-ledger/plan.md:65`. The prior plan assumed a runtime context channel that does not exist and did not identify the source of truth for packet construction. Fix: add a dedicated typed packet from the parameter-loaded item through execution configuration into transient system context; do not trust message text, metadata, model output, or a client packet.

**3. Packet bounds and failure behavior.** `docs/specs/autonomy-ledger/plan.md:58`. The packet had no schema, numerical limits, or fail-closed behavior. Fix: specify fixed fields, byte/reference limits, label/transcript exclusion, strict DTO validation, and no executor mutation or retry when lookup, audit, or serialization fails.

**4. Read-only timer eligibility.** `docs/specs/autonomy-ledger/spec.md:91`. Eligibility did not state the invariant for approved items or all side effects that must be absent. Fix: define a state/policy matrix and prove the projection neither runs/enqueues nor changes lifecycle, timestamps, audit runs, tools, filesystem, or external services.

**5. Mission Control resume target.** `docs/specs/autonomy-ledger/plan.md:156`. The existing panel had no evidence detail or ledger-specific resume behavior, and generic session resume cannot carry a ledger packet. Fix: show evidence detail/count, make Resume an explicit ledger action, and require a newly created target session for the source agent.
