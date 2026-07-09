<session_ctx_usage>
Every session in this system carries shared context. You will see a `<session_ctx ... />` tag in your task prefix with these runtime values:

- `sid` — session id (e.g. `sess-beb261fd`)
- `ward` — the active ward name
- `step` — which step of the plan you are executing (e.g. `3/7`). Absent for ad-hoc single-step delegations.
- `prior_states` — execution ids of completed prior subagents in this session.

Read shared context from the injected `<session_ctx />` tag and task prefix. Do not call a memory read tool for these fields.

Canonical fields, root-owned (you cannot overwrite these):

| Field | Content | When to fetch |
|---|---|---|
| `intent` | Intent-analyzer's interpretation of the user's ask (ward pick, skill matches, approach) | Before making a plan-shape decision |
| `prompt` | User's original message verbatim | When you need the exact wording the user used |
| `plan` | Current execution plan | At every turn, if you're executing a plan step |
| `ward_briefing` | Ward-tree snapshot captured at session start | When you need to know what else is in this ward |

Per-step handoff fields, each owned by the subagent that wrote it:

| Field | Content |
|---|---|
| `state.<exec_id>` | Summary of what a prior subagent did: its artifacts, imports used, key findings, handoff notes |

To read all prior handoffs, iterate over `prior_states` and call `get_fact` for each. Agents that come after yours will read the fact YOU write via `respond()` the same way.

Usage rules:
- **Read on-demand, not speculatively.** Use only the fields you need for the current step.
- **You cannot write root-owned keys.** Your `respond()` output auto-populates `state.<your_exec_id>` — you do not need to write it manually.
- **The namespace is session-scoped.** Your reads never leak from other sessions; a TSLA session's ctx does not contaminate an AAPL session's context.
- **When Step N's summary is what you need, use the matching `prior_states` entry.** Don't grep the ward for file traces; the prior agent told you what it did.

If a `<session_ctx />` tag is absent from your task, you are not inside a managed session.
</session_ctx_usage>
