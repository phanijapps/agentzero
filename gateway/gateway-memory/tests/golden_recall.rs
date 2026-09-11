//! Golden recall evaluation set — Phase P0 of the recall-quality migration
//! (`docs/specs/recall-quality/analysis.md`).
//!
//! This is the safety net that proves phases P1–P4 (engram retrieval
//! composition, weighted fusion, MMR, reinforcement) do not regress recall
//! quality. It runs the REAL `MemoryRecall::recall_unified` pipeline against
//! a deterministic seeded corpus and asserts labeled expectations.
//!
//! Run the full scorecard with:
//!
//! ```text
//! cargo test -p gateway-memory --test golden_recall -- --ignored --nocapture
//! ```
//!
//! The smoke test (fixture parse + corpus seed + one recall call) runs in
//! the normal suite. `cases.json` holds the 30 labeled cases;
//! `BASELINE.md` holds the pre-migration scorecard that future phases must
//! not regress.
//!
//! Determinism rules:
//! - Fixed embeddings: a token-hash embedder (FNV-1a → 384 dims, L2-normalized)
//!   shared by the recall query path and every seeded row. No network.
//! - Timestamps seeded as fixed offsets from seed time (`days_ago`); recall's
//!   internal `Utc::now()` calls only interact with these via wide margins
//!   (episode 14-day window gets 1/5-day-old episodes; bi-temporal filters
//!   see facts with no valid_from/valid_until bounds).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agent_runtime::llm::embedding::{EmbeddingClient, EmbeddingError};
use async_trait::async_trait;
use gateway_memory::{ItemKind, MemoryRecall, RecallConfig, ScoredItem};
use knowledge_graph::kg_trait::KnowledgeGraphStore as _KgStoreTrait;
use serde::Deserialize;
use zbot_engram_adapter::{
    AdapterConfig, EngramBeliefStore, EngramKnowledgeGraphStore, EngramMemoryFactStore,
    EngramProvider, EngramSidecarStores, EngramWikiStore,
};
use zbot_stores_domain::{Belief, MemoryFact, Procedure, SessionEpisode, WikiArticle};

use zbot_stores_traits::{BeliefStore, EpisodeStore, MemoryFactStore, ProcedureStore, StoreError};

// ============================================================================
// Deterministic embedder — 384 dims to match the vec0 DDL.
// ============================================================================

const EMBED_DIM: usize = 384;

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 3)
        .map(str::to_string)
        .collect()
}

fn embed_text(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0_f32; EMBED_DIM];
    for token in tokenize(text) {
        vector[(fnv1a(token.as_bytes()) % EMBED_DIM as u64) as usize] += 1.0;
    }
    // L2 normalize — cosine over shared tokens.
    let norm: f32 = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for v in &mut vector {
            *v /= norm;
        }
    }
    vector
}

/// Embedding client wrapper so the recall service and the store share the
/// same deterministic embedding function.
struct HashEmbedder;

#[async_trait]
impl EmbeddingClient for HashEmbedder {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        Ok(texts.iter().map(|t| embed_text(t)).collect())
    }
    fn dimensions(&self) -> usize {
        EMBED_DIM
    }
    fn model_name(&self) -> String {
        "golden-hash-384".to_string()
    }
    fn provider_type(&self) -> String {
        "golden-hash".to_string()
    }
}

// ============================================================================
// Corpus — deterministic seed data. IDs are stable so cases can name keys.
// ============================================================================

const AGENT: &str = "agent-a";
const WARD_FINANCE: &str = "finance";
const WARD_HR: &str = "hr";
const WARD_JOURNAL: &str = "journal";

