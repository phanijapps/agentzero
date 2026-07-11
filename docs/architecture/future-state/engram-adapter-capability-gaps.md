# Engram adapter capability gaps

## Decision

zbot owns runtime/session persistence and product behavior. Engram owns every
semantic-memory and graph persistence capability. `zbot-engram-adapter` keeps
a small zbot sidecar for product/runtime records, but may not introduce a
second semantic database, schema, migration path, or backend-specific query.

This note records the remaining generic capabilities needed before zbot can
retire its semantic store crates. The source contract is
`~/Documents/engram-host-application-requirements.md`.

## Already available: adapt directly

The adapter can use `EngramProvider` today for memory facts, graph entities and
relationships, ontology, taxonomy, beliefs, embeddings, provenance queries,
batch ingest, unified recall, migration/export-import, and observability when
the provider capability report says each handle is supported.

Adapter rules:

- Read `provider.capabilities()` during bootstrap and expose unsupported
  features as typed unavailable states; never silently fall back to SQLite.
- Use provider handles rather than opening a database or calling an Engram
  adapter implementation directly.
- Keep zbot-specific response, prompt, and UI mapping at the edge only.

## Missing generic Engram capabilities

| Capability | Current zbot duplication | Required Engram provider addition | zbot action after delivery |
| --- | --- | --- | --- |
| Episode lifecycle | `EpisodeStore` and `KgEpisodeStore` sidecars | Typed episode record/query service and provider handle; evidence links by scope/source/time | Delete episode sidecars and route distillation/tool evidence through Engram. |
| Evidence writes | zbot provenance/episode sidecars | Generic attach/list evidence API for facts, graph, and beliefs | Delete zbot evidence persistence. |
| Contradictions | `BeliefContradictionStore` mapping | Provider contradiction handle with create/query/resolve operations | Delete contradiction sidecar and adapter-specific model. |
| Maintenance | `CompactionStore`, graph compactor/pruner sidecars | Provider maintenance handle for dedup, compact, reindex, and backend health | Replace zbot semantic maintenance storage with Engram operation results. |
| Host facade exports | adapter imports several `engram-*` core crates | Re-export public port traits and DTOs, or expose façade request/response methods | Remove direct `engram-memory`, `engram-knowledge`, and `engram-belief` dependencies. |

## zbot sidecar: retained product/runtime data

Goals, procedures, agent plans, runtime checkpoints, bridge outbox records,
conversation messages, execution logs, traces, distillation run status, and
recall audit events are zbot product/runtime data. They may stay in zbot's
sidecar or runtime stores, but must not be represented as a parallel graph or
memory backend. Semantic retrieval or provenance must refer to an Engram record
rather than duplicate it.

## Retirement order

1. Move production users of currently supported provider capabilities from
   zbot store traits to adapter/provider calls.
2. Add the missing provider capabilities upstream in Engram with conformance
   fixtures and typed unsupported states.
3. Remove only semantic sidecar records for episodes, evidence, contradictions,
   and semantic maintenance; retain the product/runtime sidecar.
4. Remove `zbot-stores-sqlite`, `zbot-stores`, `zbot-stores-traits`,
   `zbot-stores-domain`, and `zbot-stores-conformance` once no production or
   migration/parity caller remains.

Until step 2 is complete, no new semantic SQLite table or zbot-local semantic
trait should be added.
