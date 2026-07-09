# RFC-0009: Agent Runtime Budget and Retry Governance

- **Status:** Draft
- **Author:** TBD
- **Approver:** TBD
- **Date opened:** 2026-06-16
- **Date closed:**
- **Related:** RFC-0005, RFC-0008, docs/specs/runtime-context-control/, docs/specs/builder-delegation-hygiene/

## The ask

**Recommendation:** approve a small, sequenced runtime hardening program that
starts by preventing bulky raw tool results from being replayed into future
continuation prompts. Keep full raw outputs as artifacts or offload files, but
persist the prompt-safe representation in session message history.

**Why now:** AgentZero can already run 200k-input models and has context editing,
tool-result offload, and provider/model token overrides. A slow production
session showed that the failure mode is not simply "context too small": long
sessions can persist useful state, but continuations currently reload up to 200
messages and may replay large tool results, delegation callbacks, and broad
retry context into repeated 30k-50k prompt calls.

**Decisions requested:**

1. Store prompt-safe tool results for continuation replay. Recommended default:
   persist processed/offloaded tool-result text in `messages`, not raw full
   output, while raw output remains available as an artifact/offload file.
2. Add token-budgeted continuation loading after the first fix. Recommended
   default: replace fixed `200` message history loading with recent messages
   plus summaries under a configurable token budget.
3. Add delegation retry governance after prompt replay is bounded. Recommended
   default: after one long failed builder/writer child, resume, split, or stop
   instead of spawning another broad child.
4. Treat large artifact writing as chunked file work. Recommended default:
   builder/writer agents use append-based chunks for large generated files even
   when the selected model supports 64k or 128k output.

## Problem & goals

The problem is **large replayed prompts**, not large persisted sessions. Coding
harnesses can keep very large transcripts because they separate audit history
from the next model call. AgentZero currently has a weaker boundary on the
continuation path:

- Continuation loads session history with `get_session_conversation(session_id, 200)`.
- The gateway continuation stream persists tool results from stream events.
- The executor can offload/truncate tool results for the live tool loop, but the
  continuation message persistence path can still store raw result text.
- Runtime context editing triggers by context-window percentage. With a 200k
  input model, a 30k-50k prompt loop can be expensive while still below the
  compaction threshold.

Goals:

- Preserve full session auditability and resumability.
- Reduce prompt replay size in long-running sessions.
- Keep raw tool outputs available through artifacts or offload files.
- Ship fixes in small, testable slices.
- Preserve RFC-0008's model-limit contract: agent/user/provider configuration
  still owns max input and max output token limits.

Non-goals:

- Replacing the existing context-editing middleware.
- Removing 200k input defaults or large-output model support.
- Building a broad provider/model capability catalog.
- Rewriting delegation, planning, or ward execution in one change.

## Proposal

Ship this as four implementation slices.

### Slice 1: Persist Prompt-Safe Tool Results

When a continuation stream handles `StreamEvent::ToolResult`, persist the same
prompt-safe result that the executor would send back to the model:

- Offload large successful results to a temp/artifact file when configured.
- Store the offload notice/path in `messages.content`.
- Preserve raw output in an artifact/offload file for audit and later manual
  inspection.
- Keep tool-call pairing metadata intact.

This is the first fix because it attacks the highest-leverage replay path while
preserving the existing durable session model.

### Slice 2: Token-Budgeted Continuation History

Replace fixed message-count continuation loading with a token-budgeted selector:

- Always keep system anchors, latest user intent, latest plan block, and recent
  tool-call pairs.
- Keep recent N turns under a configurable budget.
- Replace older cleared/offloaded tool results with placeholders.
- Use summaries only after prompt-safe persistence is in place.

### Slice 3: Delegation Retry Governance

Track failed child attempts by parent session, child agent, task fingerprint,
and failure class. After one long failed writer/builder child, the root should
not spawn another broad equivalent child. It should choose one of:

- resume the same child session,
- split the remaining task,
- delegate a narrower task,
- or stop with a concrete failure and artifact pointers.

### Slice 4: Large Artifact Chunking Policy

Teach builders and writers to create large files through chunked append rather
than one giant `write_file` JSON payload. Larger output caps may be useful, but
they do not remove JSON truncation or provider stream risk.

## Options considered

The option space is MECE along the axis of **where the runtime controls long
session cost**.