fn days_ago(days: i64) -> String {
    (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339()
}

struct Corpus {
    recall: MemoryRecall,
    _tmp: tempfile::TempDir,
}

/// Fact used for expectations keyed by `fact.key`.
#[allow(clippy::too_many_arguments)]
fn fact(
    id: &str,
    category: &str,
    key: &str,
    content: &str,
    ward: &str,
    mention: i32,
    age_days: i64,
    pinned: bool,
    scope: &str,
) -> MemoryFact {
    MemoryFact {
        id: id.to_string(),
        session_id: None,
        agent_id: AGENT.to_string(),
        scope: scope.to_string(),
        category: category.to_string(),
        key: key.to_string(),
        content: content.to_string(),
        confidence: 0.9,
        mention_count: mention,
        source_summary: None,
        embedding: None,
        ward_id: ward.to_string(),
        contradicted_by: None,
        created_at: days_ago(age_days + 1),
        updated_at: days_ago(age_days),
        expires_at: None,
        valid_from: None,
        valid_until: None,
        superseded_by: None,
        pinned,
        epistemic_class: Some("current".to_string()),
        source_episode_id: None,
        source_ref: None,
        last_accessed: None,
        importance: None,
    }
}

async fn setup_corpus() -> Result<Corpus, String> {
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;

    // --- the FULL production (engram) path --------------------------------
    // E1-a: every recall source is wired exactly the way the daemon wires
    // it — one EngramProvider + adapter stores sharing this tempdir and
    // the deterministic embedder. No sqlite store participates.
    let embedder: Arc<dyn EmbeddingClient> = Arc::new(HashEmbedder);
    let engram_root = tmp.path().join("engram");
    std::fs::create_dir_all(&engram_root).map_err(|e| e.to_string())?;
    let mut adapter_config = AdapterConfig::engram_for_data_root(&engram_root, "engram.db");
    adapter_config.embedding_provider.provider_type = "golden-hash".to_string();
    adapter_config.embedding_provider.model = "golden-hash-384".to_string();
    adapter_config.embedding_provider.dimensions = EMBED_DIM as u32;
    let provider = EngramProvider::open(adapter_config.clone()).map_err(|e| e.to_string())?;
    let memory_store: Arc<dyn MemoryFactStore> = Arc::new(
        EngramMemoryFactStore::from_provider_with_embedding_client(
            adapter_config.clone(),
            &provider,
            Some(embedder.clone()),
        )
        .map_err(|e| e.to_string())?,
    );

    let global = "__global__";
    let facts: Vec<MemoryFact> = vec![
        // Corrections — the highest-value class.
        fact("f01", "correction", "policy.research_first",
             "Never rely on LLM training data for factual content. Always delegate to research-agent to pull real data from the web before building data-driven outputs.",
             global, 9, 7, false, "agent"),
        fact("f02", "correction", "policy.web_research_tools",
             "Use duckduckgo-search skill for web research. Never use raw shell curl or wget for web scraping.",
             global, 3, 1, false, "agent"),
        fact("f03", "correction", "policy.atomic_delegation",
             "Each delegation task must produce ONE output file. One topic = one step.",
             global, 1, 14, false, "agent"),
        fact("f04", "correction", "policy.ward_first",
             "Enter the ward before starting work; the ward snapshot preamble is ground truth.",
             global, 5, 2, false, "agent"),
        fact("f22", "correction", "policy.citations",
             "Cite sources for every numeric claim in research outputs.",
             global, 2, 3, false, "agent"),
        // User facts.
        fact("f05", "user", "user.name", "The user's name is Alex.", global, 1, 90, true, "agent"),
        fact("f06", "user", "user.location.home_base", "The user lives in Seattle.", global, 1, 30, false, "agent"),
        fact("f07", "user", "user.preferred_format", "User prefers comparison tables over prose for numeric data.", global, 2, 21, false, "agent"),
        // Domain — stale/fresh pair for supersession.
        fact("f08", "domain", "aapl.valuation_verdict.v1", "AAPL is overvalued at 32x forward earnings.", global, 1, 30, false, "agent"),
        fact("f09", "domain", "aapl.valuation_verdict.v2", "AAPL is fairly valued; peer analysis puts fair value near the current price.", WARD_FINANCE, 3, 1, false, "agent"),
        fact("f10", "domain", "peers.tickers", "Peer set for AAPL: MSFT, GOOGL, AMZN.", global, 2, 7, false, "agent"),
        fact("f11", "domain", "msft.valuation_verdict", "MSFT trades at a premium to peers on cloud growth.", global, 1, 5, false, "agent"),
        fact("f21", "domain", "aapl.earnings_date", "AAPL reports earnings next Thursday.", WARD_FINANCE, 1, 1, false, "agent"),
        fact("f25", "domain", "aapl.revenue_mix", "AAPL revenue mix: iPhone 52 percent, services 24 percent, wearables 10 percent.", WARD_FINANCE, 1, 7, false, "agent"),
        // Patterns.
        fact("f12", "pattern", "pattern.financial_analysis_flow",
             "Session flow for financial analysis: recall ward context, fetch market data via research agent, build comparison table, publish.",
             WARD_FINANCE, 9, 14, false, "agent"),
        fact("f13", "pattern", "pattern.comparison_table_steps",
             "Comparison table recipe: define rows as tickers, columns as metrics, fetch metrics per ticker, render markdown table.",
             WARD_FINANCE, 3, 7, false, "agent"),
        fact("f14", "pattern", "pattern.visualization_guide",
             "Render charts with matplotlib after building the data table; prefer one chart per metric.",
             WARD_FINANCE, 1, 30, false, "agent"),
        fact("f15", "pattern", "pattern.research_publish",
             "Research flow: gather sources, summarize findings, publish to the ward wiki.",
             global, 2, 10, false, "agent"),
        // Schema.
        fact("f16", "schema", "schema.comparison_table",
             "Comparison table schema: rows are tickers, columns are pe_ratio, forward_pe, ev_ebitda, revenue_growth.",
             WARD_FINANCE, 2, 7, false, "agent"),
        // Skill / agent indices.
        fact("f17", "skill", "skill.yf-risk", "Skill yf-risk: portfolio construction and risk diagnostics from yfinance return series.", global, 1, 90, false, "agent"),
        fact("f18", "agent", "agent.builder-agent", "Agent builder-agent writes, reviews, and fixes software code.", global, 1, 90, false, "agent"),
        // Session-scoped ctx facts — excluded from fuzzy recall by design.
        fact("f19", "ctx", "ctx.session.intent", "Session intent context record.", global, 1, 0, false, "session"),
        fact("f20", "ctx", "state.exec-42", "Subagent handoff state record.", global, 1, 0, false, "session"),
        // Ward-scoped isolation probes.
        fact("f23", "user", "vacation.policy", "Vacation policy: 25 days paid time off.", WARD_HR, 1, 60, false, "agent"),
        fact("f24", "pattern", "pattern.journal_workflow", "Journal workflow: summarize day events, tag by project, update tracking table.", WARD_JOURNAL, 1, 20, false, "agent"),
    ];

    for f in &facts {
        let embedding = embed_text(&format!(
            "{} {} {}",
            f.category,
            f.key.replace('_', " "),
            f.content
        ));
        memory_store
            .upsert_typed_fact(f.clone(), Some(embedding))
            .await
            .map_err(|e: StoreError| format!("seed fact {}: {e}", f.id))?;
    }
    // Stale pair: v1 superseded by v2.
    memory_store
        .supersede_fact("f08", "f09", chrono::Utc::now())
        .await
        .map_err(|e| format!("supersede f08: {e}"))?;

    // --- procedures (engram sidecars, the production path) ----------------
    let procedure_store: Arc<dyn ProcedureStore> = Arc::new(
        EngramSidecarStores::from_provider(adapter_config.clone(), &provider)
            .map_err(|e| e.to_string())?,
    );
    let procedures = vec![
        Procedure {
            id: "p1".into(),
            agent_id: AGENT.into(),
            ward_id: Some(WARD_FINANCE.into()),
            name: "research_and_publish_comparison_table".into(),
            description: "Fetch market data per ticker via research agent, build a comparison table, publish findings.".into(),
            trigger_pattern: Some("comparison table of tickers".into()),
            steps: r#"["recall ward context","fetch market data per ticker via research agent","build comparison table","publish findings"]"#.into(),
            parameters: None,
            success_count: 12,
            failure_count: 1,
            avg_duration_ms: None,
            avg_token_cost: None,
            last_used: Some(days_ago(1)),
            embedding: None,
            created_at: days_ago(20),
            updated_at: days_ago(1),
        },
        Procedure {
            id: "p2".into(),
            agent_id: AGENT.into(),
            ward_id: Some(WARD_FINANCE.into()),
            name: "generate_comparison_datatable".into(),
            description: "Scrape financial websites with shell curl and parse html into a table.".into(),
            trigger_pattern: Some("scrape financial websites".into()),
            steps: r#"["scrape financial websites with curl","parse html into a table"]"#.into(),
            parameters: None,
            success_count: 0,
            failure_count: 5,
            avg_duration_ms: None,
            avg_token_cost: None,
            last_used: Some(days_ago(7)),
            embedding: None,
            created_at: days_ago(30),
            updated_at: days_ago(7),
        },
        Procedure {
            id: "p3".into(),
            agent_id: AGENT.into(),
            ward_id: Some(WARD_JOURNAL.into()),
            name: "journal_entry_update".into(),
            description: "Read journal events, write the daily summary, update the tracking table.".into(),
            trigger_pattern: None,
            steps: r#"["read journal events","write daily summary","update tracking table"]"#.into(),
            parameters: None,
            success_count: 2,
            failure_count: 2,
            avg_duration_ms: None,
            avg_token_cost: None,
            last_used: None,
            embedding: None,
            created_at: days_ago(40),
            updated_at: days_ago(10),
        },
    ];
    for p in procedures {
        let embedding = embed_text(&format!(
            "{} {} {}",
            p.name.replace('_', " "),
            p.description,
            p.trigger_pattern.clone().unwrap_or_default()
        ));
        procedure_store
            .upsert_procedure(p, Some(embedding))
            .await
            .map_err(|e| format!("seed procedure: {e}"))?;
    }

    // --- wiki -------------------------------------------------------------
    let wiki_store: Arc<dyn zbot_stores_traits::WikiStore> = Arc::new(
        EngramWikiStore::from_provider(adapter_config.clone(), &provider)
            .map_err(|e| e.to_string())?,
    );
    let articles = vec![
        WikiArticle {
            id: "w1".into(),
            ward_id: WARD_FINANCE.into(),
            agent_id: AGENT.into(),
            title: "How to build a comparison table".into(),
            content: "Define tickers as rows and metrics as columns; fetch each metric per ticker; render as a markdown table.".into(),
            tags: Some("howto,table".into()),
            source_fact_ids: None,
            embedding: None,
            version: 1,
            created_at: days_ago(10),
            updated_at: days_ago(1),
        },
        WikiArticle {
            id: "w2".into(),
            ward_id: WARD_FINANCE.into(),
            agent_id: AGENT.into(),
            title: "Valuation data sources".into(),
            content: "Use the research agent and yfinance for market data; never scrape with raw curl.".into(),
            tags: Some("valuation,data".into()),
            source_fact_ids: None,
            embedding: None,
            version: 1,
            created_at: days_ago(10),
            updated_at: days_ago(1),
        },
    ];
    for a in articles {
        let embedding = embed_text(&format!("{} {}", a.title, a.content));
        wiki_store
            .upsert_article(a, Some(embedding))
            .await
            .map_err(|e| format!("seed wiki: {e}"))?;
    }

    // --- episodes (chain + avoid-list) ------------------------------------
    let episode_store: Arc<dyn EpisodeStore> = Arc::new(
        EngramSidecarStores::from_provider(adapter_config.clone(), &provider)
            .map_err(|e| e.to_string())?,
    );
    let episodes = vec![
        SessionEpisode {
            id: "e1".into(),
            session_id: "sess-e1".into(),
            agent_id: AGENT.into(),
            ward_id: WARD_FINANCE.into(),
            task_summary:
                "Built AAPL peer valuation comparison table and published it to the ward.".into(),
            outcome: "success".into(),
            strategy_used: None,
            key_learnings: Some("Comparison table flow worked end to end.".into()),
            token_cost: None,
            embedding: None,
            created_at: days_ago(1),
        },
        SessionEpisode {
            id: "e2".into(),
            session_id: "sess-e2".into(),
            agent_id: AGENT.into(),
            ward_id: WARD_FINANCE.into(),
            task_summary: "Fetched peer tickers; the comparison table was left incomplete.".into(),
            outcome: "partial".into(),
            strategy_used: None,
            key_learnings: None,
            token_cost: None,
            embedding: None,
            created_at: days_ago(5),
        },
        SessionEpisode {
            id: "e3".into(),
            session_id: "sess-e3".into(),
            agent_id: AGENT.into(),
            ward_id: WARD_FINANCE.into(),
            task_summary: "Pulled AAPL quotes by scraping with raw curl; blocked by anti-bot."
                .into(),
            outcome: "failed".into(),
            strategy_used: None,
            key_learnings: Some(
                "Use research-agent for web data; raw curl scraping is blocked.".into(),
            ),
            token_cost: None,
            embedding: None,
            created_at: days_ago(1),
        },
    ];
    for e in &episodes {
        episode_store
            .insert_episode(e.clone(), None)
            .await
            .map_err(|err| format!("seed episode {}: {err}", e.id))?;
    }

    // --- beliefs ----------------------------------------------------------
    let belief_store = Arc::new(
        EngramBeliefStore::from_provider(adapter_config.clone(), &provider)
            .map_err(|e| e.to_string())?,
    );
    let beliefs = vec![
        Belief {
            id: "b1".into(),
            partition_id: AGENT.into(),
            subject: "domain.finance.aapl.valuation_verdict".into(),
            content: "AAPL is fairly valued per the latest peer analysis.".into(),
            confidence: 0.82,
            valid_from: None,
            valid_until: None,
            source_fact_ids: vec!["f09".into()],
            synthesizer_version: 1,
            reasoning: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            superseded_by: None,
            stale: false,
            embedding: Some(
                embed_text("AAPL is fairly valued per the latest peer analysis.")
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
            ),
        },
        Belief {
            id: "b2".into(),
            partition_id: AGENT.into(),
            subject: "domain.finance.data_path".into(),
            content: "research-agent-first is the effective path for market data.".into(),
            confidence: 0.75,
            valid_from: None,
            valid_until: None,
            source_fact_ids: vec!["f01".into()],
            synthesizer_version: 1,
            reasoning: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            superseded_by: None,
            stale: false,
            embedding: Some(
                embed_text("research-agent-first is the effective path for market data.")
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
            ),
        },
    ];
    for b in &beliefs {
        belief_store
            .upsert_belief(b)
            .await
            .map_err(|e| format!("seed belief: {e}"))?;
    }

    // --- knowledge graph (name-embedding lane) ----------------------------
    let kg_store = Arc::new(
        EngramKnowledgeGraphStore::from_provider(adapter_config.clone(), &provider)
            .map_err(|e| e.to_string())?,
    );
    use knowledge_graph::types::{Entity, EntityType, Relationship, RelationshipType};
    use std::collections::HashMap as JsonProps;
    let entities: Vec<(String, EntityType)> = vec![
        ("AAPL".into(), EntityType::Concept),
        ("Microsoft".into(), EntityType::Organization),
        ("research-agent".into(), EntityType::Tool),
        ("comparison-table".into(), EntityType::Artifact),
        ("valuation".into(), EntityType::Concept),
    ];
    let mut entity_ids: HashMap<String, String> = HashMap::new();
    for (name, entity_type) in entities {
        let entity = Entity {
            id: format!("kg-{name}"),
            agent_id: AGENT.into(),
            entity_type,
            name: name.clone(),
            properties: JsonProps::new(),
            first_seen_at: chrono::Utc::now(),
            last_seen_at: chrono::Utc::now(),
            mention_count: 1,
            name_embedding: Some(embed_text(&name)),
        };
        let id = kg_store
            .upsert_entity(AGENT, entity)
            .await
            .map_err(|e| format!("seed entity {name}: {e}"))?;
        entity_ids.insert(name, id.0);
    }
    let aapl = entity_ids.get("AAPL").ok_or("AAPL entity id")?.clone();
    let research_agent = entity_ids.get("research-agent").ok_or("entity id")?.clone();
    let comp_table = entity_ids
        .get("comparison-table")
        .ok_or("entity id")?
        .clone();
    let relations = vec![
        Relationship {
            id: "r1".into(),
            agent_id: AGENT.into(),
            source_entity_id: aapl.clone(),
            target_entity_id: research_agent.clone(),
            relationship_type: RelationshipType::Uses,
            properties: JsonProps::from([("confidence".to_string(), serde_json::json!(0.9))]),
            first_seen_at: chrono::Utc::now(),
            last_seen_at: chrono::Utc::now(),
            mention_count: 1,
        },
        Relationship {
            id: "r2".into(),
            agent_id: AGENT.into(),
            source_entity_id: research_agent.clone(),
            target_entity_id: comp_table.clone(),
            relationship_type: RelationshipType::Created,
            properties: JsonProps::from([("confidence".to_string(), serde_json::json!(0.8))]),
            first_seen_at: chrono::Utc::now(),
            last_seen_at: chrono::Utc::now(),
            mention_count: 1,
        },
    ];
    for r in relations {
        kg_store
            .upsert_relationship(AGENT, r)
            .await
            .map_err(|e| format!("seed relationship: {e}"))?;
    }

    // --- recall stack -----------------------------------------------------
    let mut recall = MemoryRecall::new(Some(embedder), Arc::new(RecallConfig::default()));
    // Deterministic cross-encoder stand-in: query/content token overlap.
    // Mirrors the engram adapter's own OverlapScorer — good enough to
    // measure the rerank stage's effect without an LLM in the harness.
    struct OverlapScorer;
    impl engram_rerank_cross_encoder::RerankScorer for OverlapScorer {
        fn score(&self, query: &str, candidate: &str) -> engram_runtime::CoreResult<f32> {
            use std::collections::HashSet;
            let query_lower = query.to_lowercase();
            let candidate_lower = candidate.to_lowercase();
            let q: HashSet<&str> = query_lower.split_whitespace().collect();
            let c: HashSet<&str> = candidate_lower.split_whitespace().collect();
            Ok((q.intersection(&c).count() as f32) / (q.len().max(1) as f32))
        }
    }
    recall.set_rerank_stage(gateway_memory::recall::rerank::RerankStage::new(
        std::sync::Arc::new(OverlapScorer),
        gateway_memory::recall::rerank::RerankConfig::default(),
    ));
    recall.set_memory_store(memory_store);
    recall.set_procedure_store(procedure_store);
    recall.set_wiki_store(wiki_store);
    recall.set_episode_store(episode_store);
    recall.set_belief_store(belief_store);
    recall.set_kg_store(kg_store);

    Ok(Corpus { recall, _tmp: tmp })
}

// ============================================================================
// Case fixtures + runner.
// ============================================================================

#[derive(Debug, Deserialize)]
struct CaseFile {
    #[allow(dead_code)]
    corpus_version: String,
    cases: Vec<GoldenCase>,
}

#[derive(Debug, Deserialize)]
struct GoldenCase {
    id: String,
    agent_id: String,
    ward_id: String,
    query: String,
    #[serde(default)]
    #[allow(dead_code)]
    description: String,
    #[serde(default)]
    tag: String,
    expected: Expectations,
}

#[derive(Debug, Default, Deserialize)]
struct Expectations {
    #[serde(default)]
    fact_keys_any: Vec<String>,
    #[serde(default)]
    fact_keys_none: Vec<String>,
    #[serde(default)]
    kinds_any: Vec<String>,
    #[serde(default)]
    must_be_nonempty: bool,
}

fn cases_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden_recall/cases.json")
}

