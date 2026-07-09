# Subagent Capability Policy - Component Overview

## What It Is

Subagent capability policy is the runtime boundary that decides which first-party
tools an execution actor receives. It replaces the old binary split of
`root` versus `delegated subagent` with explicit actor kinds:

| Actor kind | Meaning |
|------------|---------|
| `Root` | Top-level orchestrator for the session. |
| `DelegatedExecutor` | Ordinary implementation subagent. |
| `DelegatedReviewer` | Ordinary read-only review subagent. |
| `WardAgent` | Warm `ward:<name>` agent synthesized from a graduated ward. |

The policy lives in `gateway/gateway-execution/src/invoke/executor.rs` and is
applied when `ExecutorBuilder::build_tool_registry()` constructs the tool
registry. Prompts still describe behavior, but prompts are not the safety
boundary.

Delegation mode is separate from actor kind. Actor kind controls tool
authority; delegation mode controls the posture an executor should take for the
task. For example, a `DelegatedExecutor` may run in `direct_artifact`,
`ward_hygiene`, `ward_backed_build`, or `step_executor` mode while keeping the
same implementation-capable tool inventory.

## Why It Exists

Reviewer behavior used to be prompt-only. `SubagentRole::Reviewer` changed the
rules text, but delegated reviewers still received the same mutation-capable
tool inventory as executors. That meant a reviewer could still run shell
commands or edit files if the model chose to.

The capability policy makes the boundary executable:

- reviewers are read-only;
- executor subagents can still implement;
- ordinary delegated agents cannot orchestrate more agents;
- ward-as-agent stays a full-tool actor and is not accidentally constrained by
  reviewer hardening.

## Actor Profiles

### Root

Root keeps orchestration and session-control tools. It can delegate, update the
plan, set the session title, use memory/ward tools, run shell, query resources,
and respond. Root does not get implementation file-write tools by default
(`write_file`, `edit_file`) because root is expected to delegate specialist
implementation work.

### DelegatedExecutor

Delegated executors keep implementation tools:

- `shell`
- `write_file`
- `edit_file`
- `grep`
- `ward`
- `memory`
- `load_skill`
- `list_skills`
- `list_mcps`
- `respond`
- `multimodal_analyze`

They do not get orchestration/control tools such as `delegate_to_agent`,
`wait_agent`, `kill_agent`, or `steer_agent`.

### DelegatedReviewer

Delegated reviewers are read-only in V1:

- `read`
- `glob`
- `grep`
- `load_skill`
- `list_skills`
- `list_mcps`
- `respond`
- `multimodal_analyze`

They do not get `shell`, `write_file`, `edit_file`, `ward`, `memory`, or agent
orchestration/control tools.

Current `ward` and `memory` tools are mixed read/write tools, so they require
write capability and are excluded from reviewers until read-only variants exist.

### WardAgent

Warm ward agents are created by delegating to `ward:<name>`. They are their own
actor kind, not ordinary delegated executors or reviewers.

Ward agents retain full first-party tool access for their execution context,
including implementation tools and orchestration/control tools when the backing
services are wired. This is intentional: a graduated ward is an agent-like
workspace with its own doctrine, procedures, memory, and operating context.

Important: `WardAgent` is spawned through delegation but is not marked with the
ordinary `app:is_delegated` flag. Some tools use that flag to restrict ordinary
subagents, especially memory context writes and plan size. Ward agents must not
inherit those ordinary-subagent restrictions.

## Delegation Routing

`gateway/gateway-execution/src/delegation/spawn.rs` classifies each delegated
execution before building the executor:

1. If `child_agent_id` starts with `ward:`, classify as `WardAgent`.
2. Otherwise run `detect_subagent_role(child_agent_id, task)`.
3. Reviewer identity or explicit read-only review intent becomes
   `DelegatedReviewer`.
4. Known execution agents and tasks with execution verbs such as fetch, scrape,
   write, run, parse, extract, generate, or build become `DelegatedExecutor`.
5. Other ordinary delegated tasks default to `DelegatedExecutor`.

This ordering matters. A task like "review this" sent to `ward:finance` remains
`WardAgent`; review wording must not downgrade a ward to read-only reviewer
policy.

Generic quality words such as verify, validate, or evaluate are not enough to
make an execution agent read-only. Data and build tasks often need to verify
their own outputs while still retaining shell and file-write tools.

## Runtime State

The executor injects actor metadata into initial state:

| State key | Actors | Meaning |
|-----------|--------|---------|
| `app:actor_kind` | all actors | One of `root`, `delegated_executor`, `delegated_reviewer`, `ward_agent`. |
| `app:tool_capabilities` | all actors | Capability strings allowed for the actor. |
| `app:delegation_mode` | delegated children | Execution posture such as `direct_artifact`, `ward_hygiene`, `ward_backed_build`, or `step_executor`. |
| `app:is_delegated` | ordinary delegated executor/reviewer only | Compatibility flag used by existing tools to enforce ordinary subagent constraints. |
| `app:subagent_role` | ordinary delegated executor/reviewer only | `executor` or `reviewer`. |

Future tools should prefer `app:actor_kind` and capability state over inferring
authority from agent names or prompts. They may inspect `app:delegation_mode`
for behavior choices, but must not treat it as a tool-authorization boundary.

## Reviewer Agent Identity

The product-level reviewer identity is seeded through bundled templates:

- `gateway/templates/default_agents.json`
- `gateway/templates/agents/reviewer-agent.md`

On a normal daemon install this seeds into:

```text
~/Documents/zbot/agents/reviewer-agent/
```

The identity improves routing and UX, but enforcement still comes from
`RuntimeActorKind::DelegatedReviewer` and the tool capability policy.

## Tests

The policy is pinned by focused tests:

| Behavior | Test location |
|----------|---------------|
| Executor inventory keeps implementation tools and excludes orchestration | `gateway/gateway-execution/src/invoke/executor.rs` |
| Reviewer inventory is read-only and non-orchestrating | `gateway/gateway-execution/src/invoke/executor.rs` |
| Root keeps orchestration without implementation file writes | `gateway/gateway-execution/src/invoke/executor.rs` |
| Ward agent gets root + executor first-party tools | `gateway/gateway-execution/src/invoke/executor.rs` |
| Ward agent is not marked as ordinary `app:is_delegated` | `gateway/gateway-execution/src/invoke/executor.rs` |
| `ward:<name>` classification wins over review wording | `gateway/gateway-execution/src/delegation/spawn.rs` |
| `reviewer-agent` seeds through `AgentService` | `gateway/gateway-services/src/agents.rs` |
| Delegation mode inference and mode-specific executor rules | `gateway/gateway-execution/src/delegation/context.rs`, `gateway/gateway-execution/tests/e2e_ward_pipeline_tests.rs` |

## Change Rules

- Do not add user-facing `allowedTools` or `deniedTools` fields without a
  separate spec.
- Do not give reviewers shell, file mutation, ward mutation, memory mutation, or
  orchestration tools in V1.
- Do not collapse `WardAgent` into ordinary delegated executor/reviewer policy.
- When adding a first-party tool, assign capabilities and update the actor
  inventory tests in the same change.
