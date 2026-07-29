---
name: engram-ingest
description: Ingest unstructured docs, transcripts, or design notes into the engram knowledge graph as ontology-classified entities, relationships, and facts. Read the active ontology first, extract knowledge from the material, classify every entity into a layer (technical/domain/business), use only configured predicates, and write via the engram MCP tools. Use when you encounter docs worth remembering for a project — READMEs, ADRs, meeting notes, transcripts, architecture docs.
---

# engram-ingest

Turn unstructured material into a queryable, multi-layer knowledge graph by
writing it through the **engram** MCP server. The server is a deterministic
store — **you** (the agent) do the extraction and classification; the server
persists and retrieves. No LLM runs inside the server.

## When to run

- You just read a doc / transcript / RFC / README / design note worth remembering.
- You are building a project's knowledge graph (code + docs + domain knowledge).
- Before a task, to ground yourself: run `recall` or `search`.

## Prerequisites

The `engram` MCP server is configured with a multi-layer ontology + taxonomy:
```
engram-mcp --storage <db> --project <name>
           --ontology <layers.json> --taxonomy <concepts.json>
```

## Workflow

### 1. Read the active ontology + taxonomy

Call `ontology_read` — this returns the **layers** (technical / domain / business
/ …), their **classes**, and the allowed **within** / **across** predicates.
Call `taxonomy_read` for the concept hierarchy. **Every entity you extract must
map to a class in one of these layers; every relationship must use a configured
predicate.** Do not invent classes or predicates.

### 2. Index raw docs (optional)

For material you want retrievable verbatim, call `index_docs` with the Markdown
or text content. The server chunks it by structure (headers / code / prose)
into the **docs** lane — these chunks are searchable alongside the knowledge
graph.

### 3. Extract (your reasoning)

From the material, produce three kinds of structured knowledge:

- **facts** — `{ "content": "…" }` free-text observations worth remembering.
- **entities** — `{ "name", "kind" }` where `kind` is a valid entity kind
  AND maps to an ontology class. The entity kind must be one of the known
  variants (`concept`, `function`, `module`, `api`, `organization`, …). Unknown
  kinds are rejected.
- **relationships** — `{ "subject", "predicate", "object" }` using **only** the
  ontology's predicates:
  - **within** a layer: `depends_on`, `implements`, `contains`, `part_of`, `uses`.
  - **across** layers: `realized_by`, `describes`, `governs`, `distilled_from`,
    `extracted_from`, `contradicts`.

**Classify across layers** — a technical concept (e.g. `zbot-engram-adapter`)
`realized_by` a domain concept (e.g. `Engram Integration`); a doc `describes` a
code module; a business rule `governs` a workflow. **Cross-layer edges are the
point** — they link the semantic graph to the code.

### 4. Write the batch

Call `store_knowledge` with `{ facts, entities, relationships, idempotency_key }`.

- Supply a **stable `idempotency_key`** (e.g. `"distill-<doc-name>-v1"`) so
  re-sending the same batch dedups instead of doubling.
- The write is **best-effort, not ACID** — the result surfaces per-step status.
- Malformed entries (missing required fields) are skipped and reported.

### 5. Verify

Call `recall` with a query drawn from what you wrote — confirm it returns. Call
`search` to find entities by name. Use `lanes` on recall (`memory` / `knowledge`
/ `docs` / `beliefs`) to scope.

## Rules

- **You classify, the server stores.** Decide each entity's layer/class and each
  relationship's predicate from `ontology_read`. If you need a class or predicate
  that isn't configured, note it — don't invent.
- **One project, one graph.** Writes are scoped to the launch `--project`. Do
  not try to cross projects from one skill run.
- **Ground concepts in artifacts.** Prefer `across` predicates that link a
  domain/business concept to the code or doc that realizes it.
- **Idempotency.** Re-running the same distillation with the same
  `idempotency_key` should converge, not duplicate.

## Example sequence

```
ontology_read                                     → learn layers + predicates
taxonomy_read                                     → learn concept hierarchy
index_docs      {content, path}                   → persist the raw doc (docs lane)
store_knowledge {facts, entities,                 → persist extracted KG (best-effort)
                 relationships,
                 idempotency_key}
recall          {query, lanes: ["knowledge"]}     → verify entities are retrievable
search          {query}                            → find entities by name
```

## Entity kind → ontology class mapping

| Ontology layer | Typical entity kinds |
|---|---|
| technical | `module`, `function`, `struct`, `trait`, `repository` |
| domain | `concept` (Agent, MemoryFact, Belief, Distillation, Episode, …) |
| business | `concept` (Workflow, Task, Stakeholder, Capability, …) |

When extracting, set the entity `kind` to the Rust-level variant that best
matches the ontology class. Use `concept` for domain/business entities that
don't have a specific code-level kind.