fn load_cases() -> CaseFile {
    let raw = std::fs::read_to_string(cases_path()).expect("cases.json readable");
    serde_json::from_str(&raw).expect("cases.json parses")
}

fn kind_name(kind: &ItemKind) -> &'static str {
    match kind {
        ItemKind::Fact => "Fact",
        ItemKind::Wiki => "Wiki",
        ItemKind::Procedure => "Procedure",
        ItemKind::GraphNode => "GraphNode",
        ItemKind::Goal => "Goal",
        ItemKind::Episode => "Episode",
        ItemKind::Belief => "Belief",
        ItemKind::HierEntity => "HierEntity",
        ItemKind::HierRelation => "HierRelation",
    }
}

/// Fact items render as `[category] key: content` — recover the key.
fn fact_key_of(item: &ScoredItem) -> Option<&str> {
    if !matches!(item.kind, ItemKind::Fact) {
        return None;
    }
    let content = item.content.as_str();
    let after_bracket = content.split_once("] ").map(|(_, rest)| rest)?;
    after_bracket.split_once(": ").map(|(key, _)| key)
}

struct CaseReport {
    id: String,
    tag: String,
    passed: bool,
    failures: Vec<String>,
    top5_keys: Vec<String>,
    top5_kinds: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
fn evaluate(case: &GoldenCase, items: &[ScoredItem]) -> CaseReport {
    let mut failures = Vec::new();
    // Safety floor (asserted): presence in the returned packet — the agent
    // sees the item. The recall budget is the packet the model consumes.
    let pool_keys: Vec<&str> = items.iter().filter_map(|i| fact_key_of(i)).collect();
    let pool_kinds: Vec<&str> = items.iter().map(|i| kind_name(&i.kind)).collect();
    // Quality metric (reported, not asserted): precision within the top-5.
    let top: Vec<&ScoredItem> = items.iter().take(5).collect();
    let top_keys: Vec<&str> = top.iter().filter_map(|i| fact_key_of(i)).collect();
    let top_kinds: Vec<&str> = top.iter().map(|i| kind_name(&i.kind)).collect();

    if case.expected.must_be_nonempty && items.is_empty() {
        failures.push("expected non-empty results, got none".into());
    }
    if !case.expected.fact_keys_any.is_empty()
        && !case
            .expected
            .fact_keys_any
            .iter()
            .any(|k| pool_keys.contains(&k.as_str()))
    {
        failures.push(format!(
            "fact_keys_any {:?} not in packet keys {:?}",
            case.expected.fact_keys_any, pool_keys
        ));
    }
    for forbidden in &case.expected.fact_keys_none {
        if pool_keys.contains(&forbidden.as_str()) {
            failures.push(format!(
                "forbidden key {forbidden} surfaced in packet {pool_keys:?}"
            ));
        }
    }
    if !case.expected.kinds_any.is_empty()
        && !case
            .expected
            .kinds_any
            .iter()
            .any(|k| pool_kinds.contains(&k.as_str()))
    {
        failures.push(format!(
            "kinds_any {:?} not in packet kinds {:?}",
            case.expected.kinds_any, pool_kinds
        ));
    }

    CaseReport {
        id: case.id.clone(),
        tag: if case.tag.is_empty() {
            "recall".into()
        } else {
            case.tag.clone()
        },
        passed: failures.is_empty(),
        failures,
        top5_keys: top_keys.iter().map(|s| s.to_string()).collect(),
        top5_kinds: top_kinds.iter().map(|s| s.to_string()).collect(),
    }
}

// ============================================================================
// Tests.
// ============================================================================

/// Production-shaped packet budget for the precision metric.
const PROD_BUDGET: usize = 10;
/// Generous budget for the presence floor: with ten fused lanes, a budget-10
/// packet holds roughly one item per lane — presence there measures fusion
/// order, not retrieval. Budget 25 measures whether the pipeline finds the
/// item at all.
const FLOOR_BUDGET: usize = 25;

/// Smoke: fixtures parse, corpus seeds, and the recall stack answers without
/// error. Runs in the normal suite.
#[tokio::test]
async fn golden_recall_fixtures_parse_and_corpus_seeds() {
    let case_file = load_cases();
    assert_eq!(case_file.cases.len(), 30, "expected exactly 30 cases");
    for case in &case_file.cases {
        assert!(!case.query.is_empty(), "{}: empty query", case.id);
    }

    let corpus = setup_corpus().await.expect("corpus seeds");
    let items = corpus
        .recall
        .recall_unified(
            AGENT,
            "AAPL valuation comparison table",
            Some(WARD_FINANCE),
            &[],
            10,
        )
        .await
        .expect("recall runs");
    assert!(
        !items.is_empty(),
        "smoke query must return a non-empty pool"
    );
}

/// The golden scorecard. Run with:
/// `cargo test -p gateway-memory --test golden_recall -- --ignored --nocapture`
/// The golden scorecard. Run with:
/// `cargo test -p gateway-memory --test golden_recall -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn probe_c12_fact_lane() {
    use zbot_stores_traits::MemoryFactStore as _;
    let root = tempfile::tempdir().expect("root");
    let mut config = AdapterConfig::engram_for_data_root(root.path(), "engram.db");
    config.embedding_provider.provider_type = "golden-hash".to_string();
    config.embedding_provider.model = "golden-hash-384".to_string();
    config.embedding_provider.dimensions = EMBED_DIM as u32;
    let provider = EngramProvider::open(config.clone()).expect("provider");
    let store = EngramMemoryFactStore::from_provider_with_embedding_client(
        config,
        &provider,
        Some(Arc::new(HashEmbedder)),
    )
    .expect("store");

