//! Resource index — the write path of the capability catalog.
//!
//! Skills, agents, wards and MCP metadata are embedded into the fact store
//! so the intent router (read path, `intent/`) can retrieve candidates
//! semantically. Indexing is idempotent and delta-aware; it runs at session
//! bootstrap before classification.

use gateway_services::{
    AgentService, McpService, SharedVaultPaths, SkillFileInfo, SkillService, SkillSource,
};
use zbot_stores::{MemoryFactStore, SkillIndexRow};

/// Embedding-content schema version. Bump when `SkillFileInfo.indexed_content`
/// changes shape — the diff treats any row whose stored version is lower as
/// "modified" so a one-time re-embed pass picks up the new content.
const CURRENT_INDEX_FORMAT_VERSION: i64 = 2;

/// Count agents + wards on disk for the (still count-based) staleness
/// check on those resources. Skills are tracked per-row by
/// `reindex_skills` and intentionally excluded from this counter.
async fn count_agent_and_ward_resources(
    agent_service: &AgentService,
    vault_paths: &SharedVaultPaths,
) -> usize {
    let agent_count = agent_service.list().await.map(|a| a.len()).unwrap_or(0);
    let ward_count = std::fs::read_dir(vault_paths.wards_dir())
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .count()
        })
        .unwrap_or(0);
    agent_count + ward_count
}

/// Diff on-disk skills against the per-skill staleness tracker and embed
/// only the deltas.
async fn reindex_skills(fact_store: &dyn MemoryFactStore, skill_service: &SkillService) {
    let on_disk = skill_service.list_for_index();
    let in_db = match fact_store.list_skill_index().await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("list_skill_index failed, treating DB as empty: {}", e);
            Vec::new()
        }
    };

    let now_unix = chrono::Utc::now().timestamp();
    let mut added = 0_usize;
    let mut modified = 0_usize;
    let mut unchanged = 0_usize;

    let mut db_map: std::collections::HashMap<String, SkillIndexRow> =
        std::collections::HashMap::new();
    for row in in_db {
        db_map.insert(row.name.clone(), row);
    }

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for info in &on_disk {
        seen.insert(info.id.clone());
        let stale = match db_map.get(&info.id) {
            None => {
                added += 1;
                true
            }
            Some(row) => {
                let version_stale = row.format_version < CURRENT_INDEX_FORMAT_VERSION;
                let content_stale =
                    row.mtime_unix < info.mtime_unix || row.size_bytes != info.size_bytes as i64;
                if version_stale || content_stale {
                    modified += 1;
                    true
                } else {
                    unchanged += 1;
                    false
                }
            }
        };
        if stale {
            upsert_skill(fact_store, info, now_unix).await;
        }
    }

    // Skills deleted from disk stay in the index (history is useful); the
    // search side intersects with the live catalog before surfacing.
    tracing::info!(added, modified, unchanged, "Skill index refreshed");
}

async fn upsert_skill(fact_store: &dyn MemoryFactStore, info: &SkillFileInfo, now_unix: i64) {
    let key = format!("skill:{}", info.id);
    if let Err(e) = fact_store
        .save_fact(
            "root",
            "skill",
            &key,
            &info.indexed_content,
            1.0,
            None,
            None,
        )
        .await
    {
        tracing::warn!("save_fact failed for skill {}: {}", info.id, e);
        // Bail without writing the index row — next session retries.
        return;
    }
    let row = SkillIndexRow {
        name: info.id.clone(),
        source_root: source_label(info.source).to_string(),
        file_path: info.file_path.to_string_lossy().to_string(),
        mtime_unix: info.mtime_unix,
        size_bytes: info.size_bytes as i64,
        last_indexed_unix: now_unix,
        format_version: CURRENT_INDEX_FORMAT_VERSION,
    };
    if let Err(e) = fact_store.upsert_skill_index(row).await {
        tracing::warn!("upsert_skill_index failed for {}: {}", info.id, e);
    }
}

/// Stable string label for `SkillSource`, persisted in `source_root`.
fn source_label(source: SkillSource) -> &'static str {
    match source {
        SkillSource::Vault => "vault",
        SkillSource::Agent => "agent",
    }
}

