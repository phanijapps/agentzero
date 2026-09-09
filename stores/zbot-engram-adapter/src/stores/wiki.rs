//! `WikiStore` implementation backed by Engram knowledge records.

use agent_primitives::vec_math::cosine_f64_opt;
use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use engram_knowledge::KnowledgeRepository;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use zbot_stores_domain::WikiArticle;
use zbot_stores_traits::{EmbeddingQueryIdentity, WikiStats, WikiStore};

use crate::{
    bootstrap::EngramProvider,
    capabilities::AdapterFeature,
    config::{AdapterConfig, ProviderMode},
    error::{AdapterError, AdapterResult},
    mapping::knowledge::wiki_article_to_knowledge_records_with_governance,
    scope::ScopeMapper,
};

const SIDECAR_COMPONENT: &str = "wiki_sidecar";

/// Engram-backed implementation of AgentZero's wiki store trait.
#[derive(Clone)]
pub struct EngramWikiStore {
    knowledge: Arc<dyn KnowledgeRepository>,
    mapper: ScopeMapper,
    governance: crate::governance::GovernancePolicy,
    sidecar: WikiSidecar,
}

impl EngramWikiStore {
    /// Open an Engram-backed wiki store for `ProviderMode::Engram`.
    pub fn open(config: AdapterConfig) -> AdapterResult<Self> {
        if config.provider_mode != ProviderMode::Engram {
            return Err(AdapterError::UnsupportedFeature {
                feature: "wiki",
                reason: "provider mode is not engram".to_string(),
            });
        }

        config.validate()?;
        let provider = EngramProvider::open(config.clone())?;
        Self::from_provider(config, &provider)
    }

    /// Build a wiki store from an already-bootstrapped Engram provider.
    pub fn from_provider(config: AdapterConfig, provider: &EngramProvider) -> AdapterResult<Self> {
        if config.provider_mode != ProviderMode::Engram {
            return Err(AdapterError::UnsupportedFeature {
                feature: "wiki",
                reason: "provider mode is not engram".to_string(),
            });
        }

        config.validate()?;
        provider.require_feature(AdapterFeature::Wiki)?;
        let mapper = config.scope_mapper()?;
        let knowledge = provider.knowledge()?;
        let sidecar = WikiSidecar::open(
            &config.compatibility_store_path("zbot-wiki.sqlite")?,
            embedding_identity_from_config(&config),
        )?;

        Ok(Self {
            knowledge,
            mapper,
            governance: config.governance.clone(),
            sidecar,
        })
    }

    /// Return the adapter-preserved article embedding by article id.
    pub fn get_article_embedding(&self, id: &str) -> Result<Option<Vec<f32>>, String> {
        self.sidecar.get_embedding(id)
    }

    async fn upsert_article_record(
        &self,
        mut article: WikiArticle,
        embedding: Option<Vec<f32>>,
    ) -> Result<(), String> {
        if let Some(existing) = self
            .sidecar
            .get_by_title(&article.ward_id, &article.title)?
        {
            article.id = existing.article.id;
            article.created_at = existing.article.created_at;
            article.version = existing.article.version.saturating_add(1);
        }
        if let Some(embedding) = embedding.clone() {
            article.embedding = Some(embedding);
        }

        let records = wiki_article_to_knowledge_records_with_governance(
            &article,
            &self.mapper,
            Some(&self.governance),
        )
        .map_err(AdapterError::into_trait_error)?;
        self.knowledge
            .put_source(records.source)
            .await
            .map_err(|error| error.to_string())?;
        self.knowledge
            .put_document(records.document)
            .await
            .map_err(|error| error.to_string())?;
        self.knowledge
            .put_chunk(records.chunk)
            .await
            .map_err(|error| error.to_string())?;

        let stored_embedding = article.embedding.clone();
        article.embedding = None;
        self.sidecar
            .store_article(&article, stored_embedding.as_deref())
    }
}

#[async_trait]
impl WikiStore for EngramWikiStore {
    async fn list_articles(&self, ward_id: &str) -> Result<Vec<Value>, String> {
        self.sidecar
            .list_articles(ward_id)?
            .into_iter()
            .map(|entry| serde_json::to_value(entry.article).map_err(|error| error.to_string()))
            .collect()
    }

    async fn get_article(&self, ward_id: &str, title: &str) -> Result<Option<Value>, String> {
        self.sidecar
            .get_by_title(ward_id, title)?
            .map(|entry| serde_json::to_value(entry.article).map_err(|error| error.to_string()))
            .transpose()
    }

    async fn upsert_article(
        &self,
        mut article: WikiArticle,
        embedding: Option<Vec<f32>>,
    ) -> Result<(), String> {
        article.embedding = embedding.clone();
        self.upsert_article_record(article, embedding).await
    }

