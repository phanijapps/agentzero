//! Belief Network read tool.
//!
//! Split out of the `memory` god tool: belief reads go to a different
//! store with different semantics (synthesized aggregate stances over
//! evidence) than durable facts. One tool, one store, one schema.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use agent_primitives::{AgentError, Result, Tool, ToolContext, ToolPermissions};
use zbot_stores_traits::{BeliefContradictionStore, BeliefStore};

/// Read-only view over the Belief Network.
///
/// Both stores are optional — when the Belief Network is disabled the
/// actions surface a clean "not configured" error instead of a panic,
/// mirroring the fact-store plumbing in `memory`.
pub struct BeliefTool {
    belief_store: Option<Arc<dyn BeliefStore>>,
    contradiction_store: Option<Arc<dyn BeliefContradictionStore>>,
}

impl BeliefTool {
    #[must_use]
    pub fn new(
        belief_store: Option<Arc<dyn BeliefStore>>,
        contradiction_store: Option<Arc<dyn BeliefContradictionStore>>,
    ) -> Self {
        Self {
            belief_store,
            contradiction_store,
        }
    }

    /// Read the active belief for a subject.
    ///
    /// Returns `{ "belief": null }` when no belief exists. `subject` is
    /// required. `as_of` is an optional ISO-8601 / RFC3339 timestamp for
    /// point-in-time queries — omitting it defaults to "now" inside the
    /// store layer.
    async fn action_belief(
        &self,
        ctx: &dyn ToolContext,
        agent_id: &str,
        args: &Value,
    ) -> Result<Value> {
        let subject = args
            .get("subject")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'subject' for belief".to_string()))?;

        let as_of: Option<chrono::DateTime<chrono::Utc>> = match args
            .get("as_of")
            .and_then(|v| v.as_str())
        {
            Some(s) => Some(
                chrono::DateTime::parse_from_rfc3339(s)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .map_err(|_| {
                        AgentError::Tool("invalid as_of timestamp, expected ISO-8601".to_string())
                    })?,
            ),
            None => None,
        };

        // Partition mirrors the recall convention — agent_id buckets the
        // belief space. Ward overrides via `ward_id` context when set.
        let partition_id = ctx
            .get_state("ward_id")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| agent_id.to_string());

        let Some(store) = self.belief_store.as_ref() else {
            return Err(AgentError::Tool(
                "Belief Network is not configured (enable execution.memory.beliefNetwork in settings)"
                    .to_string(),
            ));
        };

        let belief = store
            .get_belief(&partition_id, subject, as_of)
            .await
            .map_err(AgentError::Tool)?;

        let payload = match belief {
            Some(b) => json!({
                "belief": {
                    "id": b.id,
                    "partition_id": b.partition_id,
                    "subject": b.subject,
                    "content": b.content,
                    "confidence": b.confidence,
                    "valid_from": b.valid_from.map(|t| t.to_rfc3339()),
                    "valid_until": b.valid_until.map(|t| t.to_rfc3339()),
                    "source_fact_ids": b.source_fact_ids,
                    "synthesizer_version": b.synthesizer_version,
                    "reasoning": b.reasoning }
            }),
            None => json!({ "belief": null }),
        };
        Ok(payload)
    }

    /// List belief contradictions.
    ///
    /// - When `belief_id` is provided: returns every contradiction
    ///   involving that belief (either side of the pair).
    /// - When `belief_id` is omitted: returns the most recent
    ///   contradictions in the agent's partition (or `ward_id` override),
    ///   capped by `limit` (default 10).
    async fn action_contradictions(
        &self,
        ctx: &dyn ToolContext,
        agent_id: &str,
        args: &Value,
    ) -> Result<Value> {
        let Some(store) = self.contradiction_store.as_ref() else {
            return Err(AgentError::Tool(
                "Belief Network is not configured (enable execution.memory.beliefNetwork in settings)"
                    .to_string(),
            ));
        };

        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(10);

        let rows = if let Some(belief_id) = args.get("belief_id").and_then(|v| v.as_str()) {
            store
                .for_belief(belief_id)
                .await
                .map_err(AgentError::Tool)?
        } else {
            let partition_id = ctx
                .get_state("ward_id")
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| agent_id.to_string());
            store
                .list_recent(&partition_id, limit)
                .await
                .map_err(AgentError::Tool)?
        };

        let serialized: Vec<Value> = rows
            .into_iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "belief_a_id": c.belief_a_id,
                    "belief_b_id": c.belief_b_id,
                    "contradiction_type": c.contradiction_type,
                    "severity": c.severity,
                    "judge_reasoning": c.judge_reasoning,
                    "detected_at": c.detected_at.to_rfc3339(),
                    "resolved_at": c.resolved_at.map(|t| t.to_rfc3339()),
                    "resolution": c.resolution })
            })
            .collect();

        Ok(json!({
            "count": serialized.len(),
            "contradictions": serialized }))
    }
}

