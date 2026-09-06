//! Capability inspection services. Dependencies are assembled per request so
//! changes to optional providers remain visible. No parent application state
//! or execution orchestrator is retained here; discovery uses metadata only.

use agent_primitives::connectors::{CapabilityInfo, ConnectorInfo, ResourceInfo};
use agent_runtime::{
    ContextActorKind, ContextCapability, ContextCapabilityCatalog, ContextCapabilityHealth,
    ContextCapabilityKind, ContextCostHint, ContextLatencyHint, ContextRiskLevel,
    ContextSideEffects,
};

use gateway_services::WardUsage;
use std::{collections::BTreeSet, sync::Arc};

pub(super) struct ToolCatalog {
    pub(super) paths: gateway_services::SharedVaultPaths,
    pub(super) state_service:
        Arc<execution_state::StateService<zbot_runtime_sqlite::DatabaseManager>>,
    pub(super) messages: Arc<dyn zbot_conversation::MessageStore>,
    pub(super) model_registry: Arc<gateway_services::ModelRegistry>,
    pub(super) memory_store: Option<Arc<dyn zbot_stores::MemoryFactStore>>,
    pub(super) connector_provider: Option<Arc<dyn agent_primitives::ConnectorResourceProvider>>,
    pub(super) kg_store: Option<Arc<dyn zbot_stores::KnowledgeGraphStore>>,
    pub(super) ingestion_queue: Option<Arc<gateway_execution::ingest::IngestionQueue>>,
    pub(super) kg_episode_store: Option<Arc<dyn zbot_stores_traits::KgEpisodeStore>>,
    pub(super) goal_store: Option<Arc<dyn zbot_stores_traits::GoalStore>>,
    pub(super) procedure_store: Option<Arc<dyn zbot_stores_traits::ProcedureStore>>,
}
impl ToolCatalog {
    pub(super) fn build(
        &self,
        actor_kind: gateway_execution::invoke::RuntimeActorKind,
        tool_settings: agent_tools::ToolSettings,
        session_id: Option<String>,
        agent_id: Option<String>,
    ) -> ContextCapabilityCatalog {
        let mut builder = gateway_execution::invoke::ExecutorBuilder::new(
            self.paths.vault_dir().clone(),
            tool_settings,
        )
        .with_actor_kind(actor_kind)
        .with_state_service(self.state_service.clone())
        .with_message_store(self.messages.clone())
        .with_model_registry(self.model_registry.clone());

        if let Some(store) = &self.memory_store {
            builder = builder.with_fact_store(store.clone());
        }
        if let Some(provider) = self.connector_provider.clone() {
            builder = builder.with_connector_provider(provider);
        }
        if let Some(store) = &self.kg_store {
            builder = builder.with_kg_store(store.clone());
        }
        if let (Some(queue), Some(kg_store), Some(kg_episode_store)) = (
            self.ingestion_queue.as_ref(),
            self.kg_store.as_ref(),
            self.kg_episode_store.as_ref(),
        ) {
            let adapter = Arc::new(
                gateway_execution::invoke::ingest_adapter::IngestionAdapter::new(
                    queue.clone(),
                    kg_episode_store.clone(),
                    kg_store.clone(),
                ),
            );
            builder = builder.with_ingestion_adapter(adapter);
        }
        if let Some(store) = &self.goal_store {
            let adapter = Arc::new(gateway_execution::invoke::goal_adapter::GoalAdapter::new(
                store.clone(),
            ));
            builder = builder.with_goal_adapter(adapter);
        }
        if let Some(store) = &self.procedure_store {
            builder = builder.with_procedure_store(store.clone());
        }

        let ward_usage = Arc::new(
            gateway_execution::invoke::ward_usage_adapter::WardUsageAdapter::new(Arc::new(
                WardUsage::new(self.paths.wards_dir()),
            )),
        );
        builder = builder
            .with_ward_usage(ward_usage)
            .with_steering_registry(Arc::new(agent_runtime::SteeringRegistry::new()))
            .with_agent_result_bus(Arc::new(
                gateway_execution::agent_pool::AgentResultBus::new(),
            ));

        builder.build_context_capability_catalog(session_id, agent_id)
    }
}