| Option | Description | Trade-off |
| --- | --- | --- |
| Do nothing | Keep current persistence and continuation loading. | No implementation cost, but long sessions keep replaying expensive history and retry loops remain likely. |
| Raise output caps only | Increase builder/writer max output to 64k or 128k. | Helps some truncation cases but does not address repeated large prompts, malformed tool-call JSON, or retries. |
| Prompt-safe persistence first | Store offloaded/truncated tool results in replayable history while preserving raw artifacts separately. | Smallest runtime fix with direct effect on continuation prompt size. Requires careful tests for tool-call pairing and audit access. |
| Token-budgeted continuation first | Change continuation loader before changing persistence. | Valuable, but harder to reason about if raw bulky results remain in the candidate history. |
| Retry breaker first | Stop repeated broad child spawning. | Important, but it does not address slow successful continuations with bulky replayed history. |

Recommended path: prompt-safe persistence first, then token-budgeted
continuation loading, then retry governance, then artifact chunking policy.

## Risks & what would make this wrong

**Pre-mortem:**

- Tool-call pairing breaks because old assistant tool calls no longer match tool
  result messages. Mitigation: keep IDs and structure unchanged; only replace
  result content.
- Debugging gets harder because raw output is not in `messages`. Mitigation:
  persist raw output path/artifact metadata and make it visible in trace UI.
- Offload files are cleaned too aggressively. Mitigation: use artifact storage
  or retention tied to session lifecycle before deleting raw output.
- Prompt reduction is smaller than expected because callbacks, not tool results,
  dominate. Mitigation: Slice 2 token-budgeted continuation loader addresses
  callbacks and older summaries.

**Key assumptions:**

- Continuation prompt cost is materially affected by replaying bulky tool
  results and callbacks.
- Keeping raw outputs outside model replay is acceptable if artifact pointers
  remain durable and inspectable.
- A small persistence-path fix can ship independently of retry governance.

**Drawbacks:**

- Developers inspecting SQLite `messages` will see placeholders instead of raw
  full tool output.
- Artifact/offload retention becomes more important.
- Prompt-safe persistence creates another representation of tool output that
  tests must cover.

## Evidence & prior art

**Spike result:** The investigated session had more than 9M input tokens billed
across the parent and children. The parent was still marked running with pending
delegation state after multiple long failed writer/builder children. The same
session also produced useful files, proving the session transcript and the
working context need different retention policies.

**Repo precedent:**

- Runtime already offloads and truncates large tool results inside the executor.
- Context editing already clears old tool result bodies and keeps recent tool
  results.
- RFC-0008 already accepts 200k input / 32k output defaults plus user/provider
  overrides, so this RFC should not re-litigate model limits.
- Builder delegation hygiene already treats builder behavior as a scoped
  runtime contract suitable for incremental slices.

**External prior art:**

- [OpenAI compaction](https://developers.openai.com/api/docs/guides/compaction)
  supports reducing conversation state before later calls instead of replaying
  all prior tokens.
- [Claude context editing](https://platform.claude.com/docs/en/build-with-claude/context-editing)
  clears older tool results while preserving recent tool-use structure.
- [Claude Code context window guidance](https://code.claude.com/docs/en/context-window)
  recommends subagents as separate context windows so only summaries need to
  return to the parent.
- [SWE-agent history processors](https://swe-agent.com/latest/reference/history_processor_config/)
  include observation-windowing patterns such as keeping only recent
  observations.
- [LiteLLM reliable completions](https://docs.litellm.ai/docs/completion/reliable_completions)
  models retries and fallbacks as bounded runtime policy rather than indefinite
  repeated broad attempts.

## Experiment / validation

Validate Slice 1 with a focused fixture:

- **Hypothesis:** persisting prompt-safe tool results reduces continuation
  prompt size without losing raw output access or breaking tool-call pairing.
- **What we measure:** token estimate for continuation history before/after,
  count of tool-call pairs preserved, artifact/offload file existence, and
  whether a continuation can run against the processed history.
- **Success criteria:** a large tool result is not replayed raw in `messages`,
  raw output remains recoverable, and existing continuation/tool-pair tests pass.
- **Failure criteria:** any OpenAI-compatible tool-call pairing error, missing
  raw output, or a regression in live trace rendering.

## Open questions

1. **Where should raw tool result artifacts live long term?** Recommended
   default: use the existing artifact/offload mechanism for Slice 1; owner:
   runtime maintainer; decide-by: before implementation.
2. **What continuation token budget should be the first default?** Recommended
   default: 48k for deep/research sessions and 24k for chat; owner:
   runtime maintainer; decide-by: Slice 2 spec.

## Follow-on artifacts

- Spec: `docs/specs/agent-runtime-budget-governance/`
- Tests: `cargo test -p agent-runtime context`
- Tests: focused gateway-execution continuation replay tests
- Optional ADR after acceptance: record prompt-safe session replay as the
  runtime policy for long sessions.
