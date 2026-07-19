//! Production `AggregateEntityLlm` adapter — wires the
//! `HierarchyBuilder` trait surface to the gateway's `MemoryLlmFactory`.
//!
//! Mirrors `LlmBeliefSynthesizer` (Belief Network B-1): a thin struct
//! holding an Arc<dyn MemoryLlmFactory>, with prompt construction +
//! response parsing pulled into free helpers so they can be unit-tested
//! without spinning up a real LLM.
//!
//! Two LLM calls power the hierarchy builder per cycle:
//!
//!   1. `synthesize_aggregate(members)` — given N (typically ~20) entity
//!      contexts, return a `{name, description}` for the layer-N+1
//!      aggregate entity that subsumes them.
//!   2. `synthesize_relation(agg_a, agg_b, λ)` — given two aggregate
//!      names + their connectivity strength, return a short verb-phrase
//!      that names the inter-cluster relationship between them.
//!
//! Both responses are strict JSON; parse failures fall back to safe
//! defaults in the caller (`HierarchyBuilder::write_inter_cluster_pair`
//! drops to `"related-via"`; cluster synthesis errors are counted in
//! `stats.errors` and the cycle continues).

use std::sync::Arc;

use agent_runtime::llm::ChatMessage;
use async_trait::async_trait;
use serde::Deserialize;

use crate::sleep::hierarchy_builder::{
    AggregateEntityLlm, AggregateMemberContext, AggregateResponse,
};
use crate::util::parse_llm_json;
use crate::{CachedLlmClient, LlmClientConfig, MemoryLlmFactory};

/// Default temperature for both calls. Zero so summaries are
/// reproducible — clustering is already pinned by a seeded K-means,
/// the LLM should be the same.
const TEMPERATURE: f64 = 0.0;

/// Max output tokens for the aggregate-synth call. ~256 covers a name
/// (~5 words) + description (~30 words) plus JSON overhead.
const MAX_TOKENS_AGGREGATE: u32 = 256;

/// Max output tokens for the relation-synth call. Very small — the LLM
/// just returns a short verb-phrase.
const MAX_TOKENS_RELATION: u32 = 64;

// ---------------------------------------------------------------------------
// Response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct AggregateLlmResponse {
    name: String,
    description: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RelationLlmResponse {
    relation_type: String,
}

// ---------------------------------------------------------------------------
// LlmAggregateEntity
// ---------------------------------------------------------------------------

/// Production adapter — implements [`AggregateEntityLlm`] by routing
/// each call through `MemoryLlmFactory::build_client`. Two cached
/// clients because the two methods use different `max_tokens`.
pub struct LlmAggregateEntity {
    aggregate_client: CachedLlmClient,
    relation_client: CachedLlmClient,
}

impl LlmAggregateEntity {
    pub fn new(factory: Arc<dyn MemoryLlmFactory>) -> Self {
        Self {
            aggregate_client: CachedLlmClient::new(
                factory.clone(),
                LlmClientConfig::new(TEMPERATURE, MAX_TOKENS_AGGREGATE),
            ),
            relation_client: CachedLlmClient::new(
                factory,
                LlmClientConfig::new(TEMPERATURE, MAX_TOKENS_RELATION),
            ),
        }
    }

