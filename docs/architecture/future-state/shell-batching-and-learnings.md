# Shell Tool Batching + LEARNINGS.md

**Status**: design spec, not yet implemented
**Date**: 2026-04-23
**Audience**: engineers implementing against this plan
**Relationship**: first concrete scout of Layer 4 (file-backed memory surface) from `docs/architecture/future-state/compaction-strategy.md`. Adopt this plan's decisions before building Layer 4 generally.

---

## 1. Problem

Agents currently emit sequential shell tool calls where a single batched call would do. Evidence from the session DB at `~/Documents/zbot/data/conversations.db`:

- **93** assistant messages with `tool_calls` across recent sessions.
- **82 of 93 (88%)** are single-tool-per-turn; parallel tool_use is used only 12% of the time.
- **13 runs of 2+ consecutive `shell` calls** within the last 5 executions, totaling **60 shell calls** in those runs.
- Run lengths range from 2 to **9 consecutive calls**.
- Dominant commands inside these runs: `cat` (25), `cd` (15), `ls` (8), `python3 -c "..."` (8) — overwhelmingly **read-only / inspection** operations.

Each sequential shell call is a full LLM round-trip: the prefix is re-billed, the tools-schema is re-sent, a full assistant response is generated. A 9-call run costs approximately 9× what a batched call would. The agent has no signal in the current shell tool description or system prompt that batching is preferred.

Separately, when a shell script does fail, there is no mechanism by which one session's lesson informs the next. The agent reasons from scratch on every recurring failure.

## 2. Goals

1. **Reduce round-trips** for independent read/inspection operations by teaching the agent to batch them into single shell calls.
2. **Turn failures into durable, reusable knowledge** via a human-readable learnings file the agent reads on failure and writes to on resolution.
3. **Ship zero code in the shell tool itself** beyond one small error-path augmentation. The core change is prompt engineering and one new markdown shard.

## 3. Design decisions

Each decision below was made during the brainstorm captured in session state. Locked-in values.

| # | Question | Decision | Rationale |
|---|----------|----------|-----------|
| Q1 | Who controls batching — model or framework? | **Model-driven** | Framework coalescing is risky (model may branch on intermediate output); model-driven is explicit and auditable. |
| Q2 | New tool or extend existing shell? | **Extend existing shell** | The `command` param already accepts multi-line bash. A new tool adds cognitive overhead without behavioral gain. |
| Q3 | Code change or pure prompt? | **Pure prompt (plus one error nudge)** | Fastest win; shell tool description and a system shard teach batching with zero runtime change. One small augmentation to the tool error path adds the LEARNINGS pointer on failure. |
| Q4 | Default failure behavior? | **Continue-on-error + section markers** | Dominant DB pattern is independent reads (`cat X; cat Y; cat Z`). Single-pattern teaching yields higher compliance than "pick between `;` and `&&`." Agents can still write `set -e` when they need a dependent pipeline. |
| Q5 | Where does batching teaching live? | **System prompt shard + short description pointer** | Shard is always-loaded in the cached prefix; tool description is visible every turn and points at the shard. |
| Q6 | Scope of learnings? | **Global** | Shell-failure patterns (pip quirks, permission denied, missing paths) are agent-independent. Scoping by ward or agent fragments signal. |
| Q7 | Access mechanism for LEARNINGS.md? | **Raw `read_file` / `edit_file`** | Simplest possible. No new tool surface. The markdown file is human-readable, diff-visible, user-curatable. Semantic search is overkill at expected entry counts. |

## 4. Proposed behavior — end-to-end

### 4.1 Success path (batching)

The agent has three files to read. Instead of:
```
shell: cat step_1.md
shell: cat step_2.md
shell: cat step_3.md
```
it emits one call:
```
shell: echo "--- step_1 ---"; cat step_1.md; \
       echo "--- step_2 ---"; cat step_2.md; \
       echo "--- step_3 ---"; cat step_3.md
```

The `;` separator runs all three commands regardless of individual failures. The `echo` markers let the agent parse which output came from which file.

### 4.2 Failure without prior pattern

