-- v28: Add kg_belief_contradictions table for Belief Network Phase B-2.
--
-- A contradiction is a relationship between two beliefs that the LLM judge
-- classified as logical_contradiction or tension. Stored as a separate table
-- because contradictions have their own lifecycle (detection, severity,
-- resolution) distinct from the beliefs themselves.
--
-- belief_a_id is always the lexicographically smaller of the two — canonical
-- pair ordering. UNIQUE(belief_a_id, belief_b_id) prevents double-detection.
--
-- Idempotent: CREATE TABLE IF NOT EXISTS.

CREATE TABLE IF NOT EXISTS kg_belief_contradictions (
    id TEXT PRIMARY KEY,
    belief_a_id TEXT NOT NULL,
    belief_b_id TEXT NOT NULL,
    contradiction_type TEXT NOT NULL,   -- 'logical' | 'tension' | 'temporal'
    severity REAL NOT NULL,              -- 0.0..1.0
    judge_reasoning TEXT,                 -- LLM's explanation (one sentence)
    detected_at TEXT NOT NULL,
    resolved_at TEXT,
    resolution TEXT,                      -- 'a_won' | 'b_won' | 'compatible' | 'unresolved' | NULL
    FOREIGN KEY (belief_a_id) REFERENCES kg_beliefs(id) ON DELETE CASCADE,
    FOREIGN KEY (belief_b_id) REFERENCES kg_beliefs(id) ON DELETE CASCADE,
    UNIQUE(belief_a_id, belief_b_id)
);

CREATE INDEX IF NOT EXISTS idx_belief_contradictions_a ON kg_belief_contradictions(belief_a_id);
CREATE INDEX IF NOT EXISTS idx_belief_contradictions_b ON kg_belief_contradictions(belief_b_id);
CREATE INDEX IF NOT EXISTS idx_belief_contradictions_unresolved ON kg_belief_contradictions(detected_at) WHERE resolved_at IS NULL;
