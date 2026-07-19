# Engram semantic portability handoff

**Recorded:** 2026-07-11

## Decision

zbot is a host application, not a semantic database framework. Engram owns
semantic memory and knowledge persistence, including backend choice,
connections, schema, migrations, indexes, embeddings, graph queries, and
semantic maintenance. zbot owns product behavior and operational persistence.

The desired outcome is a plug-and-play semantic engine: switching Engram from
SQLite to Postgres or Surreal changes only Engram configuration and
Engram-managed data migration. It must not require zbot code changes or any
zbot-sidecar schema change.

## Non-negotiable boundary

```text
zbot runtime + product sidecar                 Engram semantic engine
--------------------------------                ----------------------
sessions, messages, executions                  facts + embeddings
checkpoints, traces, logs                       graph entities/edges
plans, goals, workflow metadata                 ontology + taxonomy
outbox, recall audit, run status                beliefs + contradictions
opaque Engram record references                 episodes + evidence
                                                recall + semantic maintenance
                                                backend lifecycle + migration
```

The zbot sidecar may store product/workflow rows and opaque Engram IDs. It must
not store semantic records, embeddings, graph edges, semantic indexes,
backend-specific migrations, database handles, or semantic SQL/query logic.

## Current state

- `zbot-runtime-sqlite` now owns `conversations.db` runtime persistence.
- `gateway`, `gateway-execution`, and `gateway-bridge` no longer link
  `zbot-stores-sqlite` in production; it is test-only legacy/parity support.
- Active facts, graph, ontology, taxonomy, and beliefs route through
  `zbot-engram-adapter` to Engram.
- The adapter still owns `zbot-sidecars.sqlite`, with `episodes`,
  `kg_episodes`, `compaction_audit`, `procedures`, `goals`, `recall_log`,
  `distillation_runs`, and `outbox`.
- Existing `conversations.db` compatibility tables must remain untouched; they
  are legacy residue, not new semantic write targets.

## Current blockers in Engram

`engram-integration` is not yet a complete host façade. It presently exposes
provider handles but zbot still imports lower-level `engram-*` crates and uses
`engram-conformance` to bootstrap.

Engram must deliver:

1. `engram-integration` as the only host runtime semantic dependency.
2. `EngramProvider::open(config)` owning driver selection, connections,
   bootstrap, capability evaluation, and migrations.
3. Stable public DTOs, opaque IDs, scopes, errors, provenance, and ranking
   trace types re-exported from the façade.
4. Generic provider APIs for episode/evidence lifecycle, contradictions, and
   maintenance (dedup, compact, reindex, health, operation history).
5. Conformance and backend-swap tests across SQLite, Postgres, and Surreal.

The full implementation contract and examples are maintained outside this
repository at:

- `~/Documents/engram-host-application-requirements.md` (§19 and §20)

## Crates after completion

### Delete after all callers are migrated

- `stores/zbot-stores-sqlite`
- `stores/zbot-stores`
- `stores/zbot-stores-traits`
- `stores/zbot-stores-conformance`
- semantic portions of `stores/zbot-stores-domain` (move any remaining runtime
  DTOs to a runtime-owned crate first)

### Keep

- `stores/zbot-runtime-sqlite`
- `stores/zbot-conversation`
- `stores/zbot-trace`
- `stores/zbot-engram-adapter`, reduced to a thin DTO/workflow mapper and
  product sidecar owner

### Shrink, do not necessarily delete

- `gateway-memory`: retain recall policy, context budgeting, and scheduling;
  remove semantic storage/query implementations.
- `services/knowledge-graph`: retain only zbot extraction/parsing helpers if
  they remain useful; remove persistence/query code.

## Small-task backlog

Each item can ship independently. Do not begin a later item before its stated
gate is met.

1. **Engram: façade re-exports.** Re-export all host DTOs/ports from
   `engram-integration`; add a compile fixture proving a host needs no direct
   `engram-memory`, `engram-knowledge`, `engram-belief`, or
   `engram-conformance` dependency.
2. **Engram: provider-owned open.** Add `EngramProvider::open` with typed
   backend profile, bootstrap, migration readiness, and capability reporting.
   Host code must not construct an adapter or pool.
3. **Engram: episode/evidence port.** Add lifecycle, query, and evidence-write
   APIs plus backend conformance. This unlocks retiring zbot `episodes` and
   `kg_episodes` sidecars.
4. **Engram: contradiction port.** Add create/query/resolve contradiction API
   and conformance. This unlocks zbot contradiction adapters.
5. **Engram: maintenance port.** Add dedup, compact, reindex, health, and
   operation history. This unlocks `compaction_audit` retirement.
6. **Engram: portability fixture.** Export/import SQLite semantic state into
   Postgres and Surreal; assert opaque IDs, scopes, provenance, and recall
   results survive.
7. **zbot: thin adapter conversion.** Change `zbot-engram-adapter` to depend
   only on `engram-integration`; remove `rusqlite` and direct lower-level
   Engram imports from production adapter code.
8. **zbot: sidecar split.** Keep goals, plans, procedures-as-workflows, outbox,
   recall audit, and run status. Move procedure semantic text/embeddings to
   Engram and retain only opaque references in zbot.
9. **zbot: semantic trait removal.** Replace remaining
   `zbot-stores*` semantic call sites with adapter/provider requests; reject
   new semantic SQLite writes.
10. **zbot: final retirement.** Import legacy semantic data through Engram,
    delete legacy semantic crates/tables only after no migration/parity caller
    remains and the backend-swap fixture passes.

## Required tests

- zbot compile fence: production code has no direct lower-level Engram,
  `zbot-stores-sqlite`, or backend-driver imports outside approved runtime
  persistence.
- Engram host compile fixture: one `engram-integration` dependency is enough.
- Capability test: unsupported semantic capability returns typed unavailable;
  zbot creates no fallback local semantic row.
- Sidecar invariance test: switch Engram backend with a populated zbot sidecar;
  sidecar schema and rows remain compatible and all referenced IDs resolve.
- Cross-backend Engram conformance: SQLite, Postgres, and Surreal share the
  same semantic contract.

## Resume checklist for a new session

1. Read this file and `~/Documents/engram-host-application-requirements.md`.
2. Check the current Engram provider surface:
   `sed -n '1,230p' ~/projects/mem-alpha/core/integration/src/provider.rs`.
3. Check remaining zbot adapter dependencies:
   `sed -n '1,100p' stores/zbot-engram-adapter/Cargo.toml`.
4. Review the visual boundary map:
   `docs/architecture/future-state/zbot-storage-map.html`.
5. Select the earliest unfinished small-task item above; do not delete a zbot
   semantic crate until its Engram capability and portability test exist.

## Related artifacts

- `docs/architecture/future-state/engram-adapter-capability-gaps.md`
- `docs/architecture/future-state/zbot-storage-map.html`
- `docs/backlog.md` → `engram-host-application-portability-contract`
- PR #219, branch `agent/engram-agent-surfaces-cleanup`
