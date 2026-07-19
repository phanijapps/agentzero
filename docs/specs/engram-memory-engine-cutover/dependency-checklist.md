# Engram Dependency Checklist

Status: recorded for the 2026-07-06 provider-selection slice.

This checklist is path-free by design. It records capability and provenance
evidence without committing local checkout paths, private DB paths, transcripts,
row contents, embeddings, connector data, or secrets.

## Capability Rows

| Row | Status | Evidence |
| --- | --- | --- |
| provider_facade | Implemented | `zbot-engram-adapter` opens Engram through `EngramConfig` and `bootstrap_provider`. |
| sqlite_open_options | Implemented | Adapter config resolves a trusted data root, confines `engramPath`, and passes only the confined storage path to Engram. |
| sqlite_single_file_layout | Implemented | Adapter and gateway settings pass Engram's public `SqliteStorageLayout::SingleFile` option; Engram mode defaults to `engram_data.db` for core Engram stores and adapter compatibility tables. |
| scope_mapping | Implemented | Tenant, ward, session, and partition scope fixtures exist in adapter tests. |
| memory_repository | Implemented | Memory facts map to Engram memory records with adapter sidecar parity for zbot query/read-model fields. |
| knowledge_repository | Implemented | Wiki, graph, and hierarchy stores use Engram knowledge/hierarchy ports plus adapter sidecars for zbot read models. |
| belief_repository | Implemented | Belief and contradiction lifecycle uses Engram belief ports with adapter sidecar parity. |
| adapter_sidecars | Sidecar | Procedures, episodes, KG episodes, goals, recall logs, compaction audit rows, and outbox-like zbot-only records remain adapter-owned. Distillation run status remains on the existing conversation DB contract in gateway mode. |
| retrieval_ranking_trace | Missing, not provider-blocking | Upstream retrieval/ranking trace port is not yet available; recall support remains explicitly unsupported. |
| migration_manifest_gate | Implemented | Dry-run manifest fingerprint gates apply; non-empty source row import blocks apply until mappings are implemented. |

## Evidence

| Evidence | Status | Notes |
| --- | --- | --- |
| `cargo metadata --locked` | Recorded | Workspace metadata includes `zbot-engram-adapter` and GitHub-sourced Engram crates. |
| `Cargo.lock` | Recorded | Lockfile contains the Engram crate set, resolved Git revision, and adapter package entry. |
| Dependency scanner | Recorded | Repository search confirms Engram implementation imports stay behind `zbot-engram-adapter`; gateway imports only the adapter. |
| Engram source revision | Recorded | `https://github.com/phanijapps/engram` `main` revision is recorded in `Cargo.lock`. |
| Dirty-state policy | Recorded | Untracked local tool-cache state such as `.serena/` is not a release input; release/publish still requires a pinned source decision. |

## Provider-Selection Gate

Provider selection completed under the 2026-07-06 implementation waiver.
Engram is the default runtime semantic memory provider. Gateway skips active
`knowledge.db` initialization, SQLite embedding reindex, and SQLite KG backfill,
then routes memory/knowledge/wiki/procedure/episode/KG episode/compaction/belief
trait stores through `zbot-engram-adapter`. `conversations.db` remains the
zbot-owned SQLite runtime DB for conversation/execution/outbox concerns.
The selected Engram layout is single-file SQLite, so the core Engram stores and
adapter compatibility tables share `engram/engram_data.db` rather than creating
one DB per store family or zbot sidecar.

Apply migration remains marker-only until row import mappings exist. A dry-run
with non-empty allowlisted source tables produces `row_import_not_implemented`
blockers and apply refuses to write Engram storage.