    async fn delete_article(&self, ward_id: &str, title: &str) -> Result<bool, String> {
        self.sidecar.delete_article(ward_id, title)
    }

    async fn search_wiki_hybrid(
        &self,
        ward_id: Option<&str>,
        query: &str,
        limit: usize,
        query_embedding: Option<&[f32]>,
    ) -> Result<Vec<Value>, String> {
        self.search_wiki_hybrid_with_identity(ward_id, query, limit, query_embedding, None)
            .await
    }

    async fn search_wiki_hybrid_with_identity(
        &self,
        ward_id: Option<&str>,
        query: &str,
        limit: usize,
        query_embedding: Option<&[f32]>,
        query_identity: Option<&EmbeddingQueryIdentity>,
    ) -> Result<Vec<Value>, String> {
        let limit = limit.max(1);
        let query_terms = normalize_terms(query);
        let mut hits = self
            .sidecar
            .list_matching_scope(ward_id)?
            .into_iter()
            .filter_map(|entry| {
                let text_score = text_score(&entry.article, &query_terms);
                let vector_score = query_embedding
                    .zip(entry.embedding.as_deref())
                    .filter(|(query, embedding)| {
                        identity_compatible(
                            &self.sidecar.embedding_identity,
                            query_identity,
                            query.len(),
                        ) && stored_identity_compatible(
                            &self.sidecar.embedding_identity,
                            entry.embedding_identity.as_ref(),
                            embedding.len(),
                        )
                    })
                    .and_then(|(query, embedding)| cosine_f64_opt(query, embedding));
                let score = text_score + vector_score.unwrap_or(0.0);
                if score <= 0.0 {
                    return None;
                }
                let match_source = match (text_score > 0.0, vector_score.is_some()) {
                    (true, true) => "hybrid",
                    (true, false) => "fts",
                    (false, true) => "vec",
                    (false, false) => return None,
                };
                Some((entry, score, match_source.to_string()))
            })
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.0.article.title.cmp(&right.0.article.title))
        });
        hits.truncate(limit);

        hits.into_iter()
            .map(|(entry, score, match_source)| {
                Ok(json!({
                    "article": entry.article,
                    "score": score,
                    "match_source": match_source }))
            })
            .collect()
    }

    async fn wiki_stats(&self) -> Result<WikiStats, String> {
        Ok(WikiStats {
            total: self.sidecar.count_articles()?,
        })
    }

    async fn search_wiki_by_similarity_typed(
        &self,
        ward_id: &str,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(WikiArticle, f64)>, String> {
        self.search_wiki_by_similarity_typed_with_identity(ward_id, embedding, None, limit)
            .await
    }

    async fn search_wiki_by_similarity_typed_with_identity(
        &self,
        ward_id: &str,
        embedding: &[f32],
        query_identity: Option<&EmbeddingQueryIdentity>,
        limit: usize,
    ) -> Result<Vec<(WikiArticle, f64)>, String> {
        if !identity_compatible(
            &self.sidecar.embedding_identity,
            query_identity,
            embedding.len(),
        ) {
            return Ok(Vec::new());
        }
        let mut hits = self
            .sidecar
            .list_matching_scope(Some(ward_id))?
            .into_iter()
            .filter_map(|entry| {
                if !stored_identity_compatible(
                    &self.sidecar.embedding_identity,
                    entry.embedding_identity.as_ref(),
                    entry.embedding.as_ref()?.len(),
                ) {
                    return None;
                }
                let score = cosine_f64_opt(embedding, entry.embedding.as_deref()?)?;
                Some((entry.article, score))
            })
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(limit.max(1));
        Ok(hits)
    }
}

#[derive(Clone)]
struct WikiSidecar {
    connection: Arc<Mutex<Connection>>,
    embedding_identity: EmbeddingQueryIdentity,
}

#[derive(Debug, Clone)]
struct WikiSidecarEntry {
    article: WikiArticle,
    embedding: Option<Vec<f32>>,
    embedding_identity: Option<EmbeddingQueryIdentity>,
}

