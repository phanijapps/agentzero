// ============================================================================
// DATABASE SCHEMA
// SQLite schema for sessions, agent executions, and messages
// ============================================================================

use rusqlite::{Connection, OptionalExtension, Result};

/// Current schema version
const SCHEMA_VERSION: i32 = 27;

fn create_durable_work_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS durable_work_items (
            id TEXT PRIMARY KEY,
            envelope_version INTEGER NOT NULL,
            kind TEXT NOT NULL,
            source TEXT NOT NULL,
            target TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            provenance_node_id TEXT NOT NULL,
            provenance_actor_id TEXT NOT NULL,
            provenance_session_id TEXT NOT NULL,
            provenance_execution_id TEXT NOT NULL,
            correlation_id TEXT,
            dedupe_key TEXT,
            priority INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'pending',
            attempts INTEGER NOT NULL DEFAULT 0,
            max_attempts INTEGER NOT NULL,
            available_at TEXT NOT NULL,
            lease_owner TEXT,
            lease_token TEXT,
            lease_expires_at TEXT,
            last_failure_code TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            completed_at TEXT,
            CHECK (envelope_version = 1),
            CHECK (length(CAST(payload_json AS BLOB)) <= 65536),
            CHECK (status IN ('pending', 'leased', 'completed', 'dead_letter', 'canceled')),
            CHECK (attempts >= 0),
            CHECK (max_attempts BETWEEN 1 AND 20)
        );
        CREATE UNIQUE INDEX IF NOT EXISTS uq_durable_work_source_dedupe
            ON durable_work_items(source, dedupe_key)
            WHERE dedupe_key IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_durable_work_claim
            ON durable_work_items(
                target,
                status,
                priority DESC,
                available_at ASC,
                created_at ASC,
                id ASC
            );
        CREATE INDEX IF NOT EXISTS idx_durable_work_expired_lease
            ON durable_work_items(status, lease_expires_at);
        CREATE INDEX IF NOT EXISTS idx_durable_work_scope_page
            ON durable_work_items(
                source,
                kind,
                provenance_actor_id,
                updated_at DESC,
                id DESC
            );
        CREATE INDEX IF NOT EXISTS idx_durable_work_scope_correlation
            ON durable_work_items(
                source,
                kind,
                provenance_actor_id,
                correlation_id
            );",
    )
}

fn migrate_durable_work_to_v27(conn: &Connection) -> Result<()> {
    let table_sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master
             WHERE type = 'table' AND name = 'durable_work_items'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let Some(table_sql) = table_sql else {
        return create_durable_work_schema(conn);
    };
    if table_sql.contains("'canceled'") {
        return create_durable_work_schema(conn);
    }

    conn.execute_batch(
        "BEGIN IMMEDIATE;
         DROP INDEX IF EXISTS uq_durable_work_source_dedupe;
         DROP INDEX IF EXISTS idx_durable_work_claim;
         DROP INDEX IF EXISTS idx_durable_work_expired_lease;
         DROP INDEX IF EXISTS idx_durable_work_scope_page;
         DROP INDEX IF EXISTS idx_durable_work_scope_correlation;
         ALTER TABLE durable_work_items RENAME TO durable_work_items_v26;",
    )?;
    let migrated = (|| {
        create_durable_work_schema(conn)?;
        conn.execute_batch(
            "INSERT INTO durable_work_items (
                id, envelope_version, kind, source, target, payload_json,
                provenance_node_id, provenance_actor_id,
                provenance_session_id, provenance_execution_id,
                correlation_id, dedupe_key, priority, status, attempts,
                max_attempts, available_at, lease_owner, lease_token,
                lease_expires_at, last_failure_code, created_at, updated_at,
                completed_at
             )
             SELECT
                id, envelope_version, kind, source, target, payload_json,
                provenance_node_id, provenance_actor_id,
                provenance_session_id, provenance_execution_id,
                correlation_id, dedupe_key, priority, status, attempts,
                max_attempts, available_at, lease_owner, lease_token,
                lease_expires_at, last_failure_code, created_at, updated_at,
                completed_at
             FROM durable_work_items_v26;
             DROP TABLE durable_work_items_v26;
             COMMIT;",
        )
    })();
    if migrated.is_err() {
        let _ = conn.execute_batch("ROLLBACK;");
    }
    migrated
}

