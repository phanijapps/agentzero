# Spec: Fast-Path Ward Note

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** none (single task, light mode)
- **Constrained by:** [ward-slim P1+P2](../ward-slim/spec.md) (same injection family)
- **Brief:** root-cause chase, session `sess-023329cf-66b4-5132-aadd-42cccd747cc1` (2026-08-20)
- **Contract:** none
- **Shape:** fix

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Mode

Light (no risk trigger fired: single renderer branch, no structure/security/UI change).

## Objective

On the `simple` (fast-path) approach, `format_intent_injection` discards the
classifier's `ward_recommendation` entirely and injects an unconditional
"Do NOT call `ward`" prohibition — so a simple task whose domain matches an
existing ward is instructed away from storing its work there (incident:
`sess-023329cf`, AAPL valuation, `financial-analysis` identified and dropped).
Render the recommendation as soft guidance instead of discarding it.

## Acceptance criteria

- [x] AC1 — `approach=simple` + `ward_recommendation.action=use_existing`:
  the injection contains a ward note naming the exact ward that says, in
  substance: if you will create files or write memories, first call
  `ward(action="use", name="<ward>")` so the work is stored there; pure
  read-only answers may skip it. Subdirectory, when present, is named.
- [x] AC2 — `approach=simple` without a `use_existing` recommendation
  (incl. `create_new`, and the trivial-message placeholder whose
  `use_existing "general"` is fabricated — suppressed via the shared
  `TRIVIAL_WARD_REASON` sentinel; the semantic-failure fallback overwrites
  that reason with real matches and still renders the note): injection is
  unchanged from today — no ward note, prohibition stands (simple tasks
  must not scaffold new wards).
- [x] AC3 — `approach=graph` injections are byte-identical to today (warm
  path, required-workspace path, planner context all untouched).
- [x] AC4 — renderer unit tests pin AC1-AC3 (red-first for AC1).

## Boundaries

- No change to: planning gate (still graph-only), ward tool, prompt shards,
  ward-as-agent delegation, `run_procedure` prohibition.
- Live gate: user re-runs the AAPL prompt on the rebuilt daemon and confirms
  the ward note appears and a ward entry happens when file/memory work occurs.

## Testing strategy

TDD in `intent_analysis.rs` tests: build `IntentAnalysis` via serde (existing
idiom), call `format_intent_injection`, assert on rendered text. Gates:
`cargo fmt --check`, `cargo clippy -p gateway-execution`,
`cargo test -p gateway-execution`, `cargo check --workspace`.
