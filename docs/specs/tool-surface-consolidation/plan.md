# Plan: Tool-surface consolidation (SRP) + typed store errors

## Problem

The model-facing tool surface violates Single Responsibility and doubles up
recall:

1. `memory` is a god tool (3,284 lines, 10 actions) spanning three stores:
   a legacy file-backed KV (get/set/delete/list/search), the durable fact
   store (save_fact/recall/get_fact), and belief reads
   (belief/contradictions). Its description is a 13-line wall of text —
   poor discoverability, model has to parse which action hits which store.
2. Two overlapping recall surfaces: `memory.recall` and the narrow `recall`
   tool both do unified/facts recall with different schemas and validation.
3. The KV surface is dead: zero KV calls in the last ~3,000 production
   messages, and no memory.json / shared-file JSONs exist in the vault.
4. Store traits are stringly typed (`Result<_, String>`, 400+ sites) —
   Open/Closed and Dependency-Inversion debt that makes every store change
   a stringly-error hunt.

## Changes

### A1. `memory` shrinks to the durable fact surface

Delete (dead in production, never written):
- KV actions get/set/delete/list/search + `resolve_memory_path`,
  `load_store_at_path`, `save_store_at_path`, `MemoryStore`,
  `MemoryEntry`, `SHARED_FILES`, entry/size limits, file locking
- `memory.recall` action (consolidated into the `recall` tool)

Keep: `save_fact`, `get_fact`. Description becomes two sentences.

### A2. `recall` absorbs `memory.recall`'s features

Add `as_of` (bi-temporal cutoff) and `mode` (unified|facts) parameters to
the narrow `recall` tool. One recall surface, strict validation stays.

### A3. `belief` tool splits out

New `BeliefTool` with `belief` + `contradictions` actions — its own schema,
its own description ("read synthesized beliefs and contradictions"), wired
from the same stores. Registered under MemoryRead capability.

### B. `StoreError` at trait level

`zbot-stores-traits` gains:

```rust
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("unavailable: {0}")] Unavailable(String),
    #[error("not found: {0}")] NotFound(String),
    #[error("invalid: {0}")] Invalid(String),
    #[error("conflict: {0}")] Conflict(String),
    #[error("backend: {0}")] Backend(String),
}
```

All trait signatures become `Result<_, StoreError>`; impls map backend
errors to variants; `From<String>` keeps the migration mechanical.
Default-impl error strings become `StoreError::Unavailable(...)`.

## Execution

- Phase A (memory/recall/belief): direct edits, tests move with the code
- Phase B: worker-driven mechanical migration per crate, conformance green
