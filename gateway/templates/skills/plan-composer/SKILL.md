---
name: plan-composer
description: Second-pass planning skill. Reads the spec produced by spec-builder and decomposes each phase into one briefing-style step file per step. Every step file declares Goal, Skills, Inputs (reusable + domain), Outputs (reusable + domain), BDD Acceptance, Depends on, Parallel-safe, Risk tier, optional Guardrail. Pre-classifying inputs and outputs by bucket (reusable vs domain) is load-bearing — it lets ward-designer assign exact paths and lets the executing subagent place artifacts correctly without improvisation. Also returns the spec with its Steps section filled in.
metadata:
  version: "2.0.0"
---

# plan-composer — second-pass planning

This is the second of two skills the planner loads. Input: the spec written by `spec-builder`. Output: that same spec with the `## Steps` section populated, plus one briefing per step.

## What this skill returns

A single text blob of filename-delimited sections:

````
=== FILE: spec.md ===
<full updated spec, ## Steps populated>

=== FILE: step_1.md ===
<step 1 briefing>

=== FILE: step_2.md ===
<step 2 briefing>
...
````

Emit the spec first, then steps in ID order. For `delta` classification, emit only the step files that changed — the full updated spec is emitted regardless.

This skill does NOT write files and does NOT respond to the user.

## Inputs you read

- The full spec from `spec-builder`. Focus on: `Classification`, `Suggested skills`, `Additional skills required`, `Raw tools required`, `Capability gaps`, `Approach`, `Phases`, `Phase dependency graph`, `Success criteria`.
- `prompt`, `intent`, `context`, `state` for reference.

## Halt clauses

If the spec's `Classification` is `CLASSIFY_FAILED`, `blocked`, or `decomposition`, do not decompose. Emit:

````
=== FILE: halt.md ===
plan-composer halt: spec classification '<value>' is not decomposable. No steps produced.
````

## Step briefing template

Produce exactly this structure for every step. No implementation bodies. No per-tool arguments. No prose placeholders.

````markdown
# Step <N> — <short title>

## Goal
<one paragraph: what this step achieves and why it matters to the plan>

## Skills
- <skill_name>: <what it covers in this step>
- ("none — use raw tools" if no skill fits)

## MCPs
- <canonical_mcp_id>: <why this step needs its tools>
- ("none" if the step needs no MCP)

## Reusable inputs (import from ward root)
- `<module_root>/<module>.<symbol>` — <signature + what it returns>
- `templates/<name>.<ext>` — <purpose>
- ("none" if the step has no reusable inputs)

## Domain inputs (prior-step outputs or context paths)
- step_<M>.output `<sub-domain>/data/<file>` — <schema>
- context path `<path>` — <what it provides>
- ("none" if the step has no domain inputs)

## Reusable outputs (land under ward root; register in memory-bank/core_docs.md)
- `<module_root>/<file>.<ext>` — <function signature + purpose>
- `templates/<name>.<ext>` — <what it templates>
- `snippets/<name>.<ext>` — <what the snippet provides>
- `shared-docs/<name>.md` — <content>
- `data/<name>.<ext>` — <shared reference dataset>
- ("none" if the step produces nothing reusable)

## Domain outputs (land under <sub-domain>/)
- `<sub-domain>/code/<file>` — <entry script; what it hardcodes>
- `<sub-domain>/data/<file>` — <schema>
- `<sub-domain>/reports/<file>` — <deliverable>
- ("none" if the step produces nothing domain-specific)

## Acceptance (BDD)

```gherkin
Given <preconditions>
When <this step runs>
Then <verifiable observation>
 And <more observations>
```

## Depends on
- Step <M>: <what it provides>
- ("none" if root)

## Parallel-safe with
- Step <K>, Step <L>
- ("none — must run alone in its branch")

## Risk tier
<low | medium | high>

## Guardrail (high-risk only)
<halt / retry-with-adjustment / ask-user guidance>
````

Steps are **briefings** for sub-agents, not recipes. You declare *what* (outputs, placement bucket) and *when-done* (BDD acceptance). The sub-agent decides *how*. Ward-designer then assigns exact ward-relative paths in its Paths table.

### Bucket rules — use these when classifying each input and output

Reusable bucket when the artifact is:
- A function / class / constant / module potentially callable from another sub-domain.
- A template / schema / snippet / shared markdown fragment.
- A shared reference dataset used across sub-domains.

Domain bucket when the artifact is:
- A script that hardcodes this sub-domain's inputs.
- Computed data for this sub-domain only.
- A report / plot / table for this sub-domain only.

Default when ambiguous: **reusable**. A ward that hoards assets per sub-domain is not reusable.

## Risk-tiered detail

Scale richness to the step's risk:

- **low** — read-only, reversible work. Terse 1-2 line fields. Default tier.
- **medium** — reversible but nontrivial, or produces output another step depends on. BDD Acceptance must have ≥ 2 observable Then/And checks.
- **high** — external writes (commit, publish, send), destructive operations, anything touching production. MUST include `## Guardrail`.

