# Spec: Multimodal Analyze URL Fetch & Honest File Contract

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Discovery:** defect investigation, session `sess-e7d37779-6921-5c3d-80c8-52a1742a4969` (2026-08-20)
- **Contract:** none
- **Shape:** integration

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Mode

Full (risk trigger: new outbound network I/O — the tool gains a URL fetch on
agent-supplied URLs). Loop-engine/cohort state machine scripts are absent from
this repository; the loop spine (plan → execute → verify → review) runs
manually with `adversarial-reviewer` and `security-reviewer` passes.

## Objective

`multimodal_analyze` must send only content-part dialects the configured
OpenAI-compatible provider can parse, and must never advertise capabilities it
cannot deliver. Remote `http(s)` image sources are fetched by the tool and
inlined as base64 `image_url` data URIs; `type: "file"` inputs fail fast with
an agent-actionable error instead of a raw provider 400.

### Evidence (root cause, reproduced live)

- Incident: root agent passed a webpage URL as `{"type":"file"}`; the tool
  emitted `{"type":"file","file":{"url":…}}` to Ollama
  (`http://localhost:11434/v1`, `gemma4:31b-cloud`); Ollama recognizes only
  `text` and `image_url` parts → 400 `invalid message format` (reproduced
  byte-identical via curl, including with a valid PDF URL).
- Sibling: 2026-08-04 incident — remote image URL → Ollama 400
  `"image URLs are not currently supported, please use base64 encoded data
  instead"`. Ollama never fetches remote URLs for any part type.
- The `{"type":"file","file":{…}}` shape matches no OpenAI-compatible dialect
  (OpenAI chat/completions has no file part; GLM/Kimi use `file_url`), so the
  `file` path is nonfunctional against every provider, not just Ollama.
- `multimodal.rs` had zero tests; the tool description ("images, PDFs, or
  documents") and schema ("File path, URL, or base64 data") invite the
  failing calls.

## Acceptance criteria

- [x] **AC1 — image URL inline.** Given `type: "image"` with an `http(s)://`
  source, the tool fetches the bytes itself (30 s timeout, 20 MiB cap,
  http/https only), takes the MIME type from the response `Content-Type`
  header (extension inference as fallback), and emits
  `{"type":"image_url","image_url":{"url":"data:<mime>;base64,…","detail":…}}`.
  No raw remote URL is ever placed in the request body. The fetch applies
  only to `http(s)` URLs (anything else in a URL source errors); non-base64
  `data:` URIs are rejected with an explicit base64-requirement error; and
  the fetched content must be an image — a `Content-Type` that is missing or
  non-image falls back to URL-extension inference, and if neither says
  image the call rejects with "URL did not return an image" (text bodies
  must not enter the one-shot context through an image part).
- [x] **AC1a — credential-free fetch.** The URL fetch uses its own reqwest client
  and sends no `Authorization`, `Cookie`, or other credential headers. Only
  the provider POST (unchanged) attaches the provider API key. A test
  asserts the fake image server captured no `Authorization` header.
- [x] **AC1b — instruction/data isolation.** The one-shot request carries a fixed
  directive stating that instructions embedded in the attached content are
  data to report, never instructions to follow. Asserted present in the
  captured request body.
- [x] **AC2 — file fast-fail.** Given `type: "file"` (any source), the tool
  returns an `AgentError::Tool` **before any API call**, whose text names the
  limitation and tells the agent what to do instead (fetch/extract text via
  shell for text/HTML; PDFs unsupported until provider file dialects exist).
- [x] **AC3 — honest contract.** The tool `description()` and the JSON schema for
  `content` no longer advertise documents/PDFs or bare "base64 data"; they
  describe images with file path / URL / `data:` URI sources. `permissions()`
  declares `moderate(vec!["network:http"])` (the tool performs agent-directed
  outbound HTTP; `safe()` understates it — policy vocabulary at
  `agent-primitives/src/policy.rs`). The declaration is metadata only;
  nothing enforces it at runtime today. The in-repo `eagle-eye` skill
  template no longer teaches a PDF/file path.
- [x] **AC4 — regression tests.** New tests in `multimodal.rs` cover: file-type
  fast-fail (AC2); URL image fetched and inlined with a data URI in the
  captured request body (AC1, via local fake image server + fake
  OpenAI-compat endpoint that captures the request); oversize fetch rejection;
  non-image `Content-Type` rejection (AC1); non-base64 `data:` URI rejection
  (AC1); and both AC5 clauses. Tests are red against the pre-fix code for
  AC1/AC2/size-cap/mime-gate/data:-rejection.
- [x] **AC5 — no dialect regressions.** Local-path and `data:`-URI image sources
  keep producing the same request shape as before (both halves covered by
  tests).

## Boundaries

### Always do

- Bound the fetch: 30 s timeout, 20 MiB response cap, reject non-http(s) schemes implicitly (only `http(s)://` and `data:` prefixes resolve as remote/inline sources).
- Follow redirects (reqwest default cap); the timeout and size bounds apply across the whole redirect chain (reqwest refuses redirects to non-http(s) schemes; https→http downgrades are followed, which crosses no boundary here since http is allowed). Pinned by a redirect test.
- Accept the per-item 20 MiB cap as the size limit of record: aggregate request size across `content` items is unbounded but pre-exists via multi-item `data:` URIs, and the provider rejects oversized bodies cleanly. No aggregate cap is added.
- Match existing crate style: `AgentError::Tool` strings, `json!` blocks, no new dependencies.

### Ask first

- Supporting per-provider file dialects (OpenAI `input_file`, GLM `file_url`) — restore an encoder layer.
- Inlining text/HTML file content as text blocks (making `file` "work" for text documents).
- Any change to main-path `ChatMessage`/`Part` wire serialization (latent, separate).

### Never do

- Don't touch the main LLM client path (`agent-runtime/src/llm/openai.rs`), the planner, or the shell guard in this change.
- Don't add a URL-allowlist/private-IP blocklist without a security-review finding requiring it (agents already have unrestricted shell `curl`; note in review brief). Revisit this decision if the daemon ever runs hosted/off-desktop or alongside an authenticated internal network.

## Testing strategy

TDD in-crate (`runtime/agent-tools/src/tools/multimodal.rs` `mod tests`):
fake HTTP servers built on `std::net::TcpListener` + a writer thread (no new
dev-dependencies) serve (a) the image bytes and (b) an OpenAI-compat endpoint
that captures the request body and returns a canned completion. Mock
`ToolContext` supplies `multimodal_config` state (pattern from
`connectors.rs::MockToolContext`). Gates: `cargo fmt --check`,
`cargo clippy -p agent-tools`, `cargo test -p agent-tools`,
`cargo check --workspace`. Live evidence: data-URI request shape was already
validated against the real Ollama endpoint (200 OK) during investigation.

## Deferred

- `(deferred: multimodal-provider-file-dialects)` → AC2 guidance text points here; backlog slug `multimodal-provider-file-dialects`.
- Main-path multimodal wire dialect (`{"type":"image",…}` homegrown serialization in `ChatMessage`) — backlog slug `main-path-multimodal-dialect`.