impl WikiSidecar {
    fn open(path: &Path, embedding_identity: EmbeddingQueryIdentity) -> AdapterResult<Self> {
        let connection = Connection::open(path).map_err(|error| AdapterError::Storage {
            component: SIDECAR_COMPONENT,
            reason: error.to_string(),
        })?;
        connection
            .execute_batch(
                r#"
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;
                PRAGMA busy_timeout = 5000;
                CREATE TABLE IF NOT EXISTS wiki_articles (
                    id TEXT PRIMARY KEY,
                    ward_id TEXT NOT NULL,
                    agent_id TEXT NOT NULL,
                    title TEXT NOT NULL,
                    content TEXT NOT NULL,
                    tags TEXT,
                    source_fact_ids TEXT,
                    version INTEGER NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    article_json TEXT NOT NULL,
                    embedding_json TEXT,
                    embedding_identity_json TEXT,
                    UNIQUE(ward_id, title)
                );
                CREATE INDEX IF NOT EXISTS idx_wiki_articles_ward_title
                    ON wiki_articles(ward_id, title);
                CREATE INDEX IF NOT EXISTS idx_wiki_articles_title
                    ON wiki_articles(title);
                "#,
            )
            .map_err(|error| AdapterError::Storage {
                component: SIDECAR_COMPONENT,
                reason: error.to_string(),
            })?;
        ensure_optional_column(
            &connection,
            "wiki_articles",
            "embedding_identity_json",
            "TEXT",
        )
        .map_err(|error| AdapterError::Storage {
            component: SIDECAR_COMPONENT,
            reason: error.to_string(),
        })?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            embedding_identity,
        })
    }

    fn store_article(
        &self,
        article: &WikiArticle,
        embedding: Option<&[f32]>,
    ) -> Result<(), String> {
        let article_json = serde_json::to_string(article).map_err(|error| error.to_string())?;
        let embedding_json = embedding
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        let embedding_identity_json = embedding.map(|_| encode_identity(&self.embedding_identity));
        self.lock()?
            .execute(
                r#"
                INSERT INTO wiki_articles
                    (id, ward_id, agent_id, title, content, tags, source_fact_ids,
                     version, created_at, updated_at, article_json, embedding_json, embedding_identity_json)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                ON CONFLICT(ward_id, title) DO UPDATE SET
                    id = excluded.id,
                    agent_id = excluded.agent_id,
                    content = excluded.content,
                    tags = excluded.tags,
                    source_fact_ids = excluded.source_fact_ids,
                    version = excluded.version,
                    updated_at = excluded.updated_at,
                    article_json = excluded.article_json,
                    embedding_json = COALESCE(excluded.embedding_json, wiki_articles.embedding_json),
                    embedding_identity_json = COALESCE(excluded.embedding_identity_json, wiki_articles.embedding_identity_json)
                "#,
                params![
                    article.id,
                    article.ward_id,
                    article.agent_id,
                    article.title,
                    article.content,
                    article.tags,
                    article.source_fact_ids,
                    article.version,
                    article.created_at,
                    article.updated_at,
                    article_json,
                    embedding_json,
                    embedding_identity_json,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn list_articles(&self, ward_id: &str) -> Result<Vec<WikiSidecarEntry>, String> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT article_json, embedding_json, embedding_identity_json FROM wiki_articles \
                 WHERE ward_id = ?1 ORDER BY title",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![ward_id], decode_entry)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

    fn list_matching_scope(&self, ward_id: Option<&str>) -> Result<Vec<WikiSidecarEntry>, String> {
        let connection = self.lock()?;
        if let Some(ward_id) = ward_id {
            let mut statement = connection
                .prepare(
                    "SELECT article_json, embedding_json, embedding_identity_json FROM wiki_articles \
                     WHERE ward_id = ?1 ORDER BY title",
                )
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map(params![ward_id], decode_entry)
                .map_err(|error| error.to_string())?;
            collect_rows(rows)
        } else {
            let mut statement = connection
                .prepare(
                    "SELECT article_json, embedding_json, embedding_identity_json FROM wiki_articles ORDER BY title",
                )
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([], decode_entry)
                .map_err(|error| error.to_string())?;
            collect_rows(rows)
        }
    }

    fn get_by_title(&self, ward_id: &str, title: &str) -> Result<Option<WikiSidecarEntry>, String> {
        self.lock()?
            .query_row(
                "SELECT article_json, embedding_json, embedding_identity_json FROM wiki_articles \
                 WHERE ward_id = ?1 AND title = ?2",
                params![ward_id, title],
                decode_entry,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    fn get_embedding(&self, id: &str) -> Result<Option<Vec<f32>>, String> {
        self.lock()?
            .query_row(
                "SELECT embedding_json, embedding_identity_json FROM wiki_articles WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?
            .map(|(embedding_json, identity_json)| {
                let embedding = decode_embedding(embedding_json)?;
                let Some(vector) = embedding else {
                    return Ok(None);
                };
                let identity = decode_identity(identity_json)?;
                if stored_identity_compatible(
                    &self.embedding_identity,
                    identity.as_ref(),
                    vector.len(),
                ) {
                    Ok(Some(vector))
                } else {
                    Ok(None)
                }
            })
            .transpose()
            .map(Option::flatten)
    }

    fn delete_article(&self, ward_id: &str, title: &str) -> Result<bool, String> {
        let rows = self
            .lock()?
            .execute(
                "DELETE FROM wiki_articles WHERE ward_id = ?1 AND title = ?2",
                params![ward_id, title],
            )
            .map_err(|error| error.to_string())?;
        Ok(rows > 0)
    }

    fn count_articles(&self) -> Result<i64, String> {
        self.lock()?
            .query_row("SELECT COUNT(*) FROM wiki_articles", [], |row| row.get(0))
            .map_err(|error| error.to_string())
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.connection
            .lock()
            .map_err(|_| "wiki sidecar connection lock poisoned".to_string())
    }
}

fn decode_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<WikiSidecarEntry> {
    let article_json: String = row.get(0)?;
    let embedding_json: Option<String> = row.get(1)?;
    let embedding_identity_json: Option<String> = row.get(2)?;
    let article = serde_json::from_str::<WikiArticle>(&article_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let embedding = embedding_json
        .map(|json| {
            serde_json::from_str::<Vec<f32>>(&json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .transpose()?;
    let embedding_identity = decode_identity(embedding_identity_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
        )
    })?;
    Ok(WikiSidecarEntry {
        article,
        embedding,
        embedding_identity,
    })
}

fn collect_rows<I>(rows: I) -> Result<Vec<WikiSidecarEntry>, String>
where
    I: Iterator<Item = rusqlite::Result<WikiSidecarEntry>>,
{
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn decode_embedding(value: Option<String>) -> Result<Option<Vec<f32>>, String> {
    value
        .map(|json| serde_json::from_str::<Vec<f32>>(&json))
        .transpose()
        .map_err(|error| format!("decode embedding: {error}"))
}

fn embedding_identity_from_config(config: &AdapterConfig) -> EmbeddingQueryIdentity {
    EmbeddingQueryIdentity {
        provider_type: config.embedding_provider.provider_type.clone(),
        model: config.embedding_provider.model.clone(),
        dimensions: config.embedding_provider.dimensions,
        prompt_profile: config.embedding_provider.prompt_profile.clone(),
        normalization: config.embedding_provider.normalization.clone(),
    }
}

fn encode_identity(identity: &EmbeddingQueryIdentity) -> String {
    json!({
        "providerType": identity.provider_type.clone(),
        "model": identity.model.clone(),
        "dimensions": identity.dimensions,
        "promptProfile": identity.prompt_profile.clone(),
        "normalization": identity.normalization.clone() })
    .to_string()
}

fn decode_identity(value: Option<String>) -> Result<Option<EmbeddingQueryIdentity>, String> {
    let Some(json) = value else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(&json)
        .map_err(|error| format!("decode embedding identity: {error}"))?;
    let dimensions = value
        .get("dimensions")
        .and_then(Value::as_u64)
        .ok_or_else(|| "decode embedding identity: missing dimensions".to_string())?
        as u32;
    Ok(Some(EmbeddingQueryIdentity {
        provider_type: value
            .get("providerType")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        model: value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        dimensions,
        prompt_profile: value
            .get("promptProfile")
            .and_then(Value::as_str)
            .unwrap_or("query")
            .to_string(),
        normalization: value
            .get("normalization")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    }))
}

fn identity_compatible(
    expected: &EmbeddingQueryIdentity,
    actual: Option<&EmbeddingQueryIdentity>,
    vector_dimensions: usize,
) -> bool {
    let Some(actual) = actual else {
        return false;
    };
    expected.provider_type == actual.provider_type
        && expected.model == actual.model
        && expected.dimensions == actual.dimensions
        && expected.dimensions as usize == vector_dimensions
        && expected.prompt_profile == actual.prompt_profile
        && expected.normalization == actual.normalization
}

fn stored_identity_compatible(
    expected: &EmbeddingQueryIdentity,
    actual: Option<&EmbeddingQueryIdentity>,
    vector_dimensions: usize,
) -> bool {
    identity_compatible(expected, actual, vector_dimensions)
}

fn ensure_optional_column(
    connection: &Connection,
    table: &str,
    column: &str,
    column_type: &str,
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if names.iter().any(|name| name == column) {
        return Ok(());
    }
    connection.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {column_type}"),
        [],
    )?;
    Ok(())
}

fn normalize_terms(query: &str) -> Vec<String> {
    query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(|term| term.to_lowercase())
        .collect()
}

fn text_score(article: &WikiArticle, query_terms: &[String]) -> f64 {
    if query_terms.is_empty() {
        return 0.0;
    }
    let haystack = format!(
        "{} {} {}",
        article.title.to_lowercase(),
        article.content.to_lowercase(),
        article.tags.clone().unwrap_or_default().to_lowercase()
    );
    query_terms
        .iter()
        .filter(|term| haystack.contains(term.as_str()))
        .count() as f64
}