/// Run migrations for existing databases.
///
/// Checks the current schema version and applies any needed migrations.
fn migrate_database(conn: &Connection) -> Result<()> {
    // Check if schema_version table exists
    let has_version: bool = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='schema_version'",
        [],
        |row| row.get::<_, i64>(0),
    )? > 0;

    if !has_version {
        return Ok(()); // Fresh database, no migration needed
    }

    let version: i32 = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap_or(0);

    // v8 → v9: Add routing fields to sessions
    if version < 9 {
        // Use try/ignore pattern since columns may already exist on fresh DB
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN thread_id TEXT", []);
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN connector_id TEXT", []);
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN respond_to TEXT", []);
    }

    // v9 → v10: Add bridge_outbox table
    if version < 10 {
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS bridge_outbox (
                id TEXT PRIMARY KEY,
                adapter_id TEXT NOT NULL,
                capability TEXT NOT NULL,
                payload TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                session_id TEXT,
                thread_id TEXT,
                agent_id TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                sent_at TEXT,
                error TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                retry_after TEXT
            )",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_outbox_adapter_status ON bridge_outbox(adapter_id, status)",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_outbox_created ON bridge_outbox(created_at)",
            [],
        );
    }

    // v10 → v11: Add distillation_runs, session_episodes tables; add ward_id to memory_facts
    if version < 11 {
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS distillation_runs (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL UNIQUE,
                status TEXT NOT NULL,
                facts_extracted INTEGER DEFAULT 0,
                entities_extracted INTEGER DEFAULT 0,
                relationships_extracted INTEGER DEFAULT 0,
                episode_created INTEGER DEFAULT 0,
                error TEXT,
                retry_count INTEGER DEFAULT 0,
                duration_ms INTEGER,
                created_at TEXT NOT NULL
            )",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_distillation_runs_status ON distillation_runs(status)",
            [],
        );

        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS session_episodes (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                ward_id TEXT NOT NULL DEFAULT '__global__',
                task_summary TEXT NOT NULL,
                outcome TEXT NOT NULL,
                strategy_used TEXT,
                key_learnings TEXT,
                token_cost INTEGER,
                embedding BLOB,
                created_at TEXT NOT NULL
            )",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_session_episodes_agent ON session_episodes(agent_id)",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_session_episodes_ward ON session_episodes(ward_id)",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_session_episodes_outcome ON session_episodes(outcome)",
            [],
        );

        // Add ward_id column to memory_facts
        let _ = conn.execute(
            "ALTER TABLE memory_facts ADD COLUMN ward_id TEXT NOT NULL DEFAULT '__global__'",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_memory_facts_ward ON memory_facts(ward_id)",
            [],
        );
        // Drop the old unique constraint (it was an inline UNIQUE, which SQLite
        // implements as an auto-named index "sqlite_autoindex_memory_facts_1")
        let _ = conn.execute("DROP INDEX IF EXISTS sqlite_autoindex_memory_facts_1", []);
        // Create the new unique constraint including ward_id
        let _ = conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS uq_memory_facts_agent_scope_ward_key ON memory_facts(agent_id, scope, ward_id, key)",
            [],
        );
    }

    // v11 → v12: Add contradicted_by column to memory_facts
    if version < 12 {
        let _ = conn.execute(
            "ALTER TABLE memory_facts ADD COLUMN contradicted_by TEXT",
            [],
        );
    }

    // v12 → v13: Add recall_log, memory_facts_archive tables; add archived to sessions
    if version < 13 {
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS recall_log (
                session_id TEXT NOT NULL,
                fact_key TEXT NOT NULL,
                recalled_at TEXT NOT NULL,
                PRIMARY KEY (session_id, fact_key)
            )",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_recall_log_session ON recall_log(session_id)",
            [],
        );

        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS memory_facts_archive (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                scope TEXT NOT NULL DEFAULT 'agent',
                category TEXT NOT NULL,
                key TEXT NOT NULL,
                content TEXT NOT NULL,
                confidence REAL NOT NULL DEFAULT 0.8,
                ward_id TEXT NOT NULL DEFAULT '__global__',
                mention_count INTEGER NOT NULL DEFAULT 1,
                source_summary TEXT,
                embedding BLOB,
                contradicted_by TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                archived_at TEXT NOT NULL
            )",
            [],
        );

        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN archived INTEGER NOT NULL DEFAULT 0",
            [],
        );
    }

    // v13 → v14: Add pinned column to memory_facts
    if version < 14 {
        let _ = conn.execute(
            "ALTER TABLE memory_facts ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0",
            [],
        );
    }

    // v14 → v15: Add child_session_id to agent_executions for smart resume
    if version < 15 {
        let _ = conn.execute(
            "ALTER TABLE agent_executions ADD COLUMN child_session_id TEXT",
            [],
        );
    }

    // v15 → v16: Add artifacts table for agent-generated file tracking
    if version < 16 {
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS artifacts (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                ward_id TEXT,
                execution_id TEXT,
                agent_id TEXT,
                file_path TEXT NOT NULL,
                file_name TEXT NOT NULL,
                file_type TEXT,
                file_size INTEGER,
                label TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            )",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_artifacts_session ON artifacts(session_id)",
            [],
        );
    }

    // v16 → v17: Add mode column to sessions for persistent execution mode
    if version < 17 {
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN mode TEXT", []);
    }

    // v17 → v18: Add temporal columns to memory_facts; add kg_causal_edges table
    if version < 18 {
        let _ = conn.execute("ALTER TABLE memory_facts ADD COLUMN valid_from TEXT", []);
        let _ = conn.execute("ALTER TABLE memory_facts ADD COLUMN valid_until TEXT", []);
        let _ = conn.execute("ALTER TABLE memory_facts ADD COLUMN superseded_by TEXT", []);
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_facts_temporal ON memory_facts(valid_from, valid_until)",
            [],
        );

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS kg_causal_edges (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                cause_entity_id TEXT NOT NULL,
                effect_entity_id TEXT NOT NULL,
                relationship TEXT NOT NULL,
                confidence REAL DEFAULT 0.7,
                session_id TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY (cause_entity_id) REFERENCES kg_entities(id) ON DELETE CASCADE,
                FOREIGN KEY (effect_entity_id) REFERENCES kg_entities(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_causal_cause ON kg_causal_edges(cause_entity_id);
            CREATE INDEX IF NOT EXISTS idx_causal_effect ON kg_causal_edges(effect_entity_id);",
        )?;
    }

    // v18 → v19: Add ward_wiki_articles table for compiled wiki knowledge per ward
    if version < 19 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS ward_wiki_articles (
                id TEXT PRIMARY KEY,
                ward_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                tags TEXT,
                source_fact_ids TEXT,
                embedding BLOB,
                version INTEGER DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(ward_id, title)
            );
            CREATE INDEX IF NOT EXISTS idx_wiki_ward ON ward_wiki_articles(ward_id);",
        )?;
    }

    // v19 → v20: Add procedures table for learned procedure patterns
    if version < 20 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS procedures (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                ward_id TEXT DEFAULT '__global__',
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                trigger_pattern TEXT,
                steps TEXT NOT NULL,
                parameters TEXT,
                success_count INTEGER DEFAULT 1,
                failure_count INTEGER DEFAULT 0,
                avg_duration_ms INTEGER,
                avg_token_cost INTEGER,
                last_used TEXT,
                embedding BLOB,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_procedures_agent ON procedures(agent_id);
            CREATE INDEX IF NOT EXISTS idx_procedures_ward ON procedures(ward_id);",
        )?;
    }

    // v20 → v21: Add kg_episodes table and provenance columns on memory_facts
    if version < 21 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS kg_episodes (
                id TEXT PRIMARY KEY,
                source_type TEXT NOT NULL,
                source_ref TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                session_id TEXT,
                agent_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE(content_hash, source_type)
            );
            CREATE INDEX IF NOT EXISTS idx_episodes_session ON kg_episodes(session_id);
            CREATE INDEX IF NOT EXISTS idx_episodes_source ON kg_episodes(source_type, source_ref);",
        )?;

        let _ = conn.execute(
            "ALTER TABLE memory_facts ADD COLUMN epistemic_class TEXT DEFAULT 'current'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE memory_facts ADD COLUMN source_episode_id TEXT",
            [],
        );
        let _ = conn.execute("ALTER TABLE memory_facts ADD COLUMN source_ref TEXT", []);
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_facts_class ON memory_facts(agent_id, epistemic_class)",
            [],
        );
    }

    // v21 → v22: knowledge tables moved to knowledge.db (see knowledge_schema.rs)
    if version < 22 {
        // No-op: memory_facts, memory_facts_fts, memory_facts_archive, session_episodes,
        // kg_episodes, ward_wiki_articles, procedures, and embedding_cache have been
        // relocated to knowledge.db. Any pre-existing rows in conversations.db are
        // orphaned and will be ignored; the repository layer routes to KnowledgeDatabase.
    }

    // v22 → v23: Mark user-facing goal artifacts explicitly. Existing rows
    // remain hidden from Quick Chat until an agent opts in on a new declaration.
    if version < 23 {
        let has_artifacts_table: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'artifacts'",
            [],
            |row| row.get::<_, i64>(0),
        )? > 0;
        if has_artifacts_table {
            let mut columns = conn.prepare("PRAGMA table_info(artifacts)")?;
            let has_goal_column = columns
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>>>()?
                .iter()
                .any(|column| column == "is_goal_artifact");
            if !has_goal_column {
                conn.execute(
                    "ALTER TABLE artifacts ADD COLUMN is_goal_artifact INTEGER NOT NULL DEFAULT 0",
                    [],
                )?;
            }
        }
    }

    // v23 → v24: persist the selected-session current-plan snapshot and its
    // durable per-session acceptance counter.
    if version < 24 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS session_plan_counters (
                session_id TEXT PRIMARY KEY,
                last_issued_sequence INTEGER NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS session_plans (
                session_id TEXT PRIMARY KEY,
                execution_id TEXT NOT NULL,
                plan_json TEXT NOT NULL,
                explanation TEXT,
                source_event_timestamp INTEGER NOT NULL,
                source_event_sequence INTEGER NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                FOREIGN KEY (execution_id) REFERENCES agent_executions(id) ON DELETE CASCADE
            );",
        )?;
    }

    // v24 → v25: persist a bounded set of validated work surfaces per session.
    if version < 25 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS session_surfaces (
                session_id TEXT NOT NULL,
                surface_id TEXT NOT NULL,
                execution_id TEXT NOT NULL,
                surface_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (session_id, surface_id),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                FOREIGN KEY (execution_id) REFERENCES agent_executions(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_session_surfaces_updated
                ON session_surfaces(session_id, updated_at DESC);",
        )?;
    }

    // v25 → v26: add durable executable-work storage. No current execution
    // path consumes this table; the migration is additive and idempotent.
    if version < 26 {
        create_durable_work_schema(conn)?;
    }

    // v26 → v27: add an explicit canceled terminal state and peer-safe scoped
    // query indexes. Rebuild is required because SQLite cannot alter CHECK.
    if version < 27 {
        migrate_durable_work_to_v27(conn)?;
    }

    Ok(())
}

