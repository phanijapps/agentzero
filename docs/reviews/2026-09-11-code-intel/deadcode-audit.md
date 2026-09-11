# Dead-Code Audit — engram codegraph verdicts vs rg ground truth

Date: 2026-09-10 · Branch: develop · Graph: 16,988 nodes / 41,775 edges, **15,109 "dead" verdicts**

## Verdict on the graph's health report

The 15,109 figure is **not actionable** as a deletion count. The audit of its
top candidates shows the graph's dead verdict systematically misclassifies:

1. **Same-file production usage** (private helpers used once in their module — the most common Rust pattern)
2. **Feature-gated-but-wired crates** (gateway-a2a: runtime-off, compile-wired)
3. **Test-container liveness** (helpers defined+used inside `#[cfg(test)]` — container-live per engram's own code-liveness doctrine)
4. **Stale symbols** (ActiveMemorySourceAdapter — deleted in P3; the graph still carries it)

The graph's *architecture* output (centrality, bridges, communities) is sound;
its *liveness* output needs the contains-live-container fix from engram
(`~/projects/mem-alpha` commits 5fe733e/9632c63 do this for engram's own index —
zbot's index build should pick it up on the next full re-scan).

## Candidate-by-candidate (rg ground truth, workspace-wide)

| Symbol | Graph | Audit verdict | Action |
|---|---|---|---|
| `A2aClientError`, `A2aHttpState`, `A2aInbound/Outbound*Handler`, `A2aTaskService::*` | dead | **Feature surface** — workspace member (`Cargo.toml:45`), consumed by `gateway/src/tasks/a2a.rs` (production path, runtime-gated behind `--a2a`, off in daemon) + `gateway/tests/a2a_*.rs` | Keep. Crate retirement is a product decision (a2a federation awaits adoption), not a dead-code call |
| `A2uiCapabilities` | dead | **PROVEN-UNUSED** — defined `runtime/agent-surfaces/src/lib.rs:24`, zero references outside its own file | **DELETED** (commit `12e3d267`) |
| `AbortOnDropTask` (+`abort_and_wait`) | dead | **Container-live** — private type, production use at `gateway-bus/src/worker.rs:893` (panic-redaction wrapper) | Keep |
| `ActiveHandlerGuard` | dead | **Test-container-live** — defined+used inside `gateway-bus/tests/durable_work_worker.rs:409,470` | Keep |
| `ActiveMemorySourceAdapter::active_memory_texts` | dead | **Does not exist** — removed during P3; graph is stale on this symbol | Nothing to do |
| `AdapterEmbeddingProviderConfig` | dead | **Container-live** — production config type, used across `stores/zbot-engram-adapter/src/{config,lib,migration}.rs` + bootstrap tests | Keep |
| `BoundedJsonCounter::write` | dead | **Container-live** — same-file production use `execution-state/src/types.rs:872` (session-plan payload bound) | Keep |
| `FixtureReport::push` | dead | **Container-live** — adapter capability fixtures reporting (`stores/zbot-engram-adapter/src/{lib,capabilities,fixtures}`) | Keep |
| `GovernanceSelection::is_empty` | dead | **Graph false positive** — production use at `stores/zbot-engram-adapter/src/capabilities.rs:160` (`!selection.is_empty()`) | Keep |
| `AgentTaskMode::as_str` | dead | **Container-live** — `gateway/src/tasks/durable_agent.rs:534` (Debug impl) | Keep |
| `KnowledgeGraphSidecar::lock` | dead | **Container-live** — used 4× in its own module (`stores/.../knowledge_graph.rs:1208,1271,1324,1335`) | Keep |
| `SummaryOptions` (+ `daily-sessions` crate) | dead | **Dead crate candidate** — `generate_summary`/`generate_summary_with_options` have ZERO callers; sole referenced consumer `gateway/src/http/conversations.rs` carries `// TODO: Connect to daily_sessions in Phase 3b` (4×) | **Report only** — scaffolded-ahead crate; retirement needs explicit instruction |

## Scorecard

- Audited: 12 symbol families (the graph's top list) + 4 spot-checks
- Proven dead: **1** (A2uiCapabilities) — deleted
- Container-live / false positive: 10
- Feature surface (correct to keep): the gateway-a2a family
- Dead-crate candidate flagged: `services/daily-sessions` (scaffold + TODO wiring; ~small crate)

## Recommendation to the graph

The engram repo's own code-liveness doctrine (contains-live containers count
as live, dead-code verdicts must be trustworthy — mem-alpha `5fe733e`) is the
right fix: re-scan zbot with an engram build that includes it, and treat
graph-dead only as a *candidate queue* for rg audit, never as a verdict.
That's exactly how this audit was run.
