-- v27: Add kg_beliefs table for the Belief Network (Phase B-1).
--
-- A belief is an aggregate over one or more memory_facts about a single subject.
-- Confidence is derived from constituent fact confidences + recency.
-- Schema is bi-temporal (valid_from / valid_until) and partition-scoped
-- (partition_id — generic naming chosen day one; R-series rename in the
-- genericness audit won't need to touch this table).
--
-- Idempotent: CREATE TABLE IF NOT EXISTS.

CREATE TABLE IF NOT EXISTS kg_beliefs (
    id TEXT PRIMARY KEY,
    partition_id TEXT NOT NULL,
    subject TEXT NOT NULL,
    content TEXT NOT NULL,
    confidence REAL NOT NULL,
    valid_from TEXT,
    valid_until TEXT,
    source_fact_ids TEXT NOT NULL,         -- JSON array
    synthesizer_version INTEGER NOT NULL DEFAULT 1,
    reasoning TEXT,                          -- LLM's explanation for multi-fact synthesis (NULL when short-circuited)
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    superseded_by TEXT,
    UNIQUE(partition_id, subject, valid_from)
);

CREATE INDEX IF NOT EXISTS idx_beliefs_partition_subject ON kg_beliefs(partition_id, subject);
CREATE INDEX IF NOT EXISTS idx_beliefs_valid ON kg_beliefs(valid_from, valid_until);
