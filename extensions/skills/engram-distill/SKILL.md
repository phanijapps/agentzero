---
name: engram-distill
description: Distill knowledge from any source — docs, code reviews, conversations, session transcripts — into the engram knowledge graph. Extract entities, relationships, and facts; classify them against the project's multi-layer ontology (technical/domain/business); and persist them so future recall surfaces them. Use after any learning event worth remembering. Extraction is your reasoning; the server never calls an LLM.
---

# engram-distill

Turn anything you just learned into a persistent, queryable knowledge graph by
writing it through the **engram** MCP server.

**`engram-ingest`** vs **`engram-distill`**: `engram-ingest` is for **raw docs**
(chunk + index the text). `engram-distill` is for **extracted knowledge** — the
structured entities/relationships/facts you distill from any source (a doc you
read, a code review, a design conversation, a session transcript). Use both
together: `engram-ingest` for the raw text, `engram-distill` for the extracted
graph.

## When to run

- You just read a doc / RFC / README / design note and learned something.
- You reviewed code and discovered an important relationship or pattern.
- A conversation or session revealed domain knowledge worth remembering.
- You want to build a project's multi-layer KG (technical + domain + business).

## Workflow

### 1. Read the active ontology + taxonomy

```
ontology_read     → learn the layers (technical/domain/business), their classes,
                    and the allowed within/across predicates.
taxonomy_read     → learn the concept hierarchy for classification.
```

**Every entity you extract must map to a class in one of the ontology layers.**
**Every relationship must use a configured predicate.** Do not invent.

### 2. Index raw docs (if you have the source text)

```
index_docs({ "content": "<markdown>", "path": "docs/design.md" })
```

This chunks the doc into retrievable sections (docs lane) — verbatim text that
`recall` can surface alongside the extracted KG.

### 3. Extract (your reasoning)

From the material, produce three kinds of structured knowledge:

**Facts** — free-text observations worth remembering:
```json
{ "content": "The SessionDistiller extracts facts from transcripts after each session." }
```

**Entities** — named things, classified across layers:
```json
{ "name": "SessionDistiller", "kind": "function" }       // technical
{ "name": "Distillation", "kind": "concept" }             // domain
{ "name": "Onboarding Flow", "kind": "concept" }          // business
```

Valid kinds: `function`, `module`, `struct`, `trait`, `repository`, `concept`,
`api`, `organization`, `project`, … Unknown kinds are **rejected**.

**Relationships** — using only configured predicates:

| Layer | Predicates |
|---|---|
| within (same layer) | `depends_on`, `implements`, `contains`, `part_of`, `uses` |
| across (cross-layer) | `realized_by`, `describes`, `governs`, `distilled_from`, `extracted_from`, `contradicts` |

**Cross-layer edges are the point** — they bridge layers:
- `SessionDistiller` —`realized_by`→ `Distillation` (technical → domain)
- `Onboarding Flow` —`governs`→ `SessionDistiller` (business → technical)
- `docs/design.md` —`describes`→ `SessionDistiller` (doc → code)

### 4. Write the batch

```
store_knowledge({
  "idempotency_key": "distill-<source>-v1",
  "facts": [...],
  "entities": [...],
  "relationships": [...]
})
```

- **Stable `idempotency_key`** — re-sending the same batch dedups, not doubles.
- **Best-effort, not ACID** — the result surfaces per-step status.
- Malformed entries are skipped and reported.

### 5. Verify

```
recall({ "query": "distillation" })           → fused retrieval
search({ "query": "SessionDistiller" })        → entity by name
get_context({ "focus": "Distillation" })        → context packet
```

## Rules

- **You classify, the server stores.** The server never calls an LLM.
- **One project, one graph.** Writes are scoped to the launch `--project`.
- **Ground concepts in artifacts.** Prefer `across` predicates that link a
  domain/business concept to the code or doc that realizes it.
- **Idempotency.** Same `idempotency_key` → converges, not duplicates.

## Example: distilling a design doc

```
ontology_read                                     → learn layers + predicates
index_docs   {content: doc-text, path: "docs/auth-design.md"}  → raw doc
store_knowledge {
  idempotency_key: "distill-auth-design-v1",
  facts: [
    {content: "AuthService uses JWT for stateless auth."},
    {content: "Rate limiting is enforced at the gateway, not per-handler."}
  ],
  entities: [
    {name: "AuthService", kind: "function"},        // technical
    {name: "JWT", kind: "concept"},                 // domain
    {name: "Rate Limiting", kind: "concept"},       // domain
    {name: "Gateway", kind: "module"}               // technical
  ],
  relationships: [
    {subject: "AuthService", predicate: "uses", object: "JWT"},
    {subject: "Rate Limiting", predicate: "governs", object: "Gateway"},
    {subject: "AuthService", predicate: "realized_by", object: "Gateway"}
  ]
}
recall({query: "rate limiting"})                   → verify
```

## Entity kind → ontology layer

| Ontology layer | Typical kinds | Examples |
|---|---|---|
| technical | `module`, `function`, `struct`, `trait`, `repository` | zbot-stores, SessionDistiller, EngramProvider |
| domain | `concept` | Distillation, MemoryFact, Belief, Episode, Procedure |
| business | `concept` | Onboarding Flow, Workflow, Stakeholder, Capability |

When a domain/business entity doesn't have a specific code-level kind, use
`concept` and rely on the ontology class (via `ontology_read`) for classification.
