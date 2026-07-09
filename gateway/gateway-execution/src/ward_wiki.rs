// ============================================================================
// WARD WIKI COMPILATION — Karpathy compiler pattern
// ============================================================================

//! After distillation extracts facts, this module compiles them into
//! structured wiki articles per ward. Articles accumulate and evolve
//! across sessions, creating a compiled knowledge base.

use agent_runtime::llm::client::LlmClient;
use agent_runtime::llm::embedding::EmbeddingClient;
use agent_runtime::types::ChatMessage;
use serde::Deserialize;
use zbot_stores_traits::wiki::WikiArticle;
use zbot_stores_traits::WikiStore;

/// Summary of a fact for the compilation prompt.
#[derive(Debug, Clone)]
pub struct FactSummary {
    pub category: String,
    pub key: String,
    pub content: String,
}

/// LLM compilation output.
#[derive(Debug, Deserialize)]
struct CompilationResponse {
    articles: Vec<ArticleData>,
}

/// A single article from the LLM response.
#[derive(Debug, Deserialize)]
struct ArticleData {
    title: String,
    content: String,
    tags: Option<Vec<String>>,
}

/// Compile ward wiki articles from newly extracted facts.
///
/// Called after distillation completes. Takes new facts and existing
/// articles, asks the LLM to produce updated/new articles.
///
/// Returns the number of articles upserted.
pub async fn compile_ward_wiki(
    ward_id: &str,
    agent_id: &str,
    new_facts: &[FactSummary],
    wiki_store: &dyn WikiStore,
    llm_client: &dyn LlmClient,
    embedding_client: Option<&dyn EmbeddingClient>,
) -> Result<usize, String> {
    if new_facts.is_empty() {
        tracing::debug!(ward = %ward_id, "No new facts — skipping wiki compilation");
        return Ok(0);
    }

    // Load existing articles
    let existing = wiki_store.list_articles_typed(ward_id).await?;

    // Build compilation prompt
    let prompt = build_compilation_prompt(&existing, new_facts);

    // Call LLM
    let messages = vec![
        ChatMessage::system(
            "You are a knowledge compiler. Produce structured wiki articles from facts. \
             Respond with ONLY valid JSON."
                .to_string(),
        ),
        ChatMessage::user(prompt),
    ];

    let response = llm_client
        .chat(messages, None)
        .await
        .map_err(|e| format!("Wiki compilation LLM call failed: {e}"))?;

    let response_text = &response.content;

    // Parse response
    let compiled: CompilationResponse = parse_compilation_response(response_text)?;

    // Upsert articles. Two-stage dedup:
    // 1. Title-match: if the LLM reused an exact existing title (preferred
    //    behavior per prompt), the UNIQUE(ward_id, title) constraint on
    //    upsert updates in place.
    // 2. Embedding-similarity: if the LLM produced a *new* title for a topic
    //    that semantically overlaps with an existing article, merge into the
    //    existing article instead of creating a near-duplicate. Requires an
    //    embedding_client.
    let now = chrono::Utc::now().to_rfc3339();
    let mut upserted = 0;

    for article_data in &compiled.articles {
        let embedding = match embedding_client {
            Some(ec) => ec
                .embed(&[article_data.content.as_str()])
                .await
                .ok()
                .and_then(|mut vecs| {
                    if vecs.is_empty() {
                        None
                    } else {
                        Some(vecs.remove(0))
                    }
                }),
            None => None,
        };

        // Stage 2 dedup: if this is a "new" title (not matching any existing)
        // but its content embedding is very close to an existing article,
        // merge into that existing article by reusing its title.
        //
        // Threshold 0.82 chosen empirically: high enough to avoid false
        // merges of genuinely-distinct topics, low enough to catch the LLM
        // producing minor title variations like
        // `Portfolio Analysis Ward` vs `Portfolio Analysis Ward Data Availability`.
        let mut effective_title = article_data.title.clone();
        let title_is_new = !existing
            .iter()
            .any(|e| e.title == article_data.title && e.title != "__index__");

        if title_is_new {
            if let Some(ref new_emb) = embedding {
                if let Ok(matches) = wiki_store
                    .search_wiki_by_similarity_typed(ward_id, new_emb, 1)
                    .await
                {
                    if let Some((nearest, score)) = matches.first() {
                        if *score >= 0.82 && nearest.title != "__index__" {
                            tracing::info!(
                                ward = %ward_id,
                                new_title = %article_data.title,
                                merged_into = %nearest.title,
                                similarity = %format!("{:.3}", score),
                                "Wiki dedup: merging similar-topic article into existing"
                            );
                            effective_title = nearest.title.clone();
                        }
                    }
                }
            }
        }

        let article = WikiArticle {
            id: format!("wiki-{}-{}", ward_id, uuid::Uuid::new_v4()),
            ward_id: ward_id.to_string(),
            agent_id: agent_id.to_string(),
            title: effective_title,
            content: article_data.content.clone(),
            tags: article_data
                .tags
                .as_ref()
                .map(|t| serde_json::to_string(t).unwrap_or_default()),
            source_fact_ids: None,
            embedding,
            version: 1,
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        let article_v = match serde_json::to_value(&article) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(title = %article_data.title, error = %e, "Failed to encode wiki article");
                continue;
            }
        };
        if let Err(e) = wiki_store
            .upsert_article(article_v, article.embedding.clone())
            .await
        {
            tracing::warn!(title = %article_data.title, error = %e, "Failed to upsert wiki article");
        } else {
            upserted += 1;
        }
    }

    // Update index article
    if upserted > 0 {
        let index_content = build_index_content(ward_id, wiki_store).await?;
        let index_article = WikiArticle {
            id: format!("wiki-{}-index", ward_id),
            ward_id: ward_id.to_string(),
            agent_id: agent_id.to_string(),
            title: "__index__".to_string(),
            content: index_content,
            tags: None,
            source_fact_ids: None,
            embedding: None,
            version: 1,
            created_at: now.clone(),
            updated_at: now,
        };
        if let Ok(v) = serde_json::to_value(&index_article) {
            let _ = wiki_store.upsert_article(v, None).await;
        }
    }

    tracing::info!(ward = %ward_id, articles = upserted, "Ward wiki compilation complete");
    Ok(upserted)
}