When in doubt between `medium` and `high`, choose `high`.

## Decomposition rules per classification

| Classification | Step shape |
|---|---|
| `skill_match` | Exactly 1 step. `Goal` = the ask. `Skills` = the matching skill. `Acceptance` = the spec's `## Success criteria` re-expressed in BDD. |
| `research` | Gather step(s) → analyze step → synthesize step. Gather steps are `parallel-safe` with one another. Analyze depends on all gathers. Synthesize depends on analyze. |
| `build` | Step 1 is always the ward-setup step (`Skills: ward-designer`). Fill steps follow (parallel where independent). Synthesis step(s) near the end. Format-convert step ONLY if the verbatim prompt used an explicit format word. |
| `delta` | Diff old phases vs. new. Unchanged phases keep existing step files. Changed phases get re-decomposed in place. New phases get new step files. Do not renumber. |

### Step 1 for `build` — the ward-setup step

For `build` classification, step_1.md is always the ward-setup step. Its content:

- `Goal`: one sentence — "Set up (or review) the `<ward>` ward for sub-domain `<sub_domain>`."
- `Skills`: `ward-designer`
- `Reusable inputs`: none (ward-designer reads the spec directly).
- `Domain inputs`: none.
- `Reusable outputs`: `AGENTS.md` (if missing), `memory-bank/ward.md` (if missing), `memory-bank/structure.md` (if missing), `memory-bank/core_docs.md` (if missing).
- `Domain outputs`: none.
- `Acceptance (BDD)`:
  ```gherkin
  Given the ward may or may not already exist
  When ward-designer runs
  Then wards/<ward>/AGENTS.md, memory-bank/ward.md, memory-bank/structure.md, memory-bank/core_docs.md all exist with non-trivial content
   And every subsequent step file has a ## Paths table assigning exact ward-relative paths to its outputs
   And no .py/.js/.go/.ts/.rs source file was written by this step
  ```
- `Depends on`: none.
- `Parallel-safe with`: none — must complete before any other step.
- `Risk tier`: `low`.

Ward-designer owns the Paths-table assignment for step_2..step_N. Plan-composer does not pre-assign exact paths — it pre-classifies into buckets.

## Update the spec's `## Steps` section

Replace `## Steps — PENDING (plan-composer fills this)` with:

````markdown
## Steps

1. [Step 1](step_1.md) — <title> — skills `<list>` — MCPs `<canonical-id list | none>` — <one-line goal>
2. [Step 2](step_2.md) — <title> — skills `<list>` — MCPs `<canonical-id list | none>` — <one-line goal>
...

## Step dependency graph

```
1 ──▶ 2
1 ──▶ 3
2, 3 ──▶ 4
```
````

Dependency graph is ASCII. Every step depends on step 1 at minimum (ward-setup) for `build` plans.

## Self-critique gate

Before returning, check every item. Revise and re-check until every item passes:

- Every phase from the spec has ≥ 1 step.
- Every step has non-empty `Goal`, `Skills`, `MCPs`, at least one of `Reusable outputs` / `Domain outputs` populated, and a BDD `Acceptance` block with Given / When / Then.
- Every step's `Skills` list contains only real skill names from the spec's Suggested skills, Additional skills required, or planner catalog. **No invented skill names.**
- Every step's `MCPs` list contains only canonical IDs returned by `lookup_capabilities` or the planner catalog. Use `none` rather than an invented MCP or a display name.
- Every entry in `Reusable inputs` / `Domain inputs` / `Reusable outputs` / `Domain outputs` references a real path shape — no placeholder like `<TBD>`.
- Every `Depends on` entry references a real step id.
- `Parallel-safe with` is consistent with the dependency graph — a step is not parallel-safe with an ancestor or descendant.
- Every step has a risk tier. `high` tier steps have a `Guardrail`.
- BDD Acceptance has ≥ 2 observable Then/And clauses for `medium`/`high` steps.
- For `build`: step_1.md is the ward-setup step with `Skills: ward-designer`.
- For `skill_match`: exactly 1 step.
- For `build` / `research`: ≥ 2 steps.
- No cycles in the dependency graph.

## Graceful degradation

If the spec is partially complete (e.g., missing phase dependency graph), infer the missing structure from `## Approach` and `## Phases`. Do not halt. Note inferred structure in the updated spec's `## Inputs snapshot`.

## Shared conventions (from spec-builder)

- **Verbatim-prompt primacy.** On conflict between `prompt` and other inputs, trust `prompt`. Note in the updated spec's `## Assumptions`.
- **Capability cascade.** Unclear skill fit → walk: `suggested_skills` → planner catalog → raw tools → declare `Capability gap`. No invented skill names.
- **Markdown-first output.** A format-convert step (HTML / PDF / PPT / docx) is emitted ONLY when the verbatim `prompt` used that exact format word. "Report" alone defaults to markdown.

## References

- `references/step-examples.md` — one worked example per step shape (research gather, research synthesize, code fill, browser flow, high-risk write, writing synthesize). Load when unsure how to structure a particular step.