1. Batched shell call returns `exit_code != 0`.
2. The tool result carries a `hint` field telling the agent to consult `<vault>/wards/LEARNINGS.md`.
3. Agent calls `read_file` on LEARNINGS.md, scans for a relevant trigger.
4. No match found.
5. Agent reasons out a fix and re-runs.
6. On success, agent calls `edit_file` on LEARNINGS.md, appending a new `## Trigger:` section with the trigger text, the fix that worked, and contextual notes.

### 4.3 Failure with known pattern

1. Batched shell call returns `exit_code != 0`.
2. Tool result `hint` points at LEARNINGS.md.
3. Agent calls `read_file` on LEARNINGS.md, finds a matching `## Trigger:` section.
4. Agent applies the documented fix.
5. On success, optionally the agent bumps the entry (future work; see deferred).

## 5. File-by-file implementation map

| # | File | Change type | What goes here |
|---|------|-------------|----------------|
| 1 | `gateway/templates/shards/shell_batching.md` | **new file** | The batching + failure-recovery shard. See Section 6 for content outline. References LEARNINGS.md by absolute path via the `LEARNINGS:` runtime-info line (row 3). |
| 2 | `gateway/gateway-templates/src/lib.rs:22-27` | edit | Add `"shell_batching"` to `REQUIRED_SHARDS`. One line. |
| 3 | `gateway/gateway-templates/src/lib.rs:304-318` (`runtime_info`) | edit | Append a `LEARNINGS: <vault>/wards/LEARNINGS.md` line next to the existing `VAULT:` line. The agent sees its absolute learnings path literally; no string concatenation, no template substitution. |
| 4 | `runtime/agent-tools/src/tools/execution/shell.rs:352-354` (`description()`) | edit | Replace description with one sentence that teaches batching and points at the shard. Must remain under ~30 words since it renders every turn. |
| 5 | `runtime/agent-tools/src/tools/execution/shell.rs` (error-return path around line 570+) | edit | When `exit_code != 0`, add a `"hint"` field to the returned JSON. The hint names the absolute LEARNINGS.md path so the agent can read it without further lookup. |
| 6 | `gateway/gateway-services/src/paths.rs` (around line 40-70) | edit | Add `pub fn learnings_md(&self) -> PathBuf { self.vault_dir.join("wards").join("LEARNINGS.md") }`. Canonical helper; future code uses it rather than re-deriving. |
| 7 | `gateway/src/state.rs` (around `AppState::new`, line 154 area) | edit | On startup, if `paths.learnings_md()` does not exist, create it with the seed header from Section 6. Idempotent. |
| 8 | `<vault>/wards/LEARNINGS.md` | **new file (bootstrapped)** | Seeded by row 7 on first startup. Agents append to it. Human-editable and diff-visible. |

### Path-drift note

`shell.rs` today hardcodes `~/Documents/zbot/wards` in a few places (around line 517) instead of resolving via `SharedVaultPaths`. The nudge in row 5 should resolve the path through the same `dirs::document_dir().join("zbot")` pattern for consistency with existing code, **and** a follow-up refactor should route all vault-path resolution in shell.rs through `SharedVaultPaths`. That refactor is out of scope for this spec.

## 6. LEARNINGS.md format

### Seed file content (bootstrapped by row 7 above)

```markdown
# Shell Failure Learnings

Durable, global record of shell-failure patterns and their fixes.

- Agents read this file after a failed shell call to check for known patterns.
- Agents append a new H2 section after resolving a novel failure.
- One `## Trigger:` heading per pattern. Keep triggers specific enough to match; keep fixes concise.
- Humans may curate: delete stale entries, edit unclear fixes, merge near-duplicates.

Format per entry:

```
## Trigger: <short description of the failure signature>
**Fix**: <what to do>
**Context**: <when this applies / why it works>
**First seen**: <YYYY-MM-DD>
```