fn build_compilation_prompt(existing: &[WikiArticle], new_facts: &[FactSummary]) -> String {
    let mut prompt = String::new();

    prompt.push_str(
        "You are compiling a ward knowledge base. Given new facts from a session \
         and existing wiki articles, produce UPDATED or NEW articles.\n\n\
         ## Core Principle\n\n\
         PREFER UPDATING EXISTING ARTICLES over creating new ones. Only create \
         a new article if the topic is genuinely not covered by any existing \
         article. Articles with overlapping topics are a failure mode — avoid \
         them.\n\n\
         ## Rules\n\n\
         - Each article covers ONE focused topic\n\
         - Article content: 100-500 words, factual, concise, no fluff\n\
         - Use the EXACT same `title` as an existing article when updating it \
           — titles are the dedup key. Never rename.\n\
         - If new facts don't fit any existing article, ONLY THEN create a new \
           article with a distinct, specific title\n\
         - Title conventions: Title Case, specific not generic \
           (e.g., `yfinance Rate Limiting Patterns`, not `Patterns`)\n\
         - Note contradictions between new and existing knowledge inside the \
           relevant article's content\n\n\
         ## Dedup Checklist (apply before emitting each article)\n\n\
         Before outputting a NEW article, ask:\n\
         1. Is there an existing article whose topic overlaps? → UPDATE it instead \
            (reuse its exact title)\n\
         2. Could this be merged into a broader existing article? → Merge instead \
            of creating\n\
         3. Is the title too generic (e.g., `Workflow`, `Structure`)? → Make it \
            specific to the ward/domain\n\n",
    );

    if !existing.is_empty() {
        prompt.push_str(&format!(
            "## Existing Articles ({} total — reuse these titles when updating)\n\n",
            existing.iter().filter(|a| a.title != "__index__").count()
        ));
        for article in existing {
            if article.title == "__index__" {
                continue;
            }
            prompt.push_str(&format!(
                "### {}\n{}\n\n",
                article.title,
                truncate_str(&article.content, 500)
            ));
        }
    } else {
        prompt.push_str(
            "## Existing Articles\n\n(none yet — this is the first compilation for this ward)\n\n",
        );
    }

    prompt.push_str("## New Facts From This Session\n\n");
    for fact in new_facts {
        prompt.push_str(&format!(
            "- [{}] {}: {}\n",
            fact.category, fact.key, fact.content
        ));
    }

    prompt.push_str(
        "\n\nRespond with JSON only:\n\
         {\"articles\": [{\"title\": \"...\", \"content\": \"...\", \"tags\": [\"...\"]}]}",
    );

    prompt
}

fn parse_compilation_response(text: &str) -> Result<CompilationResponse, String> {
    // Try to extract JSON from response (may have markdown fences)
    let json_text = if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            &text[start..=end]
        } else {
            text
        }
    } else {
        text
    };

    serde_json::from_str(json_text)
        .map_err(|e| format!("Failed to parse compilation response: {e}"))
}

