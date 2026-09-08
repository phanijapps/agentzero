# Spec: Execution consolidation waves 2–4 (carry-forward state)

- **Status:** Drafting (sequenced specs land per wave)
- **Branch:** `op_clean_crap`

## Completed on this branch

| Commit | Wave | Receipt |
|---|---|---|
| `2ba0ca71` | 0 — EngineHook + HookSet | 4 hook aliases deleted; extensible framework |
| `f8e4999e` | 1 — ExecCtx | ContinuationArgs×30, both invokers, 59 field duplicates deleted; runner = {ctx, bootstrap} |
| `278001b8` | 1b — comment scrub | zero old-name references |

All gates green at each commit. Full deck: `docs/execution-consolidation-deck.html`.

## Wave 2 — crate merge (NOT STARTED; riskiest, do fresh)

Move gateway-execution's modules into agent-runtime, rename crate → `zbot-execution`, delete gateway-execution. Steps:
1. `git mv gateway/gateway-execution/src/* runtime/agent-runtime/src/gw/` (subtree keeps module paths stable: `zbot_execution::gw::runner` etc. — flatten later).
2. agent-runtime Cargo.toml absorbs gateway-execution's 21 dep edges.
3. Migrate `gateway_execution::` imports (14 paths in gateway/src + apps — census in the deck slide 4).
4. Collapse `ChatMessage` triple-alias imports (24 sites: `llm::ChatMessage`, `types::ChatMessage`, root).
5. Kill `engine::snapshot::CHECKPOINT_KEY` leak (13 external uses → recovery API).
6. Delete gateway-execution crate; update workspace members.
Gate: workspace builds, all suites, e2e, `cargo deny`.

## Wave 3 — ToolSpec const table (IN PROGRESS; see spec.md beside this file)

## Wave 4 — turn.rs loop (NOT STARTED)

Consume HookSet inside one top-to-bottom loop; `enum TurnSignal {Continue, Stop, Steer, Delegate(Yield|FireAndForget), Respond}` replaces the 27 interleaved flags; rig_adapter/engine.rs (1,653) dissolves into turn.rs + model/tool clients; stream_event_processor collapses to enum-driven conversion. Fire-and-forget empty-callback bug (Uber session diagnosis) dies here — `Delegate(FireAndForget)` is an explicit signal. Golden traces required before starting: record simple-qa + stop-and-continue + delegation-flow event streams.

## Standing rules (all waves)

Boy scout: no compat aliases, no shims, old names deleted. Each wave fixes upstream callers and downstream callees in the same stroke. Receipt test: net lines must go down. Gates: agent-runtime + gateway-execution suites, clippy -D warnings, fmt, workspace tests (tracked pre-existing exceptions only).