---
```

### Why H2 per pattern

- **Greppable**: `grep "^## Trigger:"` gives the agent a fast index without a tool.
- **Self-contained**: each section stands alone; no parser required.
- **Append-safe**: `edit_file` with an end-of-file anchor adds new sections without touching existing ones.
- **Human-curatable**: deleting a section is one paragraph removal.
- **No schema lock-in**: free-form body under each heading.

## 7. Acceptance criteria

Ship is complete when all of these are observable:

1. **Shell tool description mentions batching.** Loading the shell tool schema and inspecting `description` returns a string that references batching and the LEARNINGS.md file.
2. **Shard is loaded.** A fresh session's system prompt contains the `shell_batching.md` content verbatim.
3. **Runtime info contains LEARNINGS path.** The assembled system prompt includes a `LEARNINGS: <absolute path>` line next to `VAULT:`.
4. **LEARNINGS.md exists on startup.** After daemon start on a fresh vault, `<vault>/wards/LEARNINGS.md` exists with the seed header.
5. **Failed shell calls emit a hint.** A `shell` call that returns non-zero exit produces a `tool_result` whose JSON body includes a `hint` field pointing at LEARNINGS.md.
6. **Measurable batching improvement.** Running the same DB query as in Section 1 on sessions created after the change should show a measurable drop in consecutive-shell runs. Target: at least 50% reduction in runs of ≥3 consecutive shell calls. Rerun the query two weeks after landing.
7. **At least one agent-authored LEARNINGS.md entry** within the first week of production traffic, demonstrating the write-back loop works.

## 8. What we explicitly defer

1. **Growth control for LEARNINGS.md.** Manual curation for now. When the file grows past ~100 entries, the sleep-cycle compactor (demoted to memory-surface hygiene per `compaction-strategy.md`) will take over dedup and merging.
2. **Structured recall.** No embedding-based similarity search; agent reads the file directly. Revisit if the file grows large enough that full-read becomes expensive.
3. **Per-agent or per-ward scoping.** Global is sufficient for shell-failure patterns. Revisit if workload analysis shows ward-specific patterns that don't transfer.
4. **Metrics dashboard.** Add a counter for "consecutive shell calls per execution" and "commands per tool_use batch" once the prompt changes land. Not required for ship.
5. **Framework-driven automatic recall.** We chose agent-driven recall (Q5 in the brainstorm). If compliance proves low after rollout, reconsider framework-driven recall that appends matched patterns to the failure tool_result automatically.
6. **Confidence / mention_count on entries.** Not in the initial markdown format. Add when bad-pattern reinforcement becomes a measurable problem.
7. **Cross-agent memory distillation.** Session-close distillation into LEARNINGS.md (as opposed to agent-authored write-back) is out of scope. May be added later as part of the compaction plan's Layer 4.

## 9. Relationship to the compaction plan

This spec is the **first concrete scout of Layer 4** from `docs/architecture/future-state/compaction-strategy.md`. It intentionally makes conservative choices that generalize:

- **File surface** (markdown at `<vault>/wards/LEARNINGS.md`) matches Layer 4's "file-backed memory surface" prescription. Expanding Layer 4 later would add per-agent subdirectories (`<vault>/memories/<agent_id>/*.md`) using the same file-based pattern — LEARNINGS.md at the wards root sits alongside that.
- **Agent-driven read/write** via `read_file` / `edit_file` — no new tool schema. Layer 4's full design will add a thin `memory.recall` adapter over the file tree, but this spec does not require it.
- **Global scope** — consistent with Layer 4's "framework contracts in the stable prefix, episodic refinements in the memory surface" framing.
- **Compaction-safe**: LEARNINGS.md lives outside both the runtime compaction path and the sleep-cycle compactor's current scope. When Layer 4 lands fully, the sleep compactor's demoted role will include hygiene on this file.

If the compaction plan is adopted, this spec's decisions remain valid and compose forward. If this spec ships first (recommended — it is smaller and self-contained), the compaction plan inherits a working proof-point for Layer 4's file-surface pattern.

## 10. Open questions to watch after ship

Not blockers for ship, but worth monitoring once the feature is live:

- **Recall compliance rate** — do agents actually read LEARNINGS.md after a failure, or do they retry blindly? If the latter dominates, the agent-driven design has failed and we need to switch to framework-driven (deferred item 5).
- **Write-back quality** — are entries the agent writes actually useful, or are they overfit to one session's quirks? Humans will curate the first month and judge.
- **Section-marker discipline** — does the model consistently emit `echo "--- label ---"` before each sub-command, or does it drop markers under pressure? Affects how well the agent can parse failure output.
- **Batch-size ceiling** — how long do scripts get before the model starts truncating or producing malformed bash? Needs real-world measurement; adjust the shard guidance if agents write 50-line scripts that break.