/// Initialize the database with all tables
pub fn initialize_database(conn: &Connection) -> Result<()> {
    // Run migrations for existing databases before creating tables
    migrate_database(conn)?;

    // Enable foreign keys
    conn.execute("PRAGMA foreign_keys = ON", [])?;

    // =========================================================================
    // SESSIONS
    // Top-level container for a user's work session
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            status TEXT NOT NULL DEFAULT 'running',
            source TEXT NOT NULL DEFAULT 'web',
            root_agent_id TEXT NOT NULL,
            title TEXT,
            created_at TEXT NOT NULL,
            started_at TEXT,
            completed_at TEXT,
            total_tokens_in INTEGER DEFAULT 0,
            total_tokens_out INTEGER DEFAULT 0,
            metadata TEXT,
            pending_delegations INTEGER DEFAULT 0,
            continuation_needed INTEGER DEFAULT 0,
            ward_id TEXT,
            parent_session_id TEXT,
            thread_id TEXT,
            connector_id TEXT,
            respond_to TEXT,
            archived INTEGER NOT NULL DEFAULT 0,
            mode TEXT
        )",
        [],
    )?;

    create_durable_work_schema(conn)?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_created ON sessions(created_at)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_root_agent ON sessions(root_agent_id)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_source ON sessions(source)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id)",
        [],
    )?;

    // =========================================================================
    // AGENT EXECUTIONS
    // An agent's participation in a session (root or delegated subagent)
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS agent_executions (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            parent_execution_id TEXT,
            delegation_type TEXT NOT NULL DEFAULT 'root',
            task TEXT,
            status TEXT NOT NULL DEFAULT 'queued',
            started_at TEXT,
            completed_at TEXT,
            tokens_in INTEGER DEFAULT 0,
            tokens_out INTEGER DEFAULT 0,
            checkpoint TEXT,
            error TEXT,
            log_path TEXT,
            child_session_id TEXT,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (parent_execution_id) REFERENCES agent_executions(id) ON DELETE SET NULL,
            FOREIGN KEY (child_session_id) REFERENCES sessions(id) ON DELETE SET NULL
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_executions_session ON agent_executions(session_id)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_executions_parent ON agent_executions(parent_execution_id)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_executions_status ON agent_executions(status)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_executions_agent ON agent_executions(agent_id)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_executions_started ON agent_executions(started_at)",
        [],
    )?;

    // =========================================================================
    // CURRENT SESSION PLANS
    // Latest validated operational plan for one selected Mission Control session
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS session_plan_counters (
            session_id TEXT PRIMARY KEY,
            last_issued_sequence INTEGER NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS session_plans (
            session_id TEXT PRIMARY KEY,
            execution_id TEXT NOT NULL,
            plan_json TEXT NOT NULL,
            explanation TEXT,
            source_event_timestamp INTEGER NOT NULL,
            source_event_sequence INTEGER NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (execution_id) REFERENCES agent_executions(id) ON DELETE CASCADE
        )",
        [],
    )?;

    // =========================================================================
    // PERSISTED WORK SURFACES
    // Latest validated display-only descriptors for one session
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS session_surfaces (
            session_id TEXT NOT NULL,
            surface_id TEXT NOT NULL,
            execution_id TEXT NOT NULL,
            surface_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (session_id, surface_id),
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (execution_id) REFERENCES agent_executions(id) ON DELETE CASCADE
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_session_surfaces_updated
         ON session_surfaces(session_id, updated_at DESC)",
        [],
    )?;

    // =========================================================================
    // MESSAGES
    // Individual messages in an agent's conversation
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS messages (
            id TEXT PRIMARY KEY,
            execution_id TEXT,
            session_id TEXT,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at TEXT NOT NULL,
            token_count INTEGER DEFAULT 0,
            tool_calls TEXT,
            tool_results TEXT,
            tool_call_id TEXT,
            FOREIGN KEY (execution_id) REFERENCES agent_executions(id) ON DELETE CASCADE,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_messages_execution ON messages(execution_id)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_messages_created ON messages(created_at)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_messages_session_created ON messages(session_id, created_at)",
        [],
    )?;

    // =========================================================================
    // EXECUTION LOGS
    // Detailed logs for debugging and tracing agent execution
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS execution_logs (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            conversation_id TEXT,
            agent_id TEXT NOT NULL,
            parent_session_id TEXT,
            timestamp TEXT NOT NULL,
            level TEXT NOT NULL,
            category TEXT NOT NULL,
            message TEXT NOT NULL,
            metadata TEXT,
            duration_ms INTEGER
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_logs_session ON execution_logs(session_id)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_logs_timestamp ON execution_logs(timestamp)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_logs_agent ON execution_logs(agent_id)",
        [],
    )?;

    // =========================================================================
    // BRIDGE OUTBOX
    // Reliable delivery queue for outbound messages to bridge workers
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS bridge_outbox (
            id TEXT PRIMARY KEY,
            adapter_id TEXT NOT NULL,
            capability TEXT NOT NULL,
            payload TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            session_id TEXT,
            thread_id TEXT,
            agent_id TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            sent_at TEXT,
            error TEXT,
            retry_count INTEGER NOT NULL DEFAULT 0,
            retry_after TEXT
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_outbox_adapter_status ON bridge_outbox(adapter_id, status)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_outbox_created ON bridge_outbox(created_at)",
        [],
    )?;

    // =========================================================================
    // DISTILLATION RUNS
    // Tracks distillation health per session
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS distillation_runs (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL UNIQUE,
            status TEXT NOT NULL,
            facts_extracted INTEGER DEFAULT 0,
            entities_extracted INTEGER DEFAULT 0,
            relationships_extracted INTEGER DEFAULT 0,
            episode_created INTEGER DEFAULT 0,
            error TEXT,
            retry_count INTEGER DEFAULT 0,
            duration_ms INTEGER,
            created_at TEXT NOT NULL
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_distillation_runs_status ON distillation_runs(status)",
        [],
    )?;

    // =========================================================================
    // RECALL LOG
    // Tracks which facts were recalled per session for predictive recall
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS recall_log (
            session_id TEXT NOT NULL,
            fact_key TEXT NOT NULL,
            recalled_at TEXT NOT NULL,
            PRIMARY KEY (session_id, fact_key)
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_recall_log_session ON recall_log(session_id)",
        [],
    )?;

    // =========================================================================
    // ARTIFACTS
    // File artifacts produced by agent executions
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS artifacts (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            ward_id TEXT,
            execution_id TEXT,
            agent_id TEXT,
            file_path TEXT NOT NULL,
            file_name TEXT NOT NULL,
            file_type TEXT,
            file_size INTEGER,
            label TEXT,
            is_goal_artifact INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_artifacts_session ON artifacts(session_id)",
        [],
    )?;

    // kg_causal_edges moved to knowledge.db (see knowledge_schema.rs) in v22

    // =========================================================================
    // SCHEMA VERSION
    // =========================================================================
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY
        )",
        [],
    )?;

    // `version` is the primary key, so INSERT OR REPLACE would retain the old
    // version as a second row. Keep this single-row marker canonical.
    conn.execute("DELETE FROM schema_version", [])?;
    conn.execute(
        "INSERT INTO schema_version (version) VALUES (?1)",
        [SCHEMA_VERSION],
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// Helper: create a fresh in-memory database and initialize it.
    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        initialize_database(&conn).expect("initialize_database");
        conn
    }

    #[test]
    fn test_migration_creates_distillation_runs_table() {
        let conn = setup_db();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='distillation_runs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "distillation_runs table should exist");
    }

    #[test]
    fn test_schema_version_is_current() {
        let conn = setup_db();
        let version: i32 = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, 27, "schema version should be 27");
    }

    // STUB: AC-schema — v25 data survives the additive durable-work migration.
    #[test]
    fn v26_durable_work_migration_preserves_v25_data() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO sessions (id, root_agent_id, created_at) VALUES ('sess-v25', 'root', '2026-08-04T00:00:00Z')",
            [],
        )
        .expect("seed v25 session");
        conn.execute_batch(
            "DROP TABLE IF EXISTS durable_work_items;
             DELETE FROM schema_version;
             INSERT INTO schema_version (version) VALUES (25);",
        )
        .expect("seed v25 schema");

        initialize_database(&conn).expect("migrate v25 database");
        initialize_database(&conn).expect("rerun v26 initialization");

        let version: i32 = conn
            .query_row("SELECT version FROM schema_version", [], |row| row.get(0))
            .expect("schema version");
        let session_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = 'sess-v25'",
                [],
                |row| row.get(0),
            )
            .expect("preserved session");
        let work_table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'durable_work_items'",
                [],
                |row| row.get(0),
            )
            .expect("durable work table");
        let work_index_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'index' AND name IN (
                    'uq_durable_work_source_dedupe',
                    'idx_durable_work_claim',
                    'idx_durable_work_expired_lease',
                    'idx_durable_work_scope_page',
                    'idx_durable_work_scope_correlation'
                 )",
                [],
                |row| row.get(0),
            )
            .expect("durable work indexes");

        assert_eq!(version, 27);
        assert_eq!(session_count, 1);
        assert_eq!(work_table_count, 1);
        assert_eq!(work_index_count, 5);
    }

    // STUB: AC11 — v26 work survives the CHECK-table rebuild and gains canceled.
    #[test]
    fn v27_durable_work_migration_preserves_rows_and_adds_canceled() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO durable_work_items (
                id, envelope_version, kind, source, target, payload_json,
                provenance_node_id, provenance_actor_id,
                provenance_session_id, provenance_execution_id,
                correlation_id, priority, status, attempts, max_attempts,
                available_at, created_at, updated_at
             ) VALUES (
                'work-v26', 1, 'test.scoped', 'a2a', 'worker.local', '{}',
                'node-local', 'a2a:peer-a', 'sess-v26', 'exec-v26',
                'task-v26', 0, 'pending', 0, 5,
                '2026-08-04T00:00:00.000000000Z',
                '2026-08-04T00:00:00.000000000Z',
                '2026-08-04T00:00:00.000000000Z'
             )",
            [],
        )
        .expect("seed current work row");
        let current_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master
                 WHERE type = 'table' AND name = 'durable_work_items'",
                [],
                |row| row.get(0),
            )
            .expect("current durable work SQL");
        let v26_sql = current_sql.replace(", 'canceled'", "");
        conn.execute_batch(
            "DROP INDEX uq_durable_work_source_dedupe;
             DROP INDEX idx_durable_work_claim;
             DROP INDEX idx_durable_work_expired_lease;
             DROP INDEX idx_durable_work_scope_page;
             DROP INDEX idx_durable_work_scope_correlation;
             ALTER TABLE durable_work_items RENAME TO durable_work_items_v27;",
        )
        .expect("move current work table");
        conn.execute_batch(&v26_sql).expect("create v26 work table");
        conn.execute_batch(
            "INSERT INTO durable_work_items SELECT * FROM durable_work_items_v27;
             DROP TABLE durable_work_items_v27;
             DELETE FROM schema_version;
             INSERT INTO schema_version (version) VALUES (26);",
        )
        .expect("finish v26 fixture");

        initialize_database(&conn).expect("migrate v26 database");

        let preserved: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM durable_work_items WHERE id = 'work-v26'",
                [],
                |row| row.get(0),
            )
            .expect("preserved work");
        conn.execute(
            "UPDATE durable_work_items SET status = 'canceled' WHERE id = 'work-v26'",
            [],
        )
        .expect("canceled state accepted");
        let scope_indexes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'index' AND name IN (
                    'idx_durable_work_scope_page',
                    'idx_durable_work_scope_correlation'
                 )",
                [],
                |row| row.get(0),
            )
            .expect("scope indexes");
        assert_eq!(preserved, 1);
        assert_eq!(scope_indexes, 2);
    }

    #[test]
    fn session_surfaces_fresh_schema_has_composite_key_cascade_and_v25() {
        // STUB: AC1 — fresh current schema contains the bounded session-owned table.
        let conn = setup_db();
        let table_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'session_surfaces'",
                [],
                |row| row.get(0),
            )
            .expect("session_surfaces table");
        assert!(table_sql.contains("PRIMARY KEY (session_id, surface_id)"));
        assert!(table_sql.contains("ON DELETE CASCADE"));
    }

    #[test]
    fn migrates_v23_to_current_plan_tables() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        initialize_database(&conn).expect("create current schema for v23 fixture");
        conn.execute_batch(
            "DROP TABLE session_plans;
             DROP TABLE session_plan_counters;
             DELETE FROM schema_version;
             INSERT INTO schema_version (version) VALUES (23);",
        )
        .expect("seed v23 schema");

        initialize_database(&conn).expect("migrate v23 database");

        for table in ["session_plan_counters", "session_plans"] {
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .expect("read migrated table");
            assert_eq!(exists, 1, "{table} should exist after migration");
        }
    }

    #[test]
    fn session_surfaces_schema_migrates_from_v24_without_data_loss() {
        // STUB: AC1 — v24 session data survives the additive v25 migration.
        let conn = setup_db();
        conn.execute(
            "INSERT INTO sessions (id, root_agent_id, created_at) VALUES ('sess-keep', 'root', '2026-07-28T00:00:00Z')",
            [],
        )
        .expect("seed session");
        conn.execute_batch(
            "DROP TABLE session_surfaces;
             DELETE FROM schema_version;
             INSERT INTO schema_version (version) VALUES (24);",
        )
        .expect("seed v24 schema");

        initialize_database(&conn).expect("migrate v24 database");

        let session_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = 'sess-keep'",
                [],
                |row| row.get(0),
            )
            .expect("read preserved session");
        let surface_table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'session_surfaces'",
                [],
                |row| row.get(0),
            )
            .expect("read migrated table");
        assert_eq!(session_count, 1);
        assert_eq!(surface_table_count, 1);
    }

    #[test]
    fn migrates_v22_artifacts_to_hidden_goal_default() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY);
             INSERT INTO schema_version (version) VALUES (22);
             CREATE TABLE artifacts (
                 id TEXT PRIMARY KEY,
                 session_id TEXT NOT NULL,
                 ward_id TEXT,
                 execution_id TEXT,
                 agent_id TEXT,
                 file_path TEXT NOT NULL,
                 file_name TEXT NOT NULL,
                 file_type TEXT,
                 file_size INTEGER,
                 label TEXT,
                 created_at TEXT NOT NULL
             );
             INSERT INTO artifacts (id, session_id, file_path, file_name, created_at)
             VALUES ('art-old', 'sess-old', '/ward/old.md', 'old.md', '2026-01-01T00:00:00Z');",
        )
        .expect("seed v22 schema");

        initialize_database(&conn).expect("migrate v22 database");

        let goal_flag: i64 = conn
            .query_row(
                "SELECT is_goal_artifact FROM artifacts WHERE id = 'art-old'",
                [],
                |row| row.get(0),
            )
            .expect("read migrated goal flag");
        assert_eq!(goal_flag, 0);
    }

    #[test]
    fn test_recall_log_table_exists() {
        let conn = setup_db();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='recall_log'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "recall_log table should exist");

        // Verify we can insert and query
        conn.execute(
            "INSERT INTO recall_log (session_id, fact_key, recalled_at) VALUES ('s1', 'k1', datetime('now'))",
            [],
        )
        .expect("insert into recall_log");

        let fact_key: String = conn
            .query_row(
                "SELECT fact_key FROM recall_log WHERE session_id = 's1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fact_key, "k1");
    }

    #[test]
    fn test_sessions_has_archived_column() {
        let conn = setup_db();
        // Insert a session and verify archived defaults to 0
        conn.execute(
            "INSERT INTO sessions (id, root_agent_id, created_at) VALUES ('s1', 'agent1', datetime('now'))",
            [],
        )
        .expect("insert session");

        let archived: i32 = conn
            .query_row("SELECT archived FROM sessions WHERE id = 's1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(archived, 0, "default archived should be 0");

        // Update to archived
        conn.execute("UPDATE sessions SET archived = 1 WHERE id = 's1'", [])
            .expect("update archived");

        let archived: i32 = conn
            .query_row("SELECT archived FROM sessions WHERE id = 's1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(archived, 1, "archived should be 1 after update");
    }
}