/// Index skills, agents, wards, and MCP metadata into the fact store.
/// Idempotent — safe to call every session. Skills embed only deltas;
/// agents and wards use count-based staleness; MCPs always refresh.
pub async fn index_resources(
    fact_store: &dyn MemoryFactStore,
    skill_service: &SkillService,
    agent_service: &AgentService,
    mcp_service: &McpService,
    vault_paths: &SharedVaultPaths,
) {
    reindex_skills(fact_store, skill_service).await;
    index_mcps(fact_store, mcp_service).await;

    let aw_count = count_agent_and_ward_resources(agent_service, vault_paths).await;
    let temp_dir = vault_paths.vault_dir().join("temp");
    let index_marker = temp_dir.join(".aw_index_count");
    let last_count: usize = std::fs::read_to_string(&index_marker)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    if last_count == aw_count && aw_count > 0 {
        tracing::info!(
            aw_count = aw_count,
            last_indexed = last_count,
            "Agent/ward index up-to-date, skipping re-index"
        );
        return;
    }
    tracing::info!(
        aw_count = aw_count,
        last_indexed = last_count,
        "Agent/ward index stale, re-indexing"
    );

    // Index agents
    match agent_service.list().await {
        Ok(agents) => {
            tracing::info!(count = agents.len(), "Indexing agents into memory");
            for agent in &agents {
                let key = format!("agent:{}", agent.id);
                let content = format!("{} | {}", agent.id, agent.description);
                if let Err(e) = fact_store
                    .save_fact("root", "agent", &key, &content, 1.0, None, None)
                    .await
                {
                    tracing::debug!("Failed to index agent {}: {}", agent.id, e);
                }
            }
        }
        Err(e) => tracing::warn!("Failed to list agents for indexing: {}", e),
    }

    // Index wards (name + first prose line of AGENTS.md as purpose)
    let wards_dir = vault_paths.wards_dir();
    match std::fs::read_dir(&wards_dir) {
        Ok(entries) => {
            let ward_dirs: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .collect();
            tracing::info!(count = ward_dirs.len(), "Indexing wards into memory");
            for entry in &ward_dirs {
                let name = entry.file_name().to_string_lossy().to_string();
                let agents_md_path = entry.path().join("AGENTS.md");
                let purpose = if agents_md_path.exists() {
                    std::fs::read_to_string(&agents_md_path)
                        .ok()
                        .and_then(|content| {
                            content
                                .lines()
                                .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
                                .map(|l| l.trim().to_string())
                        })
                        .unwrap_or_default()
                } else {
                    String::new()
                };

                let key = format!("ward:{}", name);
                let content = if purpose.is_empty() {
                    name.clone()
                } else {
                    format!("{} | {}", name, purpose)
                };
                if let Err(e) = fact_store
                    .save_fact("root", "ward", &key, &content, 1.0, None, None)
                    .await
                {
                    tracing::debug!("Failed to index ward {}: {}", name, e);
                }
            }
        }
        Err(e) => tracing::warn!("Failed to read wards directory: {}", e),
    }

    let _ = std::fs::create_dir_all(&temp_dir);
    let _ = std::fs::write(&index_marker, aw_count.to_string());
}

const MAX_MCP_DESCRIPTION_CHARS: usize = 512;

/// Index the safe MCP metadata used by semantic intent retrieval.
/// Only ID, name, and description are stored — never commands, URLs,
/// headers, or credentials. Disabled/OAuth-blocked servers are excluded.
async fn index_mcps(fact_store: &dyn MemoryFactStore, mcp_service: &McpService) {
    match mcp_service.list_summaries() {
        Ok(summaries) => {
            let summaries = summaries
                .into_iter()
                .filter(|summary| {
                    summary.enabled
                        && matches!(
                            summary.auth_status.as_deref(),
                            None | Some("not_configured") | Some("connected")
                        )
                })
                .collect::<Vec<_>>();
            tracing::info!(count = summaries.len(), "Indexing MCPs into memory");
            for summary in summaries {
                let key = format!("mcp:{}", summary.id);
                let description = summary
                    .description
                    .chars()
                    .take(MAX_MCP_DESCRIPTION_CHARS)
                    .map(|character| {
                        if character.is_control() {
                            ' '
                        } else {
                            character
                        }
                    })
                    .collect::<String>();
                let content = format!("{} | {} | {}", summary.id, summary.name, description);
                if let Err(e) = fact_store
                    .save_fact("root", "mcp", &key, &content, 1.0, None, None)
                    .await
                {
                    tracing::debug!("Failed to index MCP {}: {}", summary.id, e);
                }
            }
        }
        Err(e) => tracing::warn!("Failed to list MCPs for indexing: {}", e),
    }
}

// ---------------------------------------------------------------------------
// Read path — semantic candidate retrieval
// ---------------------------------------------------------------------------
