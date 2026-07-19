-- stores/zbot-stores-sqlite/migrations/v25_memory_facts_valid_from_backfill.sql
--
-- Bi-temporal phase 1: backfill `valid_from` for legacy facts that predate
-- the trait-level wiring. Every new fact now records when it became true
-- in the world (default = `Utc::now()` at insert time); this one-time pass
-- sets the same value retroactively for rows that pre-date the change so
-- point-in-time queries (phase 2) can include them in "now" results
-- without special-casing NULL.
--
-- Idempotent and trigger-safe: a pure UPDATE on `memory_facts` does not
-- fire the DELETE trigger (`trg_facts_delete_vec`) that guards the
-- memory_facts_index vec0 partner table — so this migration can land
-- before `initialize_vec_tables_with_dim` materializes that virtual table.
--
-- Setting valid_from = created_at preserves the original "fact existed"
-- timestamp; we deliberately do NOT use updated_at, since an upsert can
-- bump updated_at long after the fact first became true.

UPDATE memory_facts
   SET valid_from = created_at
 WHERE valid_from IS NULL;
