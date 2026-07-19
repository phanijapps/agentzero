-- stores/zbot-stores-sqlite/migrations/v24_global_scope_backfill.sql
--
-- One-time backfill: promote facts in "global-type" categories from the
-- default scope='agent' to scope='global' so they are visible to every
-- agent via the scope-aware search filter.
--
-- Idempotent AND drift-safe: on every run this migration performs a
-- single guarded UPDATE. Rows whose promotion would collide with an
-- already-promoted global row (same agent_id / ward_id / key) are
-- SKIPPED via `NOT EXISTS`, not deleted. That avoids firing the
-- memory_facts DELETE trigger (`trg_facts_delete_vec`) which references
-- the memory_facts_index vec0 table — a table that may not yet exist
-- at this point in startup (it's created later by
-- `initialize_vec_tables_with_dim`). Firing the trigger when the vec0
-- table is missing crashes schema init with
-- "no such table: main.memory_facts_index".
--
-- Redundant scope='agent' rows left behind by the skip are harmless —
-- the global version serves the same lookup. A future hygiene pass can
-- evict them once memory_facts_index is guaranteed present.

UPDATE memory_facts
   SET scope = 'global'
 WHERE scope = 'agent'
   AND category IN ('domain', 'reference', 'book', 'research', 'user')
   AND NOT EXISTS (
       SELECT 1 FROM memory_facts AS dst
        WHERE dst.scope = 'global'
          AND dst.agent_id = memory_facts.agent_id
          AND COALESCE(dst.ward_id, '') = COALESCE(memory_facts.ward_id, '')
          AND dst.key = memory_facts.key
   );