#[async_trait]
impl Tool for BeliefTool {
    fn name(&self) -> &str {
        "belief"
    }

    fn description(&self) -> &str {
        "Read the synthesized Belief Network. Actions: belief (active stance for a (partition, subject) at as_of), \
        contradictions (by belief_id or recent). Beliefs aggregate supporting/contradicting evidence over time."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["belief", "contradictions"],
                    "description": "The belief operation to perform"
                },
                "subject": {
                    "type": "string",
                    "description": "Subject key for the 'belief' action — e.g. 'user.location' or 'domain.finance.acn.valuation_verdict'"
                },
                "belief_id": {
                    "type": "string",
                    "description": "Belief ID to scope 'contradictions' to — when omitted, returns recent contradictions in the partition"
                },
                "as_of": {
                    "type": "string",
                    "format": "date-time",
                    "description": "ISO-8601 timestamp (for belief). When set, returns the belief that was active at this time."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum contradictions to return (default 10)"
                }
            },
            "required": ["action"]
        }))
    }

    fn permissions(&self) -> ToolPermissions {
        ToolPermissions::safe()
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        if let Some(error_type) = args.get("__error__").and_then(|v| v.as_str()) {
            let message = args
                .get("__message__")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Err(AgentError::Tool(format!("{}: {}", error_type, message)));
        }

        let agent_id = ctx
            .get_state("app:agent_id")
            .and_then(|v| v.as_str().map(String::from))
            .or_else(|| {
                ctx.get_state("app:root_agent_id")
                    .and_then(|v| v.as_str().map(String::from))
            })
            .ok_or_else(|| AgentError::Tool("No agent ID in context".to_string()))?;

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AgentError::Tool(
                    "Missing 'action' parameter. Expected shape: {\"action\":\"belief\", \"subject\":\"user.location\"}"
                        .to_string(),
                )
            })?;

        match action {
            "belief" => self.action_belief(ctx.as_ref(), &agent_id, &args).await,
            "contradictions" => {
                self.action_contradictions(ctx.as_ref(), &agent_id, &args)
                    .await
            }
            _ => Err(AgentError::Tool(format!("Unknown action: {}", action))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_primitives::{CallbackContext, Content, EventActions, ReadonlyContext};

    fn ctx_with_session(session_id: &str) -> impl ToolContext + 'static {
        struct Ctx(String);
        impl ReadonlyContext for Ctx {
            fn invocation_id(&self) -> &str {
                "t"
            }
            fn agent_name(&self) -> &str {
                "t"
            }
            fn user_id(&self) -> &str {
                "t"
            }
            fn app_name(&self) -> &str {
                "t"
            }
            fn session_id(&self) -> &str {
                &self.0
            }
            fn branch(&self) -> &str {
                "t"
            }
            fn user_content(&self) -> &Content {
                use std::sync::LazyLock;
                static C: LazyLock<Content> = LazyLock::new(|| Content {
                    role: "user".to_string(),
                    parts: vec![],
                });
                &C
            }
        }
        impl CallbackContext for Ctx {
            fn get_state(&self, key: &str) -> Option<Value> {
                (key == "app:agent_id").then(|| json!("root"))
            }
            fn set_state(&self, _key: String, _value: Value) {}
        }
        impl ToolContext for Ctx {
            fn function_call_id(&self) -> String {
                "t".to_string()
            }
            fn actions(&self) -> EventActions {
                EventActions::default()
            }
            fn set_actions(&self, _: EventActions) {}
        }
        Ctx(session_id.to_string())
    }

    fn stub_belief_store_with(belief: Option<zbot_stores_traits::Belief>) -> Arc<dyn BeliefStore> {
        use std::sync::Mutex as StdMutex;
        struct StubBeliefStore {
            stored: StdMutex<Option<zbot_stores_traits::Belief>>,
        }

        #[async_trait]
        impl BeliefStore for StubBeliefStore {
            async fn get_belief(
                &self,
                _partition_id: &str,
                _subject: &str,
                _as_of: Option<chrono::DateTime<chrono::Utc>>,
            ) -> std::result::Result<Option<zbot_stores_traits::Belief>, String> {
                Ok(self.stored.lock().unwrap().clone())
            }
            async fn list_beliefs(
                &self,
                _partition_id: &str,
                _limit: usize,
            ) -> std::result::Result<Vec<zbot_stores_traits::Belief>, String> {
                Ok(vec![])
            }
            async fn upsert_belief(
                &self,
                b: &zbot_stores_traits::Belief,
            ) -> std::result::Result<(), String> {
                *self.stored.lock().unwrap() = Some(b.clone());
                Ok(())
            }
            async fn supersede_belief(
                &self,
                _old_id: &str,
                _new_id: &str,
                _t: chrono::DateTime<chrono::Utc>,
            ) -> std::result::Result<(), String> {
                Ok(())
            }
            async fn mark_stale(&self, _belief_id: &str) -> std::result::Result<(), String> {
                Ok(())
            }
            async fn retract_belief(
                &self,
                _belief_id: &str,
                _t: chrono::DateTime<chrono::Utc>,
            ) -> std::result::Result<(), String> {
                Ok(())
            }
            async fn beliefs_referencing_fact(
                &self,
                _fact_id: &str,
            ) -> std::result::Result<Vec<String>, String> {
                Ok(vec![])
            }
            async fn get_belief_by_id(
                &self,
                _belief_id: &str,
            ) -> std::result::Result<Option<zbot_stores_traits::Belief>, String> {
                Ok(self.stored.lock().unwrap().clone())
            }
            async fn list_stale(
                &self,
                _partition_id: &str,
                _limit: usize,
            ) -> std::result::Result<Vec<zbot_stores_traits::Belief>, String> {
                Ok(vec![])
            }
            async fn clear_stale(&self, _belief_id: &str) -> std::result::Result<(), String> {
                Ok(())
            }
            async fn search_beliefs(
                &self,
                _partition_id: &str,
                _query_embedding: &[f32],
                _limit: usize,
            ) -> std::result::Result<Vec<zbot_stores_traits::ScoredBelief>, String> {
                Ok(vec![])
            }
        }

        Arc::new(StubBeliefStore {
            stored: StdMutex::new(belief),
        })
    }

    #[tokio::test]
    async fn belief_returns_belief_when_present() {
        let now = chrono::Utc::now();
        let belief = zbot_stores_traits::Belief {
            id: "b-1".to_string(),
            partition_id: "root".to_string(),
            subject: "user.location".to_string(),
            content: "Mason, OH".to_string(),
            confidence: 0.9,
            valid_from: Some(now),
            valid_until: None,
            source_fact_ids: vec!["fact-1".to_string()],
            synthesizer_version: 1,
            reasoning: None,
            created_at: now,
            updated_at: now,
            superseded_by: None,
            stale: false,
            embedding: None,
        };
        let tool = BeliefTool::new(Some(stub_belief_store_with(Some(belief))), None);
        let ctx = ctx_with_session("sess-belief");

        let args = json!({ "action": "belief", "subject": "user.location" });
        let result = tool.execute(Arc::new(ctx), args).await.unwrap();
        let b = &result["belief"];
        assert_eq!(b["content"], "Mason, OH");
        assert_eq!(b["confidence"], 0.9);
        assert_eq!(b["source_fact_ids"], json!(["fact-1"]));
    }

    #[tokio::test]
    async fn belief_returns_null_when_absent() {
        let tool = BeliefTool::new(Some(stub_belief_store_with(None)), None);
        let ctx = ctx_with_session("sess-belief-null");

        let args = json!({ "action": "belief", "subject": "no.such.subject" });
        let result = tool.execute(Arc::new(ctx), args).await.unwrap();
        assert_eq!(result["belief"], Value::Null);
    }

    fn sample_contradiction(id: &str, a: &str, b: &str) -> zbot_stores_traits::BeliefContradiction {
        use zbot_stores_traits::{BeliefContradiction, ContradictionType};
        BeliefContradiction {
            id: id.to_string(),
            belief_a_id: a.to_string(),
            belief_b_id: b.to_string(),
            contradiction_type: ContradictionType::Logical,
            severity: 0.9,
            judge_reasoning: Some("test reasoning".to_string()),
            detected_at: chrono::Utc::now(),
            resolved_at: None,
            resolution: None,
        }
    }

    /// In-memory contradiction store stub. `for_belief` returns
    /// `for_belief_rows`; `list_recent` returns `list_recent_rows`. Lets
    /// tests assert routing without an SQLite dependency.
    struct StubContradictionStore {
        for_belief_rows: std::sync::Mutex<Vec<zbot_stores_traits::BeliefContradiction>>,
        list_recent_rows: std::sync::Mutex<Vec<zbot_stores_traits::BeliefContradiction>>,
    }

    #[async_trait::async_trait]
    impl zbot_stores_traits::BeliefContradictionStore for StubContradictionStore {
        async fn insert_contradiction(
            &self,
            _c: &zbot_stores_traits::BeliefContradiction,
        ) -> std::result::Result<(), String> {
            Ok(())
        }
        async fn for_belief(
            &self,
            _belief_id: &str,
        ) -> std::result::Result<Vec<zbot_stores_traits::BeliefContradiction>, String> {
            Ok(self.for_belief_rows.lock().unwrap().clone())
        }
        async fn list_recent(
            &self,
            _partition_id: &str,
            _limit: usize,
        ) -> std::result::Result<Vec<zbot_stores_traits::BeliefContradiction>, String> {
            Ok(self.list_recent_rows.lock().unwrap().clone())
        }
        async fn pair_exists(&self, _a: &str, _b: &str) -> std::result::Result<bool, String> {
            Ok(false)
        }
        async fn resolve(
            &self,
            _id: &str,
            _r: zbot_stores_traits::Resolution,
        ) -> std::result::Result<(), String> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn contradictions_by_belief_id_returns_rows() {
        let store = Arc::new(StubContradictionStore {
            for_belief_rows: std::sync::Mutex::new(vec![
                sample_contradiction("c-1", "b-a", "b-b"),
                sample_contradiction("c-2", "b-b", "b-c"),
            ]),
            list_recent_rows: std::sync::Mutex::new(vec![]),
        });
        let tool = BeliefTool::new(
            None,
            Some(store as Arc<dyn zbot_stores_traits::BeliefContradictionStore>),
        );
        let ctx = ctx_with_session("sess-contradictions");

        let args = json!({ "action": "contradictions", "belief_id": "b-b" });
        let result = tool.execute(Arc::new(ctx), args).await.unwrap();
        assert_eq!(result["count"], 2);
        let rows = result["contradictions"].as_array().unwrap();
        assert_eq!(rows[0]["id"], "c-1");
        assert_eq!(rows[0]["contradiction_type"], "logical");
        assert_eq!(rows[1]["id"], "c-2");
    }

    #[tokio::test]
    async fn contradictions_without_belief_id_uses_list_recent() {
        let store = Arc::new(StubContradictionStore {
            for_belief_rows: std::sync::Mutex::new(vec![]),
            list_recent_rows: std::sync::Mutex::new(vec![sample_contradiction(
                "c-recent", "b-1", "b-2",
            )]),
        });
        let tool = BeliefTool::new(
            None,
            Some(store as Arc<dyn zbot_stores_traits::BeliefContradictionStore>),
        );
        let ctx = ctx_with_session("sess-contradictions-recent");

        let args = json!({ "action": "contradictions", "limit": 5 });
        let result = tool.execute(Arc::new(ctx), args).await.unwrap();
        assert_eq!(result["count"], 1);
        let rows = result["contradictions"].as_array().unwrap();
        assert_eq!(rows[0]["id"], "c-recent");
    }

    #[tokio::test]
    async fn contradictions_errors_when_store_missing() {
        let tool = BeliefTool::new(None, None);
        let ctx = ctx_with_session("sess-contradictions-missing");
        let args = json!({ "action": "contradictions" });
        let err = tool.execute(Arc::new(ctx), args).await;
        assert!(err.is_err(), "missing store must surface as a tool error");
    }

    #[tokio::test]
    async fn belief_errors_when_store_missing() {
        let tool = BeliefTool::new(None, None);
        let ctx = ctx_with_session("sess-belief-missing");
        let args = json!({ "action": "belief", "subject": "user.x" });
        let err = tool.execute(Arc::new(ctx), args).await;
        assert!(err.is_err(), "missing store must surface as a tool error");
    }
}
