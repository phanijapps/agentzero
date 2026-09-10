# Engram Graph Noise Audit

**Date:** 2026-08-07
**Status:** Read-only audit
**Database:** `/home/videogamer/Documents/zbot/data/engram/engram_data.db`

## Summary

The vault database is structurally healthy but its semantic graph has enough
duplication, unconstrained predicates, and missing provenance to make broad
graph traversal unreliable. Separately, the live Engram MCP scope presents a
far larger graph containing cross-repository scan data; that is not explained
by the audited vault database and must be traced as a separate backing-store or
scope-resolution problem.

No database data was modified for this audit.

## Evidence

`PRAGMA integrity_check` returned `ok`. The file was 66 MB with a 4 MB active
WAL at the time of inspection.

| Table or view | Count | Finding |
| --- | ---: | --- |
| `kg_entities` | 399 | Agent-scoped semantic entities, not a code scan corpus. |
| `kg_relationships` | 647 | Relationships have no stored confidence values. |
| `knowledge_sources` | 5 | All are generated AgentZero ward-wiki sources. |
| `knowledge_graphs` | 0 | No source/revision graph partition exists. |
| `knowledge_entities` | 399 | Every record has a null `graph_id`. |

The live MCP returned a materially different graph: 56,538 communities and
central nodes including generic names such as `get`, `str`, and `append`, plus
Hermes-Agent scan-source nodes. It also reported a z-Bot scan of 17,711
entities and 28,654 relationships. The vault database therefore cannot be the
only corpus contributing to the live `default / agentzero` view.

## Stored graph noise

### 1. Per-agent duplicates

The same named entity is stored repeatedly under different agent IDs. Examples
include wards, files, tools, projects, organizations, and people. One ward has
six copies; several named files and tools have three or four. This fragments
the graph: callers must choose an agent-local duplicate rather than traverse a
canonical shared entity.

Some duplication may be intentional private-agent state. The current schema,
however, does not distinguish that case from a shared canonical fact, and it
does not record an alias or `same_as` relationship that a query can use safely.

### 2. Identity aliases

Equivalent user identities occur as independent entities with casing and naming
variants. These aliases attach relationships to separate nodes and therefore
split user-centered subgraphs.

### 3. Predicate proliferation

The graph uses a large set of relationship spellings. High-frequency generic
predicates include `part_of` (136), `uses` (132), and `related_to` (63), while
many predicates occur once or twice. Variants such as `documents` /
`documentedin`, `associatedwith`, and other prose-derived forms prevent
consistent traversal and make aggregation arbitrary.

Every relationship has a null `confidence`. Consumers cannot distinguish a
high-confidence structured fact from an inference made from incidental prose.

### 4. Type and edge quality

The graph includes durable facts, files, tools, concepts, people, projects,
and derived wiki content in one topology. Some extracted type/predicate pairs
are semantically weak or inconsistent. In particular, generic edges and
misclassified tool/project relationships can become traversal shortcuts even
when they do not represent a stable operational dependency.

### 5. Missing source graph identity

There are no rows in `knowledge_graphs`, and `graph_id` is null on the stored
knowledge entities. A consumer cannot filter by repository, source document,
ward, scan revision, or source lifecycle. This is the primary blocker to
maintaining a reliable code graph alongside a semantic memory graph.

## Live MCP noise

The MCP's architecture and code-health outputs show a different kind of noise:

- generic language and library methods dominate centrality;
- scanner/source records themselves become graph nodes;
- the `agentzero` workspace includes Hermes-Agent and Engram records;
- exact type/module queries can return no edges while a same-name function
  returns callers and an implausibly broad callee set.

This indicates that the live graph needs both corpus isolation and qualified
symbol identity. Re-scanning z-Bot adds current records but does not remove or
hide earlier scan data from the same default scope.

## Remediation sequence

1. **Locate the MCP backing store and scope mapper.** Prove which database and
   tenant/workspace mapping services each MCP tool uses before altering any
   data. The live graph and this vault DB do not presently agree.
2. **Partition code scans.** Create a `graph_id` for every scan from repository
   remote, revision, path, and scanner version. Default code queries to one
   repository and latest revision.
3. **Make scans convergent.** An update must upsert current records and
   tombstone records from the same repository/source that no longer exist.
   It must not merge unrelated repositories merely because they share a
   workspace name.
4. **Canonicalize entities.** Store a normalized identity key and explicit
   alias/merge relation. Preserve agent-private facts only when their privacy
   scope requires a separate node.
5. **Constrain predicates.** Map extraction output to a governed vocabulary
   such as `calls`, `implements`, `reads`, `writes`, `uses`, `part_of`, and
   `tests`. Retain raw phrasing as evidence, not as topology.
6. **Require provenance and confidence.** Every node and edge needs source,
   extractor/scanner, observed time, confidence, and visibility. Traversal
   should suppress low-confidence and generic edges by default.
7. **Filter graph metrics.** Exclude built-ins, standard-library methods,
   generated/dependency sources, scanner/source nodes, Markdown headings, and
   test-framework plumbing from architectural centrality.
8. **Separate semantic and code views.** Ward-wiki and durable-memory facts are
   valuable retrieval evidence. They should not participate in code call-graph
   or architecture-hub queries unless explicitly requested.

## Acceptance checks

- The live `agentzero` code view contains only the selected z-Bot repository
  and revision.
- An architecture query names qualified z-Bot boundaries rather than `get`,
  `str`, or scanner-source records.
- A symbol query returns callers/callees annotated with path, repository,
  qualified symbol, edge type, source range, and confidence.
- Semantically equivalent shared entities resolve to one canonical identity or
  declare their intentional scope separation.
- Predicate aggregation produces a small governed vocabulary, while raw
  extraction language remains searchable evidence.
- A fresh scan does not increase visible data from a prior Hermes-Agent or
  Engram scan.