pub(super) struct ResourceCatalog {
    pub(super) local: LocalProviderStatus,
    pub(super) mcp_service: Arc<gateway_services::McpService>,
    pub(super) connector_provider: Option<Arc<dyn agent_primitives::ConnectorResourceProvider>>,
}

impl ResourceCatalog {
    pub(super) async fn enrich(
        &self,
        mut catalog: ContextCapabilityCatalog,
    ) -> ContextCapabilityCatalog {
        let mut resource_capabilities = local_context_provider_capabilities(self.local);

        match self.mcp_service.list_summaries() {
            Ok(summaries) => resource_capabilities.extend(mcp_catalog_capabilities(summaries)),
            Err(error) => {
                tracing::warn!(%error, "Failed to enrich capability catalog with MCP metadata");
            }
        }

        if let Some(provider) = self.connector_provider.clone() {
            match provider.list_connectors().await {
                Ok(connectors) => {
                    resource_capabilities.extend(connector_catalog_capabilities(connectors));
                }
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "Failed to enrich capability catalog with connector resource metadata"
                    );
                }
            }
        }

        append_actor_visible_capabilities(&mut catalog, resource_capabilities);
        catalog
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LocalProviderStatus {
    pub(super) memory_store: bool,
    pub(super) kg_store: bool,
    pub(super) ingestion_queue: bool,
    pub(super) compaction_store: bool,
    pub(super) belief_store: bool,
}

fn append_actor_visible_capabilities(
    catalog: &mut ContextCapabilityCatalog,
    capabilities: Vec<ContextCapability>,
) {
    let mut seen: BTreeSet<String> = catalog
        .capabilities
        .iter()
        .map(|capability| capability.id.clone())
        .collect();

    for capability in capabilities {
        if !capability.actor_policy.contains(&catalog.actor_kind) {
            continue;
        }
        if seen.insert(capability.id.clone()) {
            catalog.capabilities.push(capability);
        }
    }
}

