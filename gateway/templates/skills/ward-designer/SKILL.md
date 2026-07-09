---
name: ward-designer
description: Set up or review a build ward at the ward-setup step. Use when a builder-agent must create missing ward doctrine, bootstrap ward directories, and add exact ward-relative Paths tables to later steps so subagents cannot improvise artifact locations.
---
# ward-designer
Run once at the `build` ward-setup step.
Do exactly two jobs:
1. Seed missing ward doctrine files.
2. Add exact `## Paths` tables to later step files.
Do not write source code. Do not answer the user. Emit FILE/SHELL blocks for the orchestrator to apply under `wards/<ward>/`.
## Inputs
- `wards/<ward>/specs/<sub_domain>/spec.md`
- `wards/<ward>/specs/<sub_domain>/step_*.md`
- Existing `AGENTS.md` and `memory-bank/{ward.md,structure.md,core_docs.md}`
## Halt
Emit `=== FILE: halt.md ===` with one-line reason when classification is not `build`/build-`delta`, or `spec.md` is missing.
If only the ward-setup step exists, emit `=== FILE: memory-bank/phase2-skipped.md ===`.
## Module Root
Derive `module_root` from the spec language:
| Language | module_root |
|---|---|
| python | `core/` |
| nodejs | `src/lib/` |
| go | `pkg/` |
| r | `R/` |
| perl | `lib/` |
| rust | `src/` |
## Phase 1: Doctrine
Check `AGENTS.md`, `memory-bank/ward.md`, `memory-bank/structure.md`, and `memory-bank/core_docs.md`.
If a file exists and has non-trivial content, do not rewrite or augment it. Existing ward doctrine is authoritative.
Emit only missing or empty files.
### AGENTS.md seed
```markdown
# <ward-name>
## Purpose / Scope
IN — <reusable domain this ward owns, not a single task>.
OUT — <adjacent domains this ward does not handle>. Return out_of_scope.
## Folder Map
- specs/<sub-domain>/  spec and step files
- <sub-domain>/        code/ data/ reports/ summary.md manifest.json
- <module_root>/       reusable primitives
- data/                shared reference data
- memory-bank/         ward.md structure.md core_docs.md
## Standards
- Reusable primitives live in <module_root>/, take arguments, and avoid hardcoded task values.
- Register reusable primitives in memory-bank/core_docs.md.
- Fix original files; do not create _v2 variants.
- Do not fabricate data or expand beyond IN scope.
## Handoff
Return: { status, summary, artifacts:[paths] }
```
### memory-bank/ward.md seed
```markdown
# Ward: <name>
<one-paragraph purpose>
## Sub-domains
| Slug | Description | Status |
|---|---|---|
| <current> | <current deliverable> | in progress |
## Key Concepts
- <term> — <one-line definition>
```
### memory-bank/structure.md seed
```markdown
# Structure — <ward name>
<ward>/
├── AGENTS.md
├── memory-bank/{ward.md,structure.md,core_docs.md}
├── <module_root>/      reusable code primitives
├── templates/ snippets/ shared-docs/ data/
├── <sub-domain>/{code,data,reports,summary.md,manifest.json}
└── specs/<sub-domain>/
```
### memory-bank/core_docs.md seed
```markdown
# Core docs — <ward name>
| Symbol | Module | Signature | Purpose | Added by |
|---|---|---|---|---|
| _(none yet)_ | | | | |
```
## Directory Bootstrap
Always emit:
```text
=== SHELL: directory-bootstrap ===
mkdir -p <module_root> templates snippets shared-docs data
mkdir -p <sub_domain>/{code,data,reports}
test -f <sub_domain>/summary.md || echo "# <sub_domain>" > <sub_domain>/summary.md
test -f <sub_domain>/manifest.json || echo '{"sub_domain":"<sub_domain>","produced_at":"<YYYY-MM-DD>","files":[]}' > <sub_domain>/manifest.json
```
## Phase 2: Paths
List `specs/<sub_domain>/step_*.md`. Skip the ward-setup step whose `Skills` or `Suggested skill` is `ward-designer`.
For every later step, preserve existing content and add:
```markdown
## Paths (assigned by ward-designer — do not deviate)
| Artifact | Bucket | Exact ward-relative path | Register in core_docs.md? |
|---|---|---|---|
| <filename> | reusable | <module_root>/<subpath> | yes |
| <filename> | reusable | templates/<name>.<ext> | yes |
| <filename> | reusable | snippets/<name>.<ext> | yes |
| <filename> | reusable | shared-docs/<name>.md | yes |
| <filename> | reusable | data/<name>.<ext> | yes |
| <filename> | domain | <sub_domain>/code/<subpath> | no |
| <filename> | domain | <sub_domain>/data/<subpath> | no |
| <filename> | domain | <sub_domain>/reports/<subpath> | no |
```
Placement rule: reusable when another sub-domain could use it; domain when it hardcodes this sub-domain's inputs or produces this sub-domain's deliverable.
Default ambiguous artifacts to reusable.
Every `yes` row must map to a step output and must be registered in `memory-bank/core_docs.md` by the executing subagent.
## Path Buckets
Use these buckets when translating step outputs:
- reusable code: functions, classes, constants, importable modules
- reusable templates: durable output formats or report shells
- reusable snippets: small copyable code fragments
- reusable shared-docs: markdown fragments reused across reports
- reusable data: shared reference datasets
- domain code: thin scripts hardcoding this sub-domain's inputs
- domain data: computed data for this sub-domain only
- domain reports: plots, tables, and narratives for this deliverable
## Delta Mode
For build-delta, keep existing paths for unchanged steps.
Reassign only new or changed step outputs.
Do not renumber step files.
## Return Order
Emit all applicable blocks in one response:
```text
=== FILE: AGENTS.md ===
=== FILE: memory-bank/ward.md ===
=== FILE: memory-bank/structure.md ===
=== FILE: memory-bank/core_docs.md ===
=== SHELL: directory-bootstrap ===
=== FILE: specs/<sub_domain>/step_2.md ===
=== FILE: specs/<sub_domain>/step_N.md ===
```
## Self-Check
- Existing doctrine files were not rewritten.
- Every non-setup step has a Paths table.
- Every path is explicit: no placeholders, TBD, or alternative locations.
- Ambiguous outputs default to reusable.
- No source files were emitted by this skill.
- Every `Register: yes` row maps to an output in the same step.