    /// Build the prompt body for the aggregate-synth call. Pulled out
    /// so tests can assert it without spinning up a real LLM. The
    /// shape is "you receive a list, return JSON" — same idiom as
    /// `LlmBeliefSynthesizer::build_prompt`.
    fn build_aggregate_prompt(
        members: &[AggregateMemberContext],
        prior_names: &[String],
    ) -> String {
        let formatted = members
            .iter()
            .map(|m| match m.description.as_deref() {
                Some(desc) if !desc.is_empty() => format!("- {} — {}", m.name, desc),
                _ => format!("- {}", m.name),
            })
            .collect::<Vec<_>>()
            .join("\n");
        // Only inject the avoid-list when there's something to avoid.
        // Empty list means "first cluster of the cycle" — keep the
        // prompt byte-for-byte identical to pre-avoid behaviour so
        // the first call doesn't pay for an empty section.
        let avoid_block = if prior_names.is_empty() {
            String::new()
        } else {
            let list = prior_names
                .iter()
                .map(|n| format!("  - {n}"))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "\n\
                 \n\
                 IMPORTANT — these aggregate names are already in use \
                 this cycle. Pick a DIFFERENT name (the underlying \
                 entities may be thematically similar, but the agent \
                 needs distinct labels to disambiguate at recall time):\n\
                 {list}"
            )
        };
        format!(
            "You are summarising a cluster of related entities from a \
             knowledge graph. Produce a single aggregate entity that \
             represents the whole cluster.\n\
             \n\
             Cluster members ({} entities):\n\
             {formatted}\
             {avoid_block}\n\
             \n\
             Output JSON only, no prose:\n\
             {{\"name\": \"<2-5 word concept name>\", \
             \"description\": \"<one sentence explaining what these entities have in common>\"}}\n\
             \n\
             Rules:\n\
             - Name should be a concise concept (e.g. \"infrastructure-experiments\")\n\
             - Description should generalise — capture what the cluster IS, not list members\n\
             - Be terse: name ≤ 5 words, description ≤ 25 words",
            members.len()
        )
    }

    fn build_relation_prompt(agg_a_name: &str, agg_b_name: &str, lambda: usize) -> String {
        format!(
            "Two aggregate concepts from a knowledge graph share \
             {lambda} underlying relationships. Pick one short verb \
             phrase that names how they relate.\n\
             \n\
             Concept A: {agg_a_name}\n\
             Concept B: {agg_b_name}\n\
             Co-mentioned in {lambda} underlying edges.\n\
             \n\
             Output JSON only, no prose:\n\
             {{\"relation_type\": \"<short lowercase verb-phrase>\"}}\n\
             \n\
             Examples of good values: \"encompasses\", \"differs-from\", \
             \"depends-on\", \"contrasts-with\", \"shares-topic-with\".\n\
             \n\
             Rules:\n\
             - Lowercase, hyphen-separated, no spaces\n\
             - 1-3 words max\n\
             - Generic enough to apply to abstract concepts"
        )
    }
}

#[async_trait]
impl AggregateEntityLlm for LlmAggregateEntity {
    async fn synthesize_aggregate(
        &self,
        members: &[AggregateMemberContext],
        prior_names: &[String],
    ) -> Result<AggregateResponse, String> {
        let client = self.aggregate_client.get().await?;
        let prompt = Self::build_aggregate_prompt(members, prior_names);
        let messages = vec![
            ChatMessage::system("You return only valid JSON.".to_string()),
            ChatMessage::user(prompt),
        ];
        let response = client
            .chat(messages, None)
            .await
            .map_err(|e| format!("LLM aggregate call: {e}"))?;
        let parsed = parse_llm_json::<AggregateLlmResponse>(&response.content)?;
        Ok(AggregateResponse {
            name: parsed.name,
            description: parsed.description,
        })
    }

