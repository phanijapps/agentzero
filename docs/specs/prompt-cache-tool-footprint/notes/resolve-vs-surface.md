# Resolve-vs-surface disposition

Opened: 2026-08-03

## Execution assumptions

- Touch `runtime/agent-runtime/src/llm/openai.rs` plus this spec's lifecycle documents and index entry.
- Focused request-construction tests, `cargo test -p agent-runtime`, formatting, and strict crate clippy demonstrate completion.
- Do not change cached-token parsing, provider cache directives, public APIs, persistence, or automatic tool pruning.

## Declined additions

- New module or abstraction layer: the existing private OpenAI-compatible request builder is the shared enforcement boundary.
- Runtime-configurable limits: changing limits requires an explicit product decision and is outside this phase.
- Automatic truncation or pruning: silently changing model authority is outside the approved contract.

## Dispositions

- Pre-execute adversarial findings: applied to the spec and plan; re-review clean.
- Pre-execute security findings: applied to the spec and plan; re-review clean.
- Adversarial implementation findings: applied in scope; request-body and all shared client/Rig paths covered; re-review clean.
- Security implementation finding: applied in scope; unknown provider-interpreted envelope fields rejected; re-review clean.
- Quality implementation finding: applied in scope; UTF-8 byte accounting and boundaries covered; re-review clean.

Closed: 2026-08-03. No findings deferred or surfaced beyond the two explicitly
authorized mechanical state/report repairs.

Finish-lint note: repository-wide spec-status lint still reports four pre-existing
violations in unrelated specs; those backlog artifacts were intentionally not changed.