fn local_context_provider_capabilities(status: LocalProviderStatus) -> Vec<ContextCapability> {
    let read_policy = all_actor_policy();
    let mut capabilities = Vec::new();

    capabilities.push(ContextCapability {
        id: "memory:facts".to_string(),
        kind: ContextCapabilityKind::Resource,
        display_name: "Memory Facts".to_string(),
        description: "Semantic memory fact resource exposed through the selected memory provider."
            .to_string(),
        actor_policy: read_policy.clone(),
        risk_level: ContextRiskLevel::Low,
        side_effects: ContextSideEffects::ReadExternal,
        input_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50 }
            }
        })),
        output_schema: None,
        resource_uri_template: Some("zbot://memory/facts{?query,limit}".to_string()),
        cost_hint: Some(ContextCostHint::Cheap),
        latency_hint: Some(ContextLatencyHint::Fast),
        token_hint: Some(500),
        health: health_for(status.memory_store),
        owner_crate: Some("zbot-stores-traits".to_string()),
        audit_policy: Some("memory_read_audit".to_string()),
        default_visible: false,
        visibility_policy: "resource_catalog_only".to_string(),
        split_target: Some("context:memory_recall".to_string()),
    });

    capabilities.push(ContextCapability {
        id: "memory:recall_unified".to_string(),
        kind: ContextCapabilityKind::ContextGraph,
        display_name: "Unified Recall".to_string(),
        description: "Context packet provider for semantic memory, procedures, wiki, and graph-enriched recall."
            .to_string(),
        actor_policy: read_policy.clone(),
        risk_level: ContextRiskLevel::Low,
        side_effects: ContextSideEffects::ReadExternal,
        input_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string" },
                "session_id": { "type": "string" },
                "agent_id": { "type": "string" }
            },
            "required": ["prompt"]
        })),
        output_schema: None,
        resource_uri_template: Some("zbot://context/recall/unified{?prompt,session_id,agent_id}".to_string()),
        cost_hint: Some(ContextCostHint::Moderate),
        latency_hint: Some(ContextLatencyHint::Slow),
        token_hint: Some(1_500),
        health: health_for(status.memory_store),
        owner_crate: Some("gateway-memory".to_string()),
        audit_policy: Some("context_packet_trace".to_string()),
        default_visible: false,
        visibility_policy: "context_provider_only".to_string(),
        split_target: Some("context_packet:recall_unified".to_string()),
    });

    capabilities.push(ContextCapability {
        id: "knowledge_graph:entities".to_string(),
        kind: ContextCapabilityKind::ContextGraph,
        display_name: "Knowledge Graph Entities".to_string(),
        description: "Entity and relationship graph resource exposed through the selected knowledge provider."
            .to_string(),
        actor_policy: read_policy.clone(),
        risk_level: ContextRiskLevel::Low,
        side_effects: ContextSideEffects::ReadExternal,
        input_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 500 }
            }
        })),
        output_schema: None,
        resource_uri_template: Some("zbot://knowledge-graph/entities{?query,limit}".to_string()),
        cost_hint: Some(ContextCostHint::Cheap),
        latency_hint: Some(ContextLatencyHint::Fast),
        token_hint: Some(900),
        health: health_for(status.kg_store),
        owner_crate: Some("zbot-stores-traits".to_string()),
        audit_policy: Some("graph_read_audit".to_string()),
        default_visible: false,
        visibility_policy: "resource_catalog_only".to_string(),
        split_target: Some("context_graph:knowledge_graph".to_string()),
    });

    capabilities.push(ContextCapability {
        id: "knowledge_graph:ingestion_queue".to_string(),
        kind: ContextCapabilityKind::Catalog,
        display_name: "Knowledge Graph Ingestion Queue".to_string(),
        description: "Ingestion queue health and intake boundary for evidence destined for the knowledge graph."
            .to_string(),
        actor_policy: vec![ContextActorKind::Root, ContextActorKind::WardAgent],
        risk_level: ContextRiskLevel::Moderate,
        side_effects: ContextSideEffects::WriteLocal,
        input_schema: None,
        output_schema: None,
        resource_uri_template: Some("zbot://knowledge-graph/ingestion-queue".to_string()),
        cost_hint: Some(ContextCostHint::Cheap),
        latency_hint: Some(ContextLatencyHint::Background),
        token_hint: Some(150),
        health: health_for(status.ingestion_queue),
        owner_crate: Some("gateway-execution".to_string()),
        audit_policy: Some("evidence_intake_audit".to_string()),
        default_visible: false,
        visibility_policy: "catalog_only".to_string(),
        split_target: Some("tool:ingest".to_string()),
    });

    capabilities.push(ContextCapability {
        id: "memory:compaction".to_string(),
        kind: ContextCapabilityKind::Catalog,
        display_name: "Memory Compaction".to_string(),
        description:
            "Compaction audit resource for sleep-time memory maintenance and consolidation."
                .to_string(),
        actor_policy: vec![ContextActorKind::Root, ContextActorKind::WardAgent],
        risk_level: ContextRiskLevel::Low,
        side_effects: ContextSideEffects::ReadExternal,
        input_schema: None,
        output_schema: None,
        resource_uri_template: Some("zbot://memory/compaction".to_string()),
        cost_hint: Some(ContextCostHint::Free),
        latency_hint: Some(ContextLatencyHint::Fast),
        token_hint: Some(200),
        health: health_for(status.compaction_store),
        owner_crate: Some("gateway-memory".to_string()),
        audit_policy: Some("compaction_read_audit".to_string()),
        default_visible: false,
        visibility_policy: "catalog_only".to_string(),
        split_target: Some("context:memory_compaction".to_string()),
    });

    capabilities.push(ContextCapability {
        id: "memory:belief_network".to_string(),
        kind: ContextCapabilityKind::ContextGraph,
        display_name: "Belief Network".to_string(),
        description: "Belief and contradiction graph resource when the belief network is enabled."
            .to_string(),
        actor_policy: read_policy,
        risk_level: ContextRiskLevel::Low,
        side_effects: ContextSideEffects::ReadExternal,
        input_schema: None,
        output_schema: None,
        resource_uri_template: Some("zbot://memory/belief-network".to_string()),
        cost_hint: Some(ContextCostHint::Cheap),
        latency_hint: Some(ContextLatencyHint::Fast),
        token_hint: Some(700),
        health: health_for(status.belief_store),
        owner_crate: Some("gateway-memory".to_string()),
        audit_policy: Some("belief_read_audit".to_string()),
        default_visible: false,
        visibility_policy: "resource_catalog_only".to_string(),
        split_target: Some("context_graph:belief_network".to_string()),
    });

    capabilities
}