    async fn synthesize_relation(
        &self,
        agg_a_name: &str,
        agg_b_name: &str,
        lambda: usize,
    ) -> Result<String, String> {
        let client = self.relation_client.get().await?;
        let prompt = Self::build_relation_prompt(agg_a_name, agg_b_name, lambda);
        let messages = vec![
            ChatMessage::system("You return only valid JSON.".to_string()),
            ChatMessage::user(prompt),
        ];
        let response = client
            .chat(messages, None)
            .await
            .map_err(|e| format!("LLM relation call: {e}"))?;
        let parsed = parse_llm_json::<RelationLlmResponse>(&response.content)?;
        Ok(parsed.relation_type)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use zbot_stores::types::EntityId;

    fn member(name: &str, description: Option<&str>) -> AggregateMemberContext {
        AggregateMemberContext {
            id: EntityId(format!("e-{name}")),
            name: name.to_string(),
            description: description.map(String::from),
        }
    }

    // ---- prompt-construction tests ----

    #[test]
    fn aggregate_prompt_includes_member_count_and_names() {
        let members = vec![
            member("home-server", Some("personal infra")),
            member("dotfiles", None),
            member("tmux-config", None),
        ];
        let prompt = LlmAggregateEntity::build_aggregate_prompt(&members, &[]);
        assert!(prompt.contains("(3 entities)"));
        assert!(prompt.contains("home-server"));
        assert!(prompt.contains("personal infra"));
        assert!(prompt.contains("dotfiles"));
        assert!(prompt.contains("Output JSON only"));
    }

    #[test]
    fn aggregate_prompt_omits_em_dash_when_description_missing() {
        let members = vec![member("only-name", None)];
        let prompt = LlmAggregateEntity::build_aggregate_prompt(&members, &[]);
        // The "- only-name" line must not have a trailing " — " or
        // empty description suffix.
        assert!(prompt.contains("- only-name\n") || prompt.contains("- only-name\n\n"));
        assert!(!prompt.contains("only-name — "));
    }

    #[test]
    fn aggregate_prompt_omits_avoid_block_when_prior_names_empty() {
        // First cluster of a cycle — no avoid list. The prompt must
        // not have the IMPORTANT/avoid section at all so it's
        // byte-for-byte the same as the pre-Option-B prompt and the
        // LLM provider's cache key stays stable.
        let prompt = LlmAggregateEntity::build_aggregate_prompt(&[member("x", None)], &[]);
        assert!(!prompt.contains("IMPORTANT"));
        assert!(!prompt.contains("already in use"));
    }

    #[test]
    fn aggregate_prompt_includes_avoid_list_when_prior_names_present() {
        // Subsequent clusters must see the names already chosen in
        // this cycle so the LLM avoids them. This is the regression
        // pin for the "two clusters labelled `agentic-system-components`"
        // bug surfaced in real-data smoke.
        let prompt = LlmAggregateEntity::build_aggregate_prompt(
            &[member("entity-x", None)],
            &[
                "agentic-system-components".to_string(),
                "multi-agent system cluster".to_string(),
            ],
        );
        assert!(prompt.contains("IMPORTANT"));
        assert!(prompt.contains("already in use"));
        assert!(prompt.contains("agentic-system-components"));
        assert!(prompt.contains("multi-agent system cluster"));
        // The instruction must explicitly tell the LLM to pick a
        // DIFFERENT name — otherwise it could surface the list as
        // examples rather than as a constraint.
        assert!(prompt.contains("DIFFERENT name"));
    }

    #[test]
    fn relation_prompt_includes_both_names_and_lambda() {
        let prompt =
            LlmAggregateEntity::build_relation_prompt("personal-projects", "work-projects", 7);
        assert!(prompt.contains("personal-projects"));
        assert!(prompt.contains("work-projects"));
        assert!(prompt.contains("7 underlying"));
        assert!(prompt.contains("\"relation_type\""));
    }

    // ---- response-parsing tests ----
    //
    // These exercise `parse_llm_json` against the response shapes
    // `LlmAggregateEntity` expects. Run separately from the LLM-call
    // path so a parse regression shows up immediately, without needing
    // a wired factory.

    #[test]
    fn aggregate_response_parses_clean_json() {
        let raw =
            r#"{"name": "personal-infra", "description": "Self-hosted tooling and configs."}"#;
        let parsed: AggregateLlmResponse = parse_llm_json(raw).unwrap();
        assert_eq!(parsed.name, "personal-infra");
        assert_eq!(parsed.description, "Self-hosted tooling and configs.");
    }

    #[test]
    fn aggregate_response_parses_json_in_markdown_fence() {
        // Some LLMs wrap JSON in ```json ... ``` even when told not to.
        // parse_llm_json should strip the fence.
        let raw = "```json\n{\"name\": \"x\", \"description\": \"y\"}\n```";
        let parsed: AggregateLlmResponse = parse_llm_json(raw).unwrap();
        assert_eq!(parsed.name, "x");
        assert_eq!(parsed.description, "y");
    }

    #[test]
    fn aggregate_response_rejects_malformed_json() {
        let raw = "not actually json";
        let result = parse_llm_json::<AggregateLlmResponse>(raw);
        assert!(result.is_err(), "malformed JSON must error");
    }

    #[test]
    fn relation_response_parses_clean_json() {
        let raw = r#"{"relation_type": "encompasses"}"#;
        let parsed: RelationLlmResponse = parse_llm_json(raw).unwrap();
        assert_eq!(parsed.relation_type, "encompasses");
    }

    #[test]
    fn relation_response_rejects_missing_field() {
        let raw = r#"{"foo": "bar"}"#;
        let result = parse_llm_json::<RelationLlmResponse>(raw);
        assert!(result.is_err(), "missing relation_type must error");
    }
}