    let f = fact("f02", "correction", "policy.web_research_tools",
        "Use duckduckgo-search skill for web research. Never use raw shell curl or wget for web scraping.",
        "__global__", 3, 1, false, "agent");
    let embedding = embed_text(&format!(
        "{} {} {}",
        f.category,
        f.key.replace('_', " "),
        f.content
    ));
    store
        .upsert_typed_fact(f.clone(), Some(embedding))
        .await
        .expect("seed");

    let identity = zbot_stores_traits::EmbeddingQueryIdentity {
        provider_type: "golden-hash".into(),
        model: "golden-hash-384".into(),
        dimensions: EMBED_DIM as u32,
        prompt_profile: "query".into(),
        normalization: None,
    };
    for query in ["scrape financial websites for quotes", "web research tools"] {
        let hits = store
            .search_memory_facts_hybrid_with_identity(
                Some(AGENT),
                query,
                "hybrid",
                10,
                Some(WARD_FINANCE),
                Some(&embed_text(query)),
                Some(&identity),
                None,
            )
            .await
            .expect("search");
        eprintln!(
            "PROBE query={query:?} hits={}",
            serde_json::to_string(&hits).unwrap_or_default()
        );
    }
}

#[tokio::test]
#[ignore = "full-stack golden run: use --ignored --nocapture"]
async fn golden_recall_scorecard() {
    let case_file = load_cases();
    let corpus = setup_corpus().await.expect("corpus seeds");

    let mut reports: Vec<CaseReport> = Vec::new();
    for case in &case_file.cases {
        // Presence floor: generous budget — did retrieval find it at all?
        let floor_items = corpus
            .recall
            .recall_unified(
                &case.agent_id,
                &case.query,
                Some(&case.ward_id),
                &[],
                FLOOR_BUDGET,
            )
            .await
            .unwrap_or_else(|e| panic!("{}: recall error: {e}", case.id));
        let mut report = evaluate(case, &floor_items);
        // Precision: production-shaped budget, top-5 of the packet.
        let prod_items = corpus
            .recall
            .recall_unified(
                &case.agent_id,
                &case.query,
                Some(&case.ward_id),
                &[],
                PROD_BUDGET,
            )
            .await
            .unwrap_or_else(|e| panic!("{}: recall error: {e}", case.id));
        report.top5_keys = prod_items
            .iter()
            .take(5)
            .filter_map(|i| fact_key_of(i).map(str::to_string))
            .collect();
        report.top5_kinds = prod_items
            .iter()
            .take(5)
            .map(|i| kind_name(&i.kind).to_string())
            .collect();
        println!(
            "[{}] {} ({}) — floor {}/{} items, prod {} items",
            if report.passed { "PASS" } else { "FAIL" },
            report.id,
            report.tag,
            floor_items.len(),
            FLOOR_BUDGET,
            prod_items.len()
        );
        for failure in &report.failures {
            println!("        {failure}");
        }
        reports.push(report);
    }

    // ---- scorecard -------------------------------------------------------
    // Presence floor (asserted above) vs precision@5 (reported — the
    // improvement target for the engram-retrieval migration phases).
    let case_file_ref = &case_file;
    let mut precision_hits = 0usize;
    let mut correction_top5 = 0usize;
    let mut correction_total = 0usize;
    for report in &reports {
        let case = case_file_ref
            .cases
            .iter()
            .find(|c| c.id == report.id)
            .expect("report maps to case");
        let mut this_hit = true;
        if !case.expected.fact_keys_any.is_empty()
            && !case
                .expected
                .fact_keys_any
                .iter()
                .any(|k| report.top5_keys.contains(k))
        {
            this_hit = false;
        }
        if !case.expected.kinds_any.is_empty()
            && !case
                .expected
                .kinds_any
                .iter()
                .any(|k| report.top5_kinds.contains(k))
        {
            this_hit = false;
        }
        if this_hit {
            precision_hits += 1;
        }
        if case.tag == "correction" {
            correction_total += 1;
            if this_hit {
                correction_top5 += 1;
            }
        }
    }

    let total = reports.len();
    let passed = reports.iter().filter(|r| r.passed).count();
    let by_tag: HashMap<&str, Vec<&CaseReport>> = {
        let mut map: HashMap<&str, Vec<&CaseReport>> = HashMap::new();
        for report in &reports {
            map.entry(report.tag.as_str()).or_default().push(report);
        }
        map
    };
    let tag_stat = |tag: &str| -> (usize, usize) {
        by_tag
            .get(tag)
            .map(|rs| (rs.iter().filter(|r| r.passed).count(), rs.len()))
            .unwrap_or((0, 0))
    };
    let (corr_pass, corr_total) = tag_stat("correction");
    let (avoid_pass, avoid_total) = tag_stat("avoid");
    let (stale_pass, stale_total) = tag_stat("stale");
    let (pattern_pass, pattern_total) = tag_stat("pattern");

    println!();
    println!("================ GOLDEN RECALL SCORECARD ================");
    println!("presence floor (asserted): {passed}/{total} cases");
    println!(
        "precision@5 (improvement target): {}/{} = {:.1}%",
        precision_hits,
        total,
        precision_hits as f64 * 100.0 / total as f64
    );
    println!("correction top-5: {correction_top5}/{correction_total}");
    println!("tag floors: correction {corr_pass}/{corr_total}, avoid {avoid_pass}/{avoid_total}, stale {stale_pass}/{stale_total}, pattern {pattern_pass}/{pattern_total}");
    println!("=========================================================");

    let failed: Vec<&CaseReport> = reports.iter().filter(|r| !r.passed).collect();
    assert!(
        failed.is_empty(),
        "{} golden case(s) failed: {:?}",
        failed.len(),
        failed.iter().map(|r| r.id.clone()).collect::<Vec<_>>()
    );
}

// ============================================================================
// Identity-tolerant memory store wrapper.
//
// The `MemoryFactStore` trait default for
// `search_memory_facts_hybrid_with_identity` fails closed whenever a query
// embedding is present, and the sqlite backend does not override it — only
// the production engram adapter implements the identity gate. The golden
// harness delegates to the sqlite store and relaxes ONLY that gate
// (equivalent to the engram adapter's behavior when identities match).
// Seeding uses the same wrapper so the corpus is written through one type.
// ============================================================================