fn mcp_catalog_capabilities(
    summaries: Vec<gateway_services::mcp::McpServerSummary>,
) -> Vec<ContextCapability> {
    summaries
        .into_iter()
        .map(|summary| {
            let id_component = catalog_id_component(&summary.id);
            let mut description = summary.description;
            if let Some(auth_status) = summary.auth_status {
                if !auth_status.is_empty() {
                    description = format!("{description} Auth status: {auth_status}.");
                }
            }

            ContextCapability {
                id: format!("mcp:{id_component}"),
                kind: ContextCapabilityKind::Catalog,
                display_name: format!("MCP: {}", summary.name),
                description,
                actor_policy: all_actor_policy(),
                risk_level: ContextRiskLevel::Low,
                side_effects: ContextSideEffects::None,
                input_schema: None,
                output_schema: None,
                resource_uri_template: Some(format!("zbot://mcp/{id_component}")),
                cost_hint: Some(ContextCostHint::Free),
                latency_hint: Some(ContextLatencyHint::Local),
                token_hint: Some(120),
                health: if summary.enabled {
                    ContextCapabilityHealth::Available
                } else {
                    ContextCapabilityHealth::Disabled
                },
                owner_crate: Some("gateway-services".to_string()),
                audit_policy: Some("catalog_read".to_string()),
                default_visible: false,
                visibility_policy: "catalog_only".to_string(),
                split_target: Some(format!("mcp_transport:{}", summary.transport_type)),
            }
        })
        .collect()
}

fn connector_catalog_capabilities(connectors: Vec<ConnectorInfo>) -> Vec<ContextCapability> {
    let mut capabilities = Vec::new();

    for connector in connectors {
        let connector_id = catalog_id_component(&connector.id);
        let connector_name = connector.name;
        for resource in connector.resources {
            capabilities.push(connector_resource_capability(
                &connector_name,
                &connector_id,
                resource,
            ));
        }
        for capability in connector.capabilities {
            capabilities.push(connector_action_capability(
                &connector_name,
                &connector_id,
                capability,
            ));
        }
    }

    capabilities
}

fn connector_resource_capability(
    connector_name: &str,
    connector_id: &str,
    resource: ResourceInfo,
) -> ContextCapability {
    let resource_id = catalog_id_component(&resource.name);
    let method = resource.method.to_uppercase();
    let description = resource.description.unwrap_or_else(|| {
        format!(
            "Read {} from connector {} via {}.",
            resource.name, connector_name, method
        )
    });

    ContextCapability {
        id: format!("connector:{connector_id}:resource:{resource_id}"),
        kind: ContextCapabilityKind::Resource,
        display_name: format!("{}: {}", connector_name, resource.name),
        description,
        actor_policy: connector_actor_policy(),
        risk_level: ContextRiskLevel::Moderate,
        side_effects: ContextSideEffects::ReadExternal,
        input_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "params": {
                    "type": "object",
                    "additionalProperties": { "type": "string" }
                }
            }
        })),
        output_schema: None,
        resource_uri_template: Some(format!(
            "zbot://connectors/{connector_id}/resources/{resource_id}{{?params}}"
        )),
        cost_hint: Some(ContextCostHint::Moderate),
        latency_hint: Some(ContextLatencyHint::Slow),
        token_hint: Some(800),
        health: ContextCapabilityHealth::Available,
        owner_crate: Some("gateway-connectors".to_string()),
        audit_policy: Some("connector_resource_read_audit".to_string()),
        default_visible: false,
        visibility_policy: "resource_catalog_only".to_string(),
        split_target: Some("tool:query_resource?action=query".to_string()),
    }
}

