# RFC-0005: Builder Delegation and Ward Context Hygiene

- **Status:** Accepted
- **Author:** phanijapps
- **Approver:** phanijapps
- **Date opened:** 2026-06-02
- **Date closed:** 2026-06-02
- **Related:** `docs/specs/builder-delegation-hygiene/`; `docs/specs/subagent-role-gating/`; `docs/architecture/components/subagent-capability-policy/`; `docs/architecture/components/execution-loop/data-flow.md`; `gateway/templates/agents/builder-agent.md`; `gateway/templates/agents/planner-agent.md`

## The Ask

Approve explicit delegation modes for builder-style subagents:
`direct_artifact`, `ward_hygiene`, `ward_backed_build`, and `step_executor`.
The runtime should carry the mode as metadata and prepend mode-specific executor
rules, while preserving the existing ward-as-agent full-tool policy.

Root can already delegate directly to `builder-agent`. That is useful, but the
same builder prompt currently has to serve very different jobs: exact standalone
artifact creation, ward doctrine cleanup, ward-backed implementation, and
planned step execution. Prompt-only distinction makes direct artifact tasks too
heavy and makes root-bypassed ward hygiene unreliable.

Decisions requested:

1. Add a runtime delegation mode field. Recommended: accept, because prompt
   text alone is not a durable contract.
2. Keep actor capability policy separate from mode. Recommended: accept,
   because reviewer/ward safety is about tools while delegation mode is about
   execution posture.
3. Refresh existing live default agents only when they still match known old
   bundled template signatures. Recommended: accept, because stale prompts need
   repair without overwriting user edits.

## Problem & Goals

Root-to-builder delegation has two failure modes:

- For exact standalone artifacts, builder can waste tokens reading ward/root
  documentation before writing the named output.
- For ward setup or direct ward hygiene, builder can skip filling
  `AGENTS.md` and `memory-bank/*` because the planner was bypassed.

Goals:

- Give root, planner, and runtime a small shared vocabulary for delegated
  builder posture.
- Keep `direct_artifact` lean: write named outputs first, verify, report paths.
- Make `ward_hygiene` explicit: fill missing or empty doctrine files and
  preserve non-empty content.
- Make `ward_backed_build` explicit: read the supplied ward snapshot and
  relevant ward files before coding.
- Make `step_executor` explicit: execute a spec/plan step with acceptance
  checks.
- Update bundled builder/planner prompts and safely repair stale live defaults.

Non-goals:

- No new database migration or persisted delegation-mode column.
- No user-facing `allowedTools`/`deniedTools` agent configuration.
- No change to ward-as-agent full-tool behavior.
- No broad live-agent prompt rewrite for customized agents.

## Proposal

Add `DelegationMode` as a runtime enum with four wire values:

| Mode | Use |
| --- | --- |
| `direct_artifact` | Exact-output standalone artifact work. |
| `ward_hygiene` | Fill missing/empty ward doctrine and memory-bank files. |
| `ward_backed_build` | Implementation that depends on ward conventions or reusable structure. |
| `step_executor` | Execute a decomposed spec/plan step. |

Expose `mode` as an optional `delegate_to_agent` argument. Validate it at the
tool boundary, carry it through stream events, `DelegationRequest`,
`DelegationContext`, spawn, and child executor initial state as
`app:delegation_mode`.

If root omits `mode`, infer conservatively:

- Step-spec markers imply `step_executor`.
- Ward setup/hygiene wording implies `ward_hygiene`.
- Exact output path plus self-contained/standalone wording implies
  `direct_artifact`.
- Everything else defaults to `ward_backed_build`.

Make `subagent_rules` mode-aware for executor subagents. Keep reviewer rules and
actor capability policy independent from delegation mode.

Update bundled `builder-agent` and `planner-agent` templates to describe the
four modes and use live/default seeded agent names. Existing live default
builder/planner agents refresh only when normalized `AGENTS.md` content hashes
match known old bundled template signatures; refresh writes a timestamped backup
before replacement.

## Options Considered

Axis: where the delegated work posture is represented. These options exhaust
the reasonable implementation locations: nowhere, prompt text only, runtime
metadata, persisted schema, or tool capability policy.

| Option | Trade-off |
| --- | --- |
| Do nothing | No implementation cost, but direct builder delegation remains unreliable and stale prompts remain live. |
| Prompt-only wording | Easy to ship, but root/planner/model text remains the only contract and cannot be reliably tested across the runtime path. |
| Runtime metadata plus mode-specific rules | Small, testable, no migration, and keeps capability policy separate. This is the accepted option. |
| Persist mode in the database | Useful for long-term analytics, but unnecessary for this behavior and creates migration surface before the need is proven. |
| Encode posture as tool capability policy | Conflates what an actor may do with how it should approach a task; would risk constraining ward agents incorrectly. |

Prior art inside the repo already supports the accepted option: actor kind is
runtime metadata (`app:actor_kind`) while prompts describe behavior. Delegation
mode follows the same pattern but controls execution posture, not authority.

## Risks & What Would Make This Wrong

- Inference can misclassify ambiguous builder tasks. Mitigation: explicit mode
  wins, and ambiguous work falls back to `ward_backed_build`.
- Live-agent refresh could overwrite customization. Mitigation: refresh only
  known old normalized template hashes and write a backup first.
- `direct_artifact` could skip useful context. Mitigation: it only applies to
  exact-output self-contained tasks; other implementation work remains
  ward-backed.
- Prompt size could grow. Mitigation: mode text replaces generic docs-first
  rules instead of adding another broad rule layer.

This RFC would be wrong if builder tasks routinely need full ward doctrine even
for exact standalone outputs, or if mode inference proves too noisy for common
root delegation wording. Both can be corrected by explicit mode selection and
tighter inference tests.

## Evidence & Prior Art

Repo evidence:

- `delegate_to_agent` previously had no explicit posture field and could only
  encode this distinction in task prose.
- `DelegationRequest` previously carried task/context/skills/complexity but no
  mode.
- Existing executor rules had a catch-all docs-first rule for implementation
  subagents.
- Ward creation already seeds `AGENTS.md` plus `memory-bank/` files that agents
  are expected to curate.
- Default-agent seeding skipped existing live agents, so stale bundled prompts
  needed a guarded refresh path.

No external prior art is required for this small internal runtime contract; the
important precedent is the repo's existing split between runtime actor metadata
and prompt-level behavior.

## Open Questions

None for this implementation slice. Future hardening can add analytics,
manual mode override UX, or richer read-only ward/memory tools if evidence
shows a need.

## Follow-On Artifacts

- Spec: `docs/specs/builder-delegation-hygiene/`
- Component docs: `docs/architecture/components/subagent-capability-policy/overview.md`
- Component docs: `docs/architecture/components/execution-loop/data-flow.md`
