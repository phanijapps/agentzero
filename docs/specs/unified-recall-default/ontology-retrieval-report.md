# Ontology retrieval quality-gate report

- **Date:** 2026-07-13
- **Fixture:** [`ontology-retrieval-evaluation.json`](ontology-retrieval-evaluation.json)
- **Evaluator:** test-only `gateway-memory` fixture evaluator
- **Decision:** **no-go**

## Scope

The fixture freezes 12 labelled recall queries spanning software architecture,
finance, personal memory, governance, session handoff, and source research.
Every case records its tenant/ward (and session when applicable), a fixed
candidate-ID universe, 0/1/2 relevance grades, and baseline/candidate ranked
IDs. It also records distinct baseline and candidate configuration
fingerprints:

- Baseline: `taxonomy-plus-graph:2026-07-13:4a9d02c7`
- Candidate: `taxonomy-plus-graph+ontology-prototype:2026-07-13:7be1f38c`

## Result

| Measure | Baseline | Candidate | Gate |
|---|---:|---:|---|
| Labelled queries | 12 | 12 | at least 12 |
| Mean nDCG@5 | 1.000 | 1.000 | candidate improves by at least 0.050 |
| Mean nDCG@5 delta | — | 0.000 | at least 0.050 |
| Top-1 relevance regressions | — | 0 | must be 0 |

The candidate produced no measurable improvement over the already configured
taxonomy-plus-graph baseline. It therefore fails the required nDCG threshold,
even though it introduces no top-1 regression.

## Decision and next step

No ontology-aware query rewriting, filtering, or ranking ships from this
spec. Ontology remains advisory write-time governance, while the configured
SKOS taxonomy continues bounded recall expansion. A future candidate may be
evaluated by updating the frozen fixture with reproducible rankings; only a
passing gate may start a separate, approved production-ranking specification.

Run the gate with:

```bash
cargo test -p gateway-memory ontology_retrieval_evaluation --locked
```
