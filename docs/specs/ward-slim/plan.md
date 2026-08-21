# Plan: Ward Slim — Payload & Redirect Dedup (P1+P2)

- **Status:** Done

Implements [`spec.md`](spec.md). Branch `refactor/ward-slim-p1p2`.

## Assumption trio

- **Touch:** `runtime/agent-tools/src/tools/ward.rs` (result assembly,
  `read_agents_md`, `recall_ward_facts` limit, tests),
  `runtime/agent-tools/src/tools/guards.rs` (redirect helper),
  `runtime/agent-runtime/src/rig_adapter/tool.rs` + `runtime/agent-runtime/src/executor.rs`
  (call the helper), spec/plan, `workspace.toml` + `docs/backlog.md` deferrals.
- **Done when:** all spec ACs hold; `cargo test -p agent-tools -p agent-runtime`
  green; workspace check green.
- **Not changing:** gate state machine, delegation actions, template
  fail-closed, guard error strings, prompt shards, tool registration.

## Declined temptations

- Trimming fact content length — only the count changes (5→3); content caps
  belong to the fact store, not the ward tool.
- Setting `ward_template` state from the tool (it "should" match the new ward)
  — would break the immutable-bootstrap contract that template actions stay
  stale until the next executor build; the packet reaches the model via the
  next system instruction.
- Collapsing the two enforcement sites into one — they wrap genuinely
  different dispatch paths (MCP-wrapped vs builtin); only the duplicated
  message dies.
- `ward.rs` module split — cosmetic, no behavior/token effect.

## Tasks

### T1 — Red tests (TDD)

Depends on: none

Update in `ward.rs` tests (red against current code):
- `result["planner_started"] == true/false` → `result["planner"] == "started"/"pending-template"` (sites ≈1855-1933).
- `created["ward_template"]["archetype"] == "coding"` → `created["ward_status"]["archetype"] == "coding"`.
- New `ward_entry_result_is_slim`: gated cold-graph entry with template,
  store-less → assert payload keys exactly the AC1 set; assert no
  `projection`/`digest` substring anywhere. (The with-knowledge clause of
  AC6 is pinned separately — see `ward_entry_recall_is_trimmed_and_present`.)
- New `ward_entry_recall_is_trimmed_and_present`: `RecordingFactStore` mock
  (records requested limit) → ward_knowledge present, ≤3 results, full key
  set = AC1 + `ward_knowledge`.
- New `agents_md_is_bounded`: ward with >32 KiB AGENTS.md → returned
  `agents_md` ≤ cap + truncation marker.
- Redirect helper tests: one in `guards.rs` tests (shape + canonical text);
  rig_adapter dispatch test tightened to the canonical suffix. Executor site
  is a pass-through with no local literal (AC4 grep is the drift guard).

`Done when:` red run shows failures only in the intended assertions.

### T2 — Implement (green)

Depends on: T1

- Result assembly (ward.rs use/create): build `ward_status` from
  `layout_state.packet` (`status`, `archetype` — null when undeclared);
  replace `planner_started`/`planner_error` with `planner` string enum;
  delete `recall_nudge`; `agents_md` read bounded at 32 KiB with a
  cap-derived truncation marker (invalid-UTF-8-within-cap → None, previous
  semantics); recall limit 5→3 (store envelope passes through; `count` =
  returned count).
- `guards.rs`: `pub fn cold_graph_redirect() -> serde_json::Value` (one
  canonical envelope, no separate const).
- `rig_adapter/tool.rs` + `executor.rs`: call the helper; delete local
  message literals.

`Done when:` all T1 tests green; clippy clean on both crates.

### T3 — Gates + docs

Depends on: T2

- `cargo fmt --check` + clippy + tests (agent-tools, agent-runtime) +
  `cargo check --workspace`.
- `workspace.toml`/`docs/backlog.md`: add `ward-slim-p3-prompt-dedup`,
  `ward-slim-p4-audience-split`, `ward-slim-p5-gate-vocabulary`.

`Done when:` all gates exit 0 in one pass.

### T4 — Review (full mode)

Depends on: T1-T3

- `adversarial-reviewer` on spec + diff (no security trigger: no new I/O,
  auth, or input surface — payload content and message dedup only).

`Done when:` reviewer Clean; spec `Status: Shipped`; PR to `develop`.