async fn build_index_content(ward_id: &str, wiki_store: &dyn WikiStore) -> Result<String, String> {
    let articles = wiki_store.list_articles_typed(ward_id).await?;
    let mut index = format!("# {} Wiki Index\n\n", ward_id);
    for article in &articles {
        if article.title == "__index__" {
            continue;
        }
        // One-line summary: first sentence of content
        let summary = article
            .content
            .lines()
            .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .unwrap_or("(no summary)");
        let summary = truncate_str(summary, 100);
        index.push_str(&format!("- **{}** — {}\n", article.title, summary));
    }
    Ok(index)
}

/// Truncate a string to `max_len` characters, appending "..." if truncated.
/// Uses char boundaries to avoid panicking on multi-byte characters.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let end: usize = s
            .char_indices()
            .nth(max_len)
            .map_or(s.len(), |(idx, _)| idx);
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_compilation_prompt_with_facts() {
        let facts = vec![FactSummary {
            category: "pattern".into(),
            key: "rate_limiting".into(),
            content: "Use 1 req/sec for yfinance".into(),
        }];
        let prompt = build_compilation_prompt(&[], &facts);
        assert!(prompt.contains("rate_limiting"));
        assert!(prompt.contains("New Facts"));
        assert!(prompt.contains("JSON only"));
    }

    #[test]
    fn test_build_compilation_prompt_with_existing() {
        let existing = vec![WikiArticle {
            id: "1".into(),
            ward_id: "w".into(),
            agent_id: "r".into(),
            title: "old-topic".into(),
            content: "Old content here.".into(),
            tags: None,
            source_fact_ids: None,
            embedding: None,
            version: 1,
            created_at: "2026".into(),
            updated_at: "2026".into(),
        }];
        let facts = vec![FactSummary {
            category: "domain".into(),
            key: "new_fact".into(),
            content: "New info".into(),
        }];
        let prompt = build_compilation_prompt(&existing, &facts);
        assert!(prompt.contains("old-topic"));
        assert!(prompt.contains("Old content"));
        assert!(prompt.contains("new_fact"));
    }

    #[test]
    fn test_parse_compilation_response_clean() {
        let json = r#"{"articles": [{"title": "test", "content": "body", "tags": ["a"]}]}"#;
        let parsed = parse_compilation_response(json).unwrap();
        assert_eq!(parsed.articles.len(), 1);
        assert_eq!(parsed.articles[0].title, "test");
    }

    #[test]
    fn test_parse_compilation_response_with_fences() {
        let text = "```json\n{\"articles\": [{\"title\": \"t\", \"content\": \"c\"}]}\n```";
        let parsed = parse_compilation_response(text).unwrap();
        assert_eq!(parsed.articles.len(), 1);
    }

    #[test]
    fn test_parse_compilation_response_invalid() {
        let result = parse_compilation_response("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world", 5), "hello...");
        // Multi-byte safety
        assert_eq!(truncate_str("héllo", 3), "hél...");
    }

    // ------------------------------------------------------------------
    // Integration: compile_ward_wiki against a real WardWikiRepository +
    // scripted LLM / embedding clients.
    // ------------------------------------------------------------------

    use agent_runtime::llm::client::StreamCallback;
    use agent_runtime::llm::embedding::EmbeddingError;
    use agent_runtime::llm::LlmError;
    use async_trait::async_trait;
    use gateway_services::VaultPaths;
    use std::sync::{Arc, Mutex};
    use zbot_stores_sqlite::vector_index::VectorIndex;
    use zbot_stores_sqlite::{
        GatewayWikiStore, KnowledgeDatabase, SqliteVecIndex, WardWikiRepository,
    };

    /// Scripted LLM returning a fixed textual response — whatever the caller
    /// passes in. We only use `chat`; `chat_stream` panics if invoked so a
    /// mis-wired future caller is caught immediately.
    struct MockLlm {
        response: Mutex<String>,
    }
    impl MockLlm {
        fn new(text: &str) -> Self {
            Self {
                response: Mutex::new(text.to_string()),
            }
        }
    }

    #[async_trait]
    impl LlmClient for MockLlm {
        fn model(&self) -> &str {
            "mock-model"
        }
        fn provider(&self) -> &str {
            "mock"
        }
        async fn chat(
            &self,
            _messages: Vec<ChatMessage>,
            _tools: Option<serde_json::Value>,
        ) -> Result<agent_runtime::llm::client::ChatResponse, LlmError> {
            Ok(agent_runtime::llm::client::ChatResponse {
                content: self.response.lock().unwrap().clone(),
                tool_calls: None,
                reasoning: None,
                usage: None,
            })
        }
        async fn chat_stream(
            &self,
            _messages: Vec<ChatMessage>,
            _tools: Option<serde_json::Value>,
            _callback: StreamCallback,
        ) -> Result<agent_runtime::llm::client::ChatResponse, LlmError> {
            panic!("MockLlm::chat_stream should not be called in tests");
        }
    }

    /// Deterministic embedding client — same 384-float vector for every input.
    /// Matches the KnowledgeDatabase default dim so the vec-index write
    /// inside `upsert_article` succeeds.
    struct ConstEmbedding;
    #[async_trait]
    impl EmbeddingClient for ConstEmbedding {
        async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            let mut v = vec![0.0f32; 384];
            v[0] = 1.0;
            Ok(texts.iter().map(|_| v.clone()).collect())
        }
        fn dimensions(&self) -> usize {
            384
        }
        fn model_name(&self) -> String {
            "const".to_string()
        }
    }

    fn make_wiki_store() -> (tempfile::TempDir, GatewayWikiStore) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        std::fs::create_dir_all(paths.conversations_db().parent().expect("parent")).expect("mkdir");
        let db = Arc::new(KnowledgeDatabase::new(paths).expect("knowledge db"));
        let vec: Arc<dyn VectorIndex> =
            Arc::new(SqliteVecIndex::new(db.clone(), "wiki_articles_index", "article_id").unwrap());
        let repo = Arc::new(WardWikiRepository::new(db, vec));
        (tmp, GatewayWikiStore::new(repo))
    }

    #[tokio::test]
    async fn compile_ward_wiki_empty_facts_short_circuits() {
        let (_tmp, repo) = make_wiki_store();
        let llm = MockLlm::new("unused");
        let out = compile_ward_wiki("w1", "root", &[], &repo, &llm, None)
            .await
            .expect("compile");
        assert_eq!(out, 0);
    }

    #[tokio::test]
    async fn compile_ward_wiki_upserts_article_and_writes_index() {
        let (_tmp, repo) = make_wiki_store();
        let llm_response = r#"{"articles": [
            {"title": "Rate Limits", "content": "Use 1 req/sec.", "tags": ["yfinance"]}
        ]}"#;
        let llm = MockLlm::new(llm_response);
        let facts = vec![FactSummary {
            category: "pattern".into(),
            key: "rate_limiting".into(),
            content: "1 req/sec".into(),
        }];

        let out = compile_ward_wiki("w1", "root", &facts, &repo, &llm, None)
            .await
            .expect("compile");
        assert_eq!(out, 1);

        let articles = repo.list_articles_typed("w1").await.expect("list");
        // Expect 2: the new article + the __index__ rollup.
        assert!(articles.iter().any(|a| a.title == "Rate Limits"));
        assert!(articles.iter().any(|a| a.title == "__index__"));

        // Index must name the new article.
        let index = articles
            .iter()
            .find(|a| a.title == "__index__")
            .expect("index exists");
        assert!(index.content.contains("Rate Limits"));
        assert!(index.content.contains("Use 1 req/sec"));
    }

    #[tokio::test]
    async fn compile_ward_wiki_malformed_llm_json_returns_err() {
        let (_tmp, repo) = make_wiki_store();
        let llm = MockLlm::new("not json at all");
        let facts = vec![FactSummary {
            category: "x".into(),
            key: "y".into(),
            content: "z".into(),
        }];

        let err = compile_ward_wiki("w1", "root", &facts, &repo, &llm, None)
            .await
            .unwrap_err();
        assert!(err.contains("parse"));
    }

    #[tokio::test]
    async fn compile_ward_wiki_uses_embedding_client_when_provided() {
        // We don't assert dedup behavior (that's integration-testing the
        // repo's similarity search) — just that the embedding path runs
        // end-to-end without erroring when an embedding client is supplied.
        let (_tmp, repo) = make_wiki_store();
        let llm_response = r#"{"articles": [
            {"title": "Doc A", "content": "body", "tags": null}
        ]}"#;
        let llm = MockLlm::new(llm_response);
        let emb = ConstEmbedding;
        let facts = vec![FactSummary {
            category: "pattern".into(),
            key: "k".into(),
            content: "c".into(),
        }];

        let out = compile_ward_wiki("w1", "root", &facts, &repo, &llm, Some(&emb))
            .await
            .expect("compile");
        assert_eq!(out, 1);
    }

    // ------------------------------------------------------------------
    // Pure helper — build_index_content — exercised directly now that it
    // takes a live repo.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn build_index_content_empty_ward_returns_just_header() {
        let (_tmp, repo) = make_wiki_store();
        let out = build_index_content("w1", &repo).await.expect("build index");
        assert!(out.starts_with("# w1 Wiki Index"));
        // No bullet lines: the only article would've been __index__ itself,
        // which is filtered out.
        assert!(!out.contains("\n- "));
    }
}