fn connector_action_capability(
    connector_name: &str,
    connector_id: &str,
    capability: CapabilityInfo,
) -> ContextCapability {
    let capability_id = catalog_id_component(&capability.name);
    let schema = if capability.schema.is_null() {
        None
    } else {
        Some(capability.schema)
    };

    ContextCapability {
        id: format!("connector:{connector_id}:capability:{capability_id}"),
        kind: ContextCapabilityKind::Tool,
        display_name: format!("{}: {}", connector_name, capability.name),
        description: capability.description.unwrap_or_else(|| {
            format!(
                "Invoke connector capability {} on {}.",
                capability.name, connector_name
            )
        }),
        actor_policy: connector_actor_policy(),
        risk_level: ContextRiskLevel::Moderate,
        side_effects: ContextSideEffects::WriteExternal,
        input_schema: schema,
        output_schema: None,
        resource_uri_template: Some(format!(
            "zbot://connectors/{connector_id}/capabilities/{capability_id}"
        )),
        cost_hint: Some(ContextCostHint::Moderate),
        latency_hint: Some(ContextLatencyHint::Slow),
        token_hint: Some(900),
        health: ContextCapabilityHealth::Available,
        owner_crate: Some("gateway-connectors".to_string()),
        audit_policy: Some("connector_capability_invoke_audit".to_string()),
        default_visible: false,
        visibility_policy: "resource_catalog_only".to_string(),
        split_target: Some("tool:query_resource?action=invoke".to_string()),
    }
}

fn all_actor_policy() -> Vec<ContextActorKind> {
    vec![
        ContextActorKind::Root,
        ContextActorKind::DelegatedExecutor,
        ContextActorKind::DelegatedReviewer,
        ContextActorKind::WardAgent,
    ]
}

fn connector_actor_policy() -> Vec<ContextActorKind> {
    vec![ContextActorKind::Root, ContextActorKind::WardAgent]
}

fn health_for(enabled: bool) -> ContextCapabilityHealth {
    if enabled {
        ContextCapabilityHealth::Available
    } else {
        ContextCapabilityHealth::Disabled
    }
}

