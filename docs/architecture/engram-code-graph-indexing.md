# Engram Code-Graph Indexing Guidance

**Status:** Operational guidance

This document describes which AgentZero repository facts are useful as graph
nodes and edges, which facts are useful only as retrieval evidence, and which
inputs should be excluded. The goal is an index that can answer an engineering
question with a small, source-linked graph rather than a large popularity
ranking of language tokens.

## The decision rule

Index something in the **knowledge graph** when it has a stable identity and
its relationship to another thing helps answer one of these questions:

- Who calls, implements, owns, reads, writes, publishes, or configures this?
- What is the path from an entry point to a side effect or durable state?
- What code and tests are affected if this changes?

Keep something as **retrieval evidence** when it explains, describes, or
verifies the system but is not itself a durable program relationship. Exclude
it when it has no useful identity, is generated/duplicated, or is unsafe to
retain.

## Index as graph entities

Use a qualified identity for every code entity. A bare name such as `get`,
`run`, `main`, or `append` is never a sufficient node identity.

| Entity | Useful identity | Why it belongs in the graph |
| --- | --- | --- |
| Workspace package and crate | workspace + Cargo package + module path | Shows layer ownership and dependency direction. |
| Module, type, trait, and interface | repository revision + path + qualified name | Captures ownership and implementation boundaries. |
| Public function or method | receiver/type + qualified name + signature | Enables entry-point, caller, and blast-radius queries. |
| Meaningful private function | same identity, only when it crosses a subsystem concern | Completes an execution or persistence path without indexing every helper. |
| API route, WebSocket message, tool, event, and job | protocol identifier plus implementation symbol | Connects user-visible behavior to runtime code. |
| Persistent domain concept | schema/table/message name plus owning code | Connects reads/writes to durable state and migrations. |
| Configuration key or capability | canonical key plus owning service | Explains configuration-to-behavior paths. |
| Test case or fixture | test path + fully-qualified test name | Records behavioral coverage of a symbol or contract. |

For AgentZero, this means the graph should make it possible to traverse a
request through the HTTP/WebSocket handler, gateway execution entry point,
agent runtime facade, tool/event handling, persistence, and client delivery.
It should also represent products seams such as `respond`, `StreamEvent`,
store traits, MCP/connector boundaries, and the `zbot-engram-adapter`.

## Index as graph relationships

Prefer typed edges with a direction and provenance. High-value examples are:

| Edge | Example use |
| --- | --- |
| `contains` / `owns` | crate → module → symbol; service → configuration key |
| `calls` | runner → response resolver |
| `implements` | concrete store → zbot store trait |
| `routes_to` / `handles` | HTTP or WebSocket route → handler → execution entry point |
| `emits` / `consumes` | runtime → `StreamEvent` → gateway/UI delivery path |
| `reads` / `writes` | symbol → durable record, table, or repository port |
| `invokes` | model turn → tool or MCP capability |
| `tests` | test → symbol, route, or regression invariant |
| `configured_by` | runtime behavior → settings key/provider/agent configuration |
| `migrates` | migration → persistent schema or compatibility contract |

For every edge, retain the source file, source range when available, scanner
revision, and extraction confidence. This lets callers distinguish a
Tree-sitter-derived call from a semantic/documented relationship.

## Keep as retrieval evidence, not graph topology

These inputs remain valuable, but should be chunked and retrieved as evidence
instead of participating in centrality or call-graph traversal:

- Architecture documents, RFCs, ADRs, specifications, plans, and changelogs.
- `AGENTS.md` instructions and subsystem ownership notes.
- Test failure text, tracebacks, benchmark output, and run logs.
- Code comments, README examples, and long source excerpts.
- Historical knowledge notes and incident summaries, including K-0001.

Document chunks may link to graph entities through `documents`, `specifies`,
or `explains` edges, but their headings, Markdown link labels, and prose words
must not become call-graph nodes. A plan is evidence for intent; the source
symbol and its test are evidence for current behavior.

## Exclude or suppress

Do not index the following as ordinary graph entities or call edges:

- Language and standard-library primitives (`get`, `str`, `append`, `new`,
  `clone`, `read`, `write`, and equivalent syntax-level operations).
- Third-party dependency sources, package-manager trees, lockfiles, vendored
  code, generated code, build artifacts, and coverage output, unless a
  specific provenance/security index is intentionally built for them.
- Test-framework plumbing, assertion helpers, mocks, fixture builders, and
  test names as call-graph hubs. Keep only their `tests` relationship to the
  behavior they verify.
- Duplicate code from old revisions or sibling repositories in the same
  workspace scope.
- Credentials, access tokens, raw customer data, private prompts, and any
  other content not authorized for the knowledge store.
- Ephemeral scratch files, temporary files, node modules, target directories,
  and generated snapshots.

If such data must be searchable, keep it in a separate corpus/type that is
excluded from architecture, impact, and centrality queries by default.

## Identity, scope, and lifecycle

The minimum identity for a graph node is:

```text
tenant / workspace / repository remote / revision / path / qualified symbol
```

`workspace` is a visibility boundary, not a substitute for repository
identity. Two repositories may be intentionally visible to one workspace, but
queries must be able to require one repository and one revision.

Scanning must be convergent. A new scan of a repository/revision should upsert
current entities and edges and remove or tombstone records that disappeared
from that indexed source. It must not leave prior repositories or old revisions
in the same default query result unless the caller explicitly asks for history.

## Ranking and query behavior

Centrality should be computed only over eligible, qualified code entities and
typed program edges. Downweight or remove broad hubs, generated sources,
document nodes, and external repositories. A route handler, trait boundary, or
execution facade can be a useful hub; `get` cannot.

Query defaults should be:

1. Restrict to the requested repository and latest indexed revision.
2. Resolve exact qualified symbols before semantic recall.
3. For code questions, return code entities and direct tests first.
4. Include document evidence only as a separately labelled supporting section.
5. Bound results by count and byte size while preserving path, symbol, type,
   score, revision, and provenance for every retained result.

## Acceptance checks

The index is useful when the following queries return a short, source-linked,
single-repository result:

- `symbol_context(assistant_turn_content)` identifies its continuation and
  tool-result callers without unrelated repositories or generic callees.
- A search for `respond assistant row continuation` returns the response
  resolver and its direct tests before planning documents.
- A route-to-storage trace exposes the path from request handler through
  `gateway-execution` and runtime to persisted assistant content and emitted
  client events.
- Architecture hubs name qualified AgentZero boundaries, not generic method
  names or scanner/source records.
- Re-scanning AgentZero does not increase result contamination from a previous
  Hermes-Agent or Engram scan.

These checks measure retrieval quality, graph correctness, and index lifecycle
independently. A graph that has many entities but cannot pass them is large,
not useful.
