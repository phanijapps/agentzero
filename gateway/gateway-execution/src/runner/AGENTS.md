# runner

Session orchestration with focused control, bootstrap, streaming and dispatch
components. **Read this before adding code here.**

## Build & Test

```bash
cargo test -p gateway-execution --features test-stubs
cargo clippy -p gateway-execution --all-targets --features test-stubs -- -D warnings
```

## Module map

| File                       | Owns                                          |
|----------------------------|-----------------------------------------------|
| `core.rs`                  | `ExecutionRunner` facade, DI wiring, invocation and persisted recovery |
| `session_control.rs`       | Live stop/pause/resume/cancel/end/iteration control; shared handles and delegation registry |
| `session_invoker.rs`       | Narrow traits handlers depend on instead      |
|                            | of `Arc<ExecutionRunner>`                     |
| `invoke_bootstrap.rs`      | Pre-execution setup (per session, two-phase)  |
| `execution_stream.rs`      | Per-execution event loop                      |
| `delegation_dispatcher.rs` | Long-lived queue for spawning subagents       |
| `continuation_watcher.rs`  | Long-lived listener for continuations         |

## The rule

Every handler is a struct that declares — in its field list —
exactly the services it uses. Adding a new dependency means adding
a field; the field list IS the documentation of what this code
touches.

If you find yourself wanting `Arc<ExecutionRunner>` in a new
handler, **stop**. Use a narrow trait (or define a new one). The
whole point of this layout is to never hand a single handler the
god-struct again.

## Setter mirroring invariant

`SessionControl` owns the runner's handle map, delegation registry and state
service references. Bootstrap, streams and recovery receive clones of these
same handles; never create a replacement registry during extraction. Public
control methods delegate to it, while `resume` retains persisted-subagent
recovery orchestration and delegates only its live-handle fallback. Legacy
broad controls and exact conversation-tree cancellation are distinct semantics.

Late-wired setters on `ExecutionRunner` (e.g.
`set_graph_storage`, `set_ingestion_adapter`, `set_goal_adapter`)
must update BOTH `self.<field>` AND `self.bootstrap.<field>` —
`InvokeBootstrap` reads its own clones at session-setup time. The
setter implementations are explicit about this; preserve the
pattern when adding new late-wired services. (Fields backed by
`ArcSwapOption` like `model_registry` do not need explicit
mirroring — bootstrap and runner share the same `ArcSwap` interior.)

## How to add a new handler

1. Define struct with explicit fields (only what you use).
2. Add `pub fn spawn(self) -> JoinHandle<()>` (long-lived loop) or
   `pub async fn run(&self, ctx, …) -> Result<…>` (per-execution).
3. In `core.rs`, wire it in the constructor: clone the right
   `Arc`s, pass them in, store the `JoinHandle` if the caller
   needs to await it.
4. Co-located tests in the same file using `#[cfg(test)] mod tests`
   with the `TempDir + real-SQLite` pattern, or in
   `tests/<handler>_tests.rs` if the test needs the `test-stubs`
   feature flag.

## Decomposition history

- 2026-04-26: Extracted from runner.rs (3,067 → ~1,663 residual LOC).
  Natural floor for core.rs is ~1,663 LOC because `invoke_continuation`
  (~460 LOC) and the model registry regression tests (~100 LOC) remain
  here by design; they depend on `ExecutionRunner` internals or are
  intra-module helpers used by `execution_stream.rs` and
  `invoke_bootstrap.rs`. No dead code found after the sweep (clippy -D
  warnings passes cleanly).
  See `docs/superpowers/specs/2026-04-26-runner-decomposition-design.md`
  and the implementation PR.