fn catalog_id_component(raw: &str) -> String {
    let mut normalized = String::with_capacity(raw.len());
    let mut last_was_separator = false;
    for ch in raw.chars() {
        let next = if ch.is_ascii_alphanumeric() {
            last_was_separator = false;
            Some(ch.to_ascii_lowercase())
        } else if !last_was_separator {
            last_was_separator = true;
            Some('_')
        } else {
            None
        };
        if let Some(ch) = next {
            normalized.push(ch);
        }
    }
    let normalized = normalized.trim_matches('_');
    if normalized.is_empty() {
        "unnamed".to_string()
    } else {
        normalized.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_goal(catalog: &ContextCapabilityCatalog) -> bool {
        catalog
            .capabilities
            .iter()
            .any(|capability| capability.id == "goal")
    }

    #[tokio::test]
    async fn fallback_catalog_observes_goal_store_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = crate::state::AppState::minimal(dir.path().to_path_buf());
        let goals = state
            .goal_store
            .take()
            .expect("minimal state wires Engram goals");
        let actor = gateway_execution::invoke::RuntimeActorKind::Root;
        assert!(!has_goal(
            &state.context_capability_catalog(actor, None, None)
        ));
        state.goal_store = Some(goals);
        assert!(has_goal(
            &state.context_capability_catalog(actor, None, None)
        ));
        state.goal_store = None;
        assert!(!has_goal(
            &state.context_capability_catalog(actor, None, None)
        ));
    }

    #[tokio::test]
    async fn catalog_prefers_installed_runner_over_fallback_stores() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = crate::state::AppState::minimal(dir.path().to_path_buf());
        assert!(state.goal_store.is_some());
        let actor = gateway_execution::invoke::RuntimeActorKind::Root;
        assert!(has_goal(
            &state.context_capability_catalog(actor, None, None)
        ));
        // The runner has no goal adapter; only the fallback AppState has one.
        state.runtime = Arc::new(crate::services::RuntimeService::with_runner(
            state.event_bus.clone(),
            state.agents.clone(),
            state.provider_service.clone(),
            state.paths.clone(),
            state.messages.clone(),
            state.session_meta.clone(),
            state.checkpoints.clone(),
            state.mcp_service.clone(),
            state.skills.clone(),
            state.log_service.clone(),
            state.state_service.clone(),
        ));
        let catalog =
            state.context_capability_catalog(actor, Some("session".into()), Some("root".into()));
        assert!(
            !has_goal(&catalog),
            "installed runner owns the tool inventory"
        );
        assert_eq!(catalog.session_id.as_deref(), Some("session"));
        assert_eq!(catalog.agent_id.as_deref(), Some("root"));
    }

    struct MetadataOnlyProvider {
        fail: bool,
        lists: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl agent_primitives::ConnectorResourceProvider for MetadataOnlyProvider {
        async fn list_connectors(&self) -> Result<Vec<ConnectorInfo>, String> {
            self.lists.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.fail {
                Err("metadata unavailable".into())
            } else {
                Ok(vec![ConnectorInfo {
                    id: "mail".into(),
                    name: "Mail".into(),
                    resources: vec![ResourceInfo {
                        name: "inbox".into(),
                        uri: "https://private.invalid/inbox".into(),
                        method: "GET".into(),
                        description: None,
                    }],
                    capabilities: vec![],
                }])
            }
        }

        async fn query_resource(
            &self,
            _: &str,
            _: &str,
            _: Option<std::collections::HashMap<String, String>>,
        ) -> Result<serde_json::Value, String> {
            panic!("catalog inspection must not query resources")
        }

        async fn invoke_capability(
            &self,
            _: &str,
            _: &str,
            _: serde_json::Value,
            _: &str,
            _: &str,
        ) -> Result<serde_json::Value, String> {
            panic!("catalog inspection must not invoke capabilities")
        }
    }

    #[tokio::test]
    async fn enrichment_is_metadata_only_and_survives_provider_failure() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::state::AppState::minimal(dir.path().to_path_buf());
        for fail in [false, true] {
            let provider = Arc::new(MetadataOnlyProvider {
                fail,
                lists: std::sync::atomic::AtomicUsize::new(0),
            });
            let base = state.context_capability_catalog(
                gateway_execution::invoke::RuntimeActorKind::Root,
                None,
                None,
            );
            let original_capabilities = serde_json::to_value(&base.capabilities).unwrap();
            let base_len = base.capabilities.len();
            let catalog = ResourceCatalog {
                local: LocalProviderStatus {
                    memory_store: false,
                    kg_store: false,
                    ingestion_queue: false,
                    compaction_store: false,
                    belief_store: false,
                },
                mcp_service: state.mcp_service.clone(),
                connector_provider: Some(provider.clone()),
            }
            .enrich(base)
            .await;
            assert_eq!(provider.lists.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert_eq!(
                serde_json::to_value(&catalog.capabilities[..base_len]).unwrap(),
                original_capabilities
            );
            assert_eq!(
                catalog
                    .capabilities
                    .iter()
                    .any(|capability| capability.id == "connector:mail:resource:inbox"),
                !fail
            );
            assert_eq!(
                catalog
                    .capabilities
                    .iter()
                    .find(|c| c.id == "memory:facts")
                    .unwrap()
                    .health,
                ContextCapabilityHealth::Disabled
            );
            assert!(!serde_json::to_string(&catalog)
                .unwrap()
                .contains("private.invalid"));
        }
    }

    #[test]
    fn local_context_provider_catalog_entries_report_resource_health() {
        let capabilities = local_context_provider_capabilities(LocalProviderStatus {
            memory_store: true,
            kg_store: true,
            ingestion_queue: false,
            compaction_store: false,
            belief_store: false,
        });

        let memory = capabilities
            .iter()
            .find(|capability| capability.id == "memory:facts")
            .expect("memory resource");
        assert_eq!(memory.kind, ContextCapabilityKind::Resource);
        assert_eq!(memory.health, ContextCapabilityHealth::Available);
        assert!(!memory.default_visible);
        assert!(memory
            .actor_policy
            .contains(&ContextActorKind::DelegatedReviewer));

        let graph = capabilities
            .iter()
            .find(|capability| capability.id == "knowledge_graph:entities")
            .expect("knowledge graph resource");
        assert_eq!(graph.kind, ContextCapabilityKind::ContextGraph);
        assert_eq!(
            graph.resource_uri_template.as_deref(),
            Some("zbot://knowledge-graph/entities{?query,limit}")
        );

        let ingestion = capabilities
            .iter()
            .find(|capability| capability.id == "knowledge_graph:ingestion_queue")
            .expect("ingestion queue resource");
        assert_eq!(ingestion.health, ContextCapabilityHealth::Disabled);
        assert_eq!(ingestion.side_effects, ContextSideEffects::WriteLocal);
    }

    #[test]
    fn connector_catalog_entries_use_logical_resource_uris() {
        let connectors = vec![ConnectorInfo {
            id: "Local Mail".to_string(),
            name: "Local Mail".to_string(),
            resources: vec![ResourceInfo {
                name: "Recent Messages".to_string(),
                uri: "http://127.0.0.1:9999/private/messages".to_string(),
                method: "GET".to_string(),
                description: Some("Read recent messages.".to_string()),
            }],
            capabilities: vec![CapabilityInfo {
                name: "Send Message".to_string(),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "to": { "type": "string" },
                        "body": { "type": "string" }
                    }
                }),
                description: None,
            }],
        }];

        let capabilities = connector_catalog_capabilities(connectors);
        let resource = capabilities
            .iter()
            .find(|capability| capability.id == "connector:local_mail:resource:recent_messages")
            .expect("connector resource");
        assert_eq!(resource.kind, ContextCapabilityKind::Resource);
        assert_eq!(resource.side_effects, ContextSideEffects::ReadExternal);
        assert_eq!(
            resource.resource_uri_template.as_deref(),
            Some("zbot://connectors/local_mail/resources/recent_messages{?params}")
        );
        assert!(!serde_json::to_string(resource)
            .unwrap()
            .contains("127.0.0.1"));

        let action = capabilities
            .iter()
            .find(|capability| capability.id == "connector:local_mail:capability:send_message")
            .expect("connector action");
        assert_eq!(action.side_effects, ContextSideEffects::WriteExternal);
        assert_eq!(
            action.actor_policy,
            vec![ContextActorKind::Root, ContextActorKind::WardAgent]
        );
    }

    #[test]
    fn resource_catalog_filters_entries_by_actor_and_dedupes() {
        let mut catalog = ContextCapabilityCatalog {
            version: "test".to_string(),
            actor_kind: ContextActorKind::DelegatedReviewer,
            session_id: None,
            agent_id: None,
            capabilities: vec![ContextCapability {
                id: "memory:facts".to_string(),
                kind: ContextCapabilityKind::Tool,
                display_name: "Existing".to_string(),
                description: "Existing entry wins.".to_string(),
                actor_policy: vec![ContextActorKind::DelegatedReviewer],
                risk_level: ContextRiskLevel::Low,
                side_effects: ContextSideEffects::None,
                input_schema: None,
                output_schema: None,
                resource_uri_template: None,
                cost_hint: None,
                latency_hint: None,
                token_hint: None,
                health: ContextCapabilityHealth::Available,
                owner_crate: None,
                audit_policy: None,
                default_visible: true,
                visibility_policy: "default_visible".to_string(),
                split_target: None,
            }],
        };

        let mut extra = local_context_provider_capabilities(LocalProviderStatus {
            memory_store: true,
            kg_store: true,
            ingestion_queue: true,
            compaction_store: true,
            belief_store: true,
        });
        extra.extend(connector_catalog_capabilities(vec![ConnectorInfo {
            id: "crm".to_string(),
            name: "CRM".to_string(),
            resources: vec![ResourceInfo {
                name: "contacts".to_string(),
                uri: "https://crm.example/resources/contacts".to_string(),
                method: "GET".to_string(),
                description: None,
            }],
            capabilities: vec![],
        }]));

        append_actor_visible_capabilities(&mut catalog, extra);
        let ids: BTreeSet<_> = catalog
            .capabilities
            .iter()
            .map(|capability| capability.id.as_str())
            .collect();

        assert_eq!(
            catalog
                .capabilities
                .iter()
                .filter(|capability| capability.id == "memory:facts")
                .count(),
            1
        );
        assert!(ids.contains("knowledge_graph:entities"));
        assert!(!ids.contains("knowledge_graph:ingestion_queue"));
        assert!(!ids.contains("connector:crm:resource:contacts"));
    }
}
