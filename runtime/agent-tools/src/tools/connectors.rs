// ============================================================================
// QUERY RESOURCE TOOL
// Discover and query data from external connector resources
// ============================================================================

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use agent_primitives::connectors::ConnectorResourceProvider;
use agent_primitives::{AgentError, Result, Tool, ToolContext, ToolPermissions};

use super::ingest::{EvidenceRecord, IngestionAccess};

/// Compatibility tool for querying resources and invoking capabilities on
/// external connectors.
///
/// Provides three actions:
/// - `list_resources`: Discover available connectors, resources (GET), and capabilities (POST)
/// - `query`: Fetch data from a connector resource URI
/// - `invoke`: Invoke a capability on a connector (e.g., send_message)
pub struct QueryResourceTool {
    provider: Arc<dyn ConnectorResourceProvider>,
    evidence_intake: Option<Arc<dyn IngestionAccess>>,
}

impl QueryResourceTool {
    /// Create a new QueryResourceTool with the given provider.
    pub fn new(provider: Arc<dyn ConnectorResourceProvider>) -> Self {
        Self {
            provider,
            evidence_intake: None,
        }
    }

    /// Wire the shared evidence-intake boundary used for explicit
    /// resource-read distillation.
    #[must_use]
    pub fn with_optional_evidence_intake(
        mut self,
        evidence_intake: Option<Arc<dyn IngestionAccess>>,
    ) -> Self {
        self.evidence_intake = evidence_intake;
        self
    }
}

/// Narrow model-facing tool for connector discovery and read-only resource
/// queries.
pub struct ConnectorResourceTool {
    provider: Arc<dyn ConnectorResourceProvider>,
    evidence_intake: Option<Arc<dyn IngestionAccess>>,
}

impl ConnectorResourceTool {
    /// Create a new ConnectorResourceTool with the given provider.
    #[must_use]
    pub fn new(provider: Arc<dyn ConnectorResourceProvider>) -> Self {
        Self {
            provider,
            evidence_intake: None,
        }
    }

    /// Wire the shared evidence-intake boundary used for explicit
    /// resource-read distillation.
    #[must_use]
    pub fn with_optional_evidence_intake(
        mut self,
        evidence_intake: Option<Arc<dyn IngestionAccess>>,
    ) -> Self {
        self.evidence_intake = evidence_intake;
        self
    }
}

/// Narrow model-facing tool for side-effecting connector capability calls.
pub struct ConnectorInvokeTool {
    provider: Arc<dyn ConnectorResourceProvider>,
}

impl ConnectorInvokeTool {
    /// Create a new ConnectorInvokeTool with the given provider.
    #[must_use]
    pub fn new(provider: Arc<dyn ConnectorResourceProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Tool for QueryResourceTool {
    fn name(&self) -> &str {
        "query_resource"
    }

    fn description(&self) -> &str {
        "Discover resources and capabilities on external connectors. \
        Actions: 'list_resources' (discover connectors, resources, and capabilities), \
        'query' (fetch data from a resource via GET), \
        'invoke' (call a capability like send_message via POST). \
        Use list_resources first to discover what's available."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list_resources", "query", "invoke"],
                    "description": "The operation to perform"
                },
                "connector_id": {
                    "type": "string",
                    "description": "Connector ID (required for 'query' and 'invoke')"
                },
                "resource": {
                    "type": "string",
                    "description": "Resource name to query (required for 'query')"
                },
                "capability": {
                    "type": "string",
                    "description": "Capability name to invoke, e.g. 'send_message' (required for 'invoke')"
                },
                "params": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Parameters for URI template expansion (for 'query')"
                },
                "payload": {
                    "type": "object",
                    "description": "Payload to send when invoking a capability (for 'invoke')"
                },
                "record_evidence": {
                    "type": "boolean",
                    "description": "For query action only. Defaults false. When true, explicitly records the successful resource read through the evidence intake boundary and enqueues the response for background extraction.",
                    "default": false
                },
                "source_id": {
                    "type": "string",
                    "description": "Optional provenance id for record_evidence=true. Defaults to '<connector_id>:<resource>:<tool_call_id>'."
                },
                "retention_policy": {
                    "type": "string",
                    "description": "Durable evidence retention policy for record_evidence=true. Defaults to 'durable'.",
                    "default": "durable"
                },
                "ontology_labels": {
                    "type": "array",
                    "description": "Optional host-selected dynamic ontology labels for record_evidence=true.",
                    "items": {"type": "string"}
                },
                "taxonomy_labels": {
                    "type": "array",
                    "description": "Optional host-selected SKOS/taxonomy labels for record_evidence=true.",
                    "items": {"type": "string"}
                }
            },
            "required": ["action"]
        }))
    }

    fn permissions(&self) -> ToolPermissions {
        ToolPermissions::moderate(vec!["network:http".into()])
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'action' parameter".to_string()))?;

        match action {
            "list_resources" => {
                list_connector_resources(
                    self.provider.as_ref(),
                    "connector_resource(action='query', ...) fetches data from resources. connector_invoke(connector_id='...', capability='...', payload={...}) calls a capability.",
                )
                .await
            }

            "query" => {
                query_connector_resource(
                    self.provider.as_ref(),
                    self.evidence_intake.as_deref(),
                    ctx.as_ref(),
                    &args,
                )
                .await
            }

            "invoke" => {
                invoke_connector_capability(self.provider.as_ref(), ctx.as_ref(), &args).await
            }

            _ => Err(AgentError::Tool(format!(
                "Unknown action '{}'. Valid: list_resources, query, invoke",
                action
            ))),
        }
    }
}

#[async_trait]
impl Tool for ConnectorResourceTool {
    fn name(&self) -> &str {
        "connector_resource"
    }

    fn description(&self) -> &str {
        "Discover external connector resources and fetch read-only connector resource data. Use connector_invoke for side-effecting connector actions."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "query"],
                    "description": "Use 'list' to discover connectors/resources/capabilities, or 'query' to fetch one read-only resource."
                },
                "connector_id": {
                    "type": "string",
                    "description": "Connector ID (required for 'query')"
                },
                "resource": {
                    "type": "string",
                    "description": "Resource name to query (required for 'query')"
                },
                "params": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Parameters for URI template expansion (for 'query')"
                },
                "record_evidence": {
                    "type": "boolean",
                    "description": "For query action only. Defaults false. When true, explicitly records the successful resource read through the evidence intake boundary and enqueues the response for background extraction.",
                    "default": false
                },
                "source_id": {
                    "type": "string",
                    "description": "Optional provenance id for record_evidence=true. Defaults to '<connector_id>:<resource>:<tool_call_id>'."
                },
                "retention_policy": {
                    "type": "string",
                    "description": "Durable evidence retention policy for record_evidence=true. Defaults to 'durable'.",
                    "default": "durable"
                },
                "ontology_labels": {
                    "type": "array",
                    "description": "Optional host-selected dynamic ontology labels for record_evidence=true.",
                    "items": {"type": "string"}
                },
                "taxonomy_labels": {
                    "type": "array",
                    "description": "Optional host-selected SKOS/taxonomy labels for record_evidence=true.",
                    "items": {"type": "string"}
                }
            },
            "required": ["action"]
        }))
    }

    fn permissions(&self) -> ToolPermissions {
        ToolPermissions::moderate(vec!["network:http".into()])
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'action' parameter".to_string()))?;

        match action {
            "list" => {
                list_connector_resources(
                    self.provider.as_ref(),
                    "connector_resource(action='query', connector_id='...', resource='...', params={...}) fetches read-only resource data. connector_invoke(connector_id='...', capability='...', payload={...}) calls a capability.",
                )
                .await
            }
            "query" => {
                query_connector_resource(
                    self.provider.as_ref(),
                    self.evidence_intake.as_deref(),
                    ctx.as_ref(),
                    &args,
                )
                .await
            }
            _ => Err(AgentError::Tool(format!(
                "Unknown action '{}'. Valid: list, query",
                action
            ))),
        }
    }
}

#[async_trait]
impl Tool for ConnectorInvokeTool {
    fn name(&self) -> &str {
        "connector_invoke"
    }

    fn description(&self) -> &str {
        "Invoke a side-effecting capability on an external connector. Use connector_resource for read-only connector discovery and resource queries."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "connector_id": {
                    "type": "string",
                    "description": "Connector ID."
                },
                "capability": {
                    "type": "string",
                    "description": "Capability name to invoke, e.g. 'send_message'."
                },
                "payload": {
                    "type": "object",
                    "description": "Payload to send when invoking the connector capability.",
                    "default": {}
                }
            },
            "required": ["connector_id", "capability"]
        }))
    }

    fn permissions(&self) -> ToolPermissions {
        ToolPermissions::moderate(vec!["network:http".into()])
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        invoke_connector_capability(self.provider.as_ref(), ctx.as_ref(), &args).await
    }
}

async fn list_connector_resources(
    provider: &dyn ConnectorResourceProvider,
    usage: &str,
) -> Result<Value> {
    let connectors = provider.list_connectors().await.map_err(AgentError::Tool)?;

    if connectors.is_empty() {
        return Ok(json!({
            "message": "No connectors configured. Add connectors via the web UI or API.",
            "connectors": []
        }));
    }

    let summary: Vec<Value> = connectors
        .iter()
        .map(|c| {
            let resources: Vec<Value> = c
                .resources
                .iter()
                .map(|r| {
                    json!({
                        "name": r.name,
                        "type": "resource",
                        "method": r.method,
                        "description": r.description,
                    })
                })
                .collect();

            let capabilities: Vec<Value> = c
                .capabilities
                .iter()
                .map(|cap| {
                    json!({
                        "name": cap.name,
                        "type": "capability",
                        "method": "POST",
                        "description": cap.description,
                        "schema": cap.schema,
                    })
                })
                .collect();

            json!({
                "connector_id": c.id,
                "name": c.name,
                "resources": resources,
                "capabilities": capabilities,
            })
        })
        .collect();

    Ok(json!({
        "connectors": summary,
        "usage": usage
    }))
}

async fn query_connector_resource(
    provider: &dyn ConnectorResourceProvider,
    evidence_intake: Option<&dyn IngestionAccess>,
    ctx: &dyn ToolContext,
    args: &Value,
) -> Result<Value> {
    let connector_id = args
        .get("connector_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            AgentError::Tool("Missing 'connector_id' parameter for query action".to_string())
        })?;

    let resource = args
        .get("resource")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            AgentError::Tool("Missing 'resource' parameter for query action".to_string())
        })?;

    let params: Option<HashMap<String, String>> = args.get("params").and_then(|v| {
        v.as_object().map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
    });

    let should_record = args
        .get("record_evidence")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let intake = if should_record {
        Some(evidence_intake.ok_or_else(|| {
            AgentError::Tool(
                "record_evidence=true requires evidence intake to be configured".to_string(),
            )
        })?)
    } else {
        None
    };

    let result = provider
        .query_resource(connector_id, resource, params)
        .await
        .map_err(AgentError::Tool)?;

    if let Some(intake) = intake {
        let source_id = args
            .get("source_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("{connector_id}:{resource}:{}", ctx.function_call_id()));
        let source_type = format!("resource_read:{connector_id}:{resource}");
        let record = resource_read_evidence_record(ctx, &source_id, &source_type, args);
        intake
            .record_evidence(record)
            .await
            .map_err(AgentError::Tool)?;
        let serialized = serde_json::to_string(&result)
            .map_err(|e| AgentError::Tool(format!("serialize resource result: {e}")))?;
        intake
            .enqueue(
                &source_id,
                &source_type,
                &serialized,
                non_empty(ctx.session_id()),
                ctx.agent_name(),
            )
            .await
            .map_err(AgentError::Tool)?;
    }

    Ok(result)
}

async fn invoke_connector_capability(
    provider: &dyn ConnectorResourceProvider,
    ctx: &dyn ToolContext,
    args: &Value,
) -> Result<Value> {
    let connector_id = args
        .get("connector_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            AgentError::Tool("Missing 'connector_id' parameter for invoke action".to_string())
        })?;

    let capability = args
        .get("capability")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            AgentError::Tool("Missing 'capability' parameter for invoke action".to_string())
        })?;

    let payload = args.get("payload").cloned().unwrap_or(json!({}));

    provider
        .invoke_capability(
            connector_id,
            capability,
            payload,
            ctx.session_id(),
            ctx.agent_name(),
        )
        .await
        .map_err(AgentError::Tool)
}

fn non_empty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

fn resource_read_evidence_record(
    ctx: &dyn ToolContext,
    source_id: &str,
    source_type: &str,
    args: &Value,
) -> EvidenceRecord {
    let session_id = ctx.session_id();
    EvidenceRecord {
        evidence_id: format!("{}:resource_read:{source_id}", ctx.agent_name()),
        action: "resource_read_distillation".to_string(),
        source_id: source_id.to_string(),
        source_type: source_type.to_string(),
        session_id: non_empty(session_id).map(str::to_string),
        agent_id: ctx.agent_name().to_string(),
        retention_policy: args
            .get("retention_policy")
            .and_then(Value::as_str)
            .unwrap_or("durable")
            .to_string(),
        ontology_labels: string_array_arg(args, "ontology_labels"),
        taxonomy_labels: string_array_arg(args, "taxonomy_labels"),
    }
}

fn string_array_arg(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_primitives::connectors::{CapabilityInfo, ConnectorInfo, ResourceInfo};
    use agent_primitives::context::{CallbackContext, ReadonlyContext};
    use agent_primitives::event::EventActions;
    use agent_primitives::types::Content;
    use std::sync::Mutex;

    /// Mock provider for testing.
    struct MockProvider {
        connectors: Vec<ConnectorInfo>,
    }

    struct PanicQueryProvider;

    #[derive(Default)]
    struct MockIntake {
        records: Mutex<Vec<EvidenceRecord>>,
        enqueued: Mutex<Vec<(String, String, String, Option<String>, String)>>,
    }

    #[async_trait]
    impl IngestionAccess for MockIntake {
        async fn record_evidence(&self, record: EvidenceRecord) -> std::result::Result<(), String> {
            self.records.lock().unwrap().push(record);
            Ok(())
        }

        async fn enqueue(
            &self,
            source_id: &str,
            source_type: &str,
            text: &str,
            session_id: Option<&str>,
            agent_id: &str,
        ) -> std::result::Result<(String, usize), String> {
            self.enqueued.lock().unwrap().push((
                source_id.to_string(),
                source_type.to_string(),
                text.to_string(),
                session_id.map(str::to_string),
                agent_id.to_string(),
            ));
            Ok((source_id.to_string(), 1))
        }

        async fn ingest_structured(
            &self,
            _agent_id: &str,
            _entities: Vec<super::super::ingest::StructuredEntity>,
            _relationships: Vec<super::super::ingest::StructuredRelationship>,
        ) -> std::result::Result<super::super::ingest::StructuredCounts, String> {
            Ok(super::super::ingest::StructuredCounts {
                entities_upserted: 0,
                relationships_upserted: 0,
            })
        }
    }

    #[async_trait]
    impl ConnectorResourceProvider for MockProvider {
        async fn list_connectors(&self) -> std::result::Result<Vec<ConnectorInfo>, String> {
            Ok(self.connectors.clone())
        }

        async fn query_resource(
            &self,
            connector_id: &str,
            resource_name: &str,
            _params: Option<HashMap<String, String>>,
        ) -> std::result::Result<serde_json::Value, String> {
            if connector_id == "signal" && resource_name == "aliases" {
                Ok(json!([
                    {"alias": "dev-team", "number": "+1234567890"},
                    {"alias": "ops", "number": "+0987654321"}
                ]))
            } else {
                Err(format!(
                    "Resource '{}' not found on '{}'",
                    resource_name, connector_id
                ))
            }
        }

        async fn invoke_capability(
            &self,
            connector_id: &str,
            capability: &str,
            _payload: serde_json::Value,
            _session_id: &str,
            _agent_id: &str,
        ) -> std::result::Result<serde_json::Value, String> {
            if connector_id == "signal" && capability == "send_message" {
                Ok(json!({"success": true, "status": 200, "body": "sent"}))
            } else {
                Err(format!(
                    "Capability '{}' not found on '{}'",
                    capability, connector_id
                ))
            }
        }
    }

    #[async_trait]
    impl ConnectorResourceProvider for PanicQueryProvider {
        async fn list_connectors(&self) -> std::result::Result<Vec<ConnectorInfo>, String> {
            Ok(Vec::new())
        }

        async fn query_resource(
            &self,
            _connector_id: &str,
            _resource_name: &str,
            _params: Option<HashMap<String, String>>,
        ) -> std::result::Result<serde_json::Value, String> {
            panic!("query_resource should not run when evidence intake is unavailable")
        }

        async fn invoke_capability(
            &self,
            _connector_id: &str,
            _capability: &str,
            _payload: serde_json::Value,
            _session_id: &str,
            _agent_id: &str,
        ) -> std::result::Result<serde_json::Value, String> {
            Ok(json!({}))
        }
    }

    fn mock_provider() -> Arc<dyn ConnectorResourceProvider> {
        Arc::new(MockProvider {
            connectors: vec![ConnectorInfo {
                id: "signal".to_string(),
                name: "Signal Bridge".to_string(),
                resources: vec![
                    ResourceInfo {
                        name: "aliases".to_string(),
                        uri: "http://localhost:9001/aliases".to_string(),
                        method: "GET".to_string(),
                        description: Some("List signal aliases".to_string()),
                    },
                    ResourceInfo {
                        name: "messages".to_string(),
                        uri: "http://localhost:9001/messages/{thread_id}".to_string(),
                        method: "GET".to_string(),
                        description: Some("Get messages for a thread".to_string()),
                    },
                ],
                capabilities: vec![CapabilityInfo {
                    name: "send_message".to_string(),
                    schema: json!({"type": "object", "properties": {"text": {"type": "string"}, "recipient": {"type": "string"}}}),
                    description: Some("Send a message via Signal".to_string()),
                }],
            }],
        })
    }

    struct MockToolContext;

    impl ReadonlyContext for MockToolContext {
        fn invocation_id(&self) -> &str {
            "test"
        }
        fn agent_name(&self) -> &str {
            "test-agent"
        }
        fn user_id(&self) -> &str {
            "test"
        }
        fn app_name(&self) -> &str {
            "test"
        }
        fn session_id(&self) -> &str {
            "test"
        }
        fn branch(&self) -> &str {
            "test"
        }
        fn user_content(&self) -> &Content {
            use std::sync::LazyLock;
            static CONTENT: LazyLock<Content> = LazyLock::new(|| Content {
                role: "user".to_string(),
                parts: vec![],
            });
            &CONTENT
        }
    }

    impl CallbackContext for MockToolContext {
        fn get_state(&self, _key: &str) -> Option<Value> {
            None
        }
        fn set_state(&self, _key: String, _value: Value) {}
    }

    impl ToolContext for MockToolContext {
        fn function_call_id(&self) -> String {
            "test-call".to_string()
        }
        fn actions(&self) -> EventActions {
            EventActions::default()
        }
        fn set_actions(&self, _actions: EventActions) {}
    }

    fn mock_context() -> Arc<dyn ToolContext> {
        Arc::new(MockToolContext)
    }

    #[tokio::test]
    async fn test_list_resources() {
        let tool = QueryResourceTool::new(mock_provider());
        let ctx = mock_context();

        let result = tool
            .execute(ctx, json!({"action": "list_resources"}))
            .await
            .unwrap();

        let connectors = result["connectors"].as_array().unwrap();
        assert_eq!(connectors.len(), 1);
        assert_eq!(connectors[0]["connector_id"], "signal");

        let resources = connectors[0]["resources"].as_array().unwrap();
        assert_eq!(resources.len(), 2);
        assert_eq!(resources[0]["name"], "aliases");
    }

    #[tokio::test]
    async fn connector_resource_lists_resources_and_capabilities() {
        let tool = ConnectorResourceTool::new(mock_provider());
        let ctx = mock_context();

        let result = tool.execute(ctx, json!({"action": "list"})).await.unwrap();

        let connectors = result["connectors"].as_array().unwrap();
        assert_eq!(connectors.len(), 1);
        assert_eq!(connectors[0]["connector_id"], "signal");
        assert_eq!(connectors[0]["resources"][0]["name"], "aliases");
        assert_eq!(connectors[0]["capabilities"][0]["name"], "send_message");
        assert!(
            result["usage"]
                .as_str()
                .unwrap()
                .contains("connector_invoke")
        );
        assert!(!result["usage"].as_str().unwrap().contains("query_resource"));
    }

    #[tokio::test]
    async fn connector_resource_queries_read_only_resource() {
        let tool = ConnectorResourceTool::new(mock_provider());
        let ctx = mock_context();

        let result = tool
            .execute(
                ctx,
                json!({
                    "action": "query",
                    "connector_id": "signal",
                    "resource": "aliases"
                }),
            )
            .await
            .unwrap();

        let aliases = result.as_array().unwrap();
        assert_eq!(aliases.len(), 2);
        assert_eq!(aliases[0]["alias"], "dev-team");
    }

    #[tokio::test]
    async fn test_query_resource() {
        let tool = QueryResourceTool::new(mock_provider());
        let ctx = mock_context();

        let result = tool
            .execute(
                ctx,
                json!({
                    "action": "query",
                    "connector_id": "signal",
                    "resource": "aliases"
                }),
            )
            .await
            .unwrap();

        let aliases = result.as_array().unwrap();
        assert_eq!(aliases.len(), 2);
        assert_eq!(aliases[0]["alias"], "dev-team");
    }

    #[tokio::test]
    async fn query_resource_does_not_record_evidence_by_default() {
        let intake = Arc::new(MockIntake::default());
        let tool = QueryResourceTool::new(mock_provider())
            .with_optional_evidence_intake(Some(intake.clone()));
        let ctx = mock_context();

        let result = tool
            .execute(
                ctx,
                json!({
                    "action": "query",
                    "connector_id": "signal",
                    "resource": "aliases"
                }),
            )
            .await
            .unwrap();

        assert_eq!(result.as_array().unwrap().len(), 2);
        assert!(intake.records.lock().unwrap().is_empty());
        assert!(intake.enqueued.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn query_resource_records_evidence_when_explicitly_requested() {
        let intake = Arc::new(MockIntake::default());
        let tool = QueryResourceTool::new(mock_provider())
            .with_optional_evidence_intake(Some(intake.clone()));
        let ctx = mock_context();

        let result = tool
            .execute(
                ctx,
                json!({
                    "action": "query",
                    "connector_id": "signal",
                    "resource": "aliases",
                    "record_evidence": true,
                    "source_id": "signal:aliases:snapshot-1",
                    "retention_policy": "durable",
                    "ontology_labels": ["contact_alias"],
                    "taxonomy_labels": ["skos:communications"]
                }),
            )
            .await
            .unwrap();

        assert_eq!(result.as_array().unwrap().len(), 2);

        let records = intake.records.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].action, "resource_read_distillation");
        assert_eq!(records[0].source_id, "signal:aliases:snapshot-1");
        assert_eq!(records[0].source_type, "resource_read:signal:aliases");
        assert_eq!(records[0].session_id.as_deref(), Some("test"));
        assert_eq!(records[0].ontology_labels, vec!["contact_alias"]);
        assert_eq!(records[0].taxonomy_labels, vec!["skos:communications"]);

        let enqueued = intake.enqueued.lock().unwrap();
        assert_eq!(enqueued.len(), 1);
        assert_eq!(enqueued[0].0, "signal:aliases:snapshot-1");
        assert_eq!(enqueued[0].1, "resource_read:signal:aliases");
        assert!(enqueued[0].2.contains("dev-team"));
        assert_eq!(enqueued[0].3.as_deref(), Some("test"));
        assert_eq!(enqueued[0].4, "test-agent");
    }

    #[tokio::test]
    async fn connector_resource_records_evidence_when_explicitly_requested() {
        let intake = Arc::new(MockIntake::default());
        let tool = ConnectorResourceTool::new(mock_provider())
            .with_optional_evidence_intake(Some(intake.clone()));
        let ctx = mock_context();

        let result = tool
            .execute(
                ctx,
                json!({
                    "action": "query",
                    "connector_id": "signal",
                    "resource": "aliases",
                    "record_evidence": true,
                    "source_id": "signal:aliases:snapshot-2",
                    "ontology_labels": ["contact_alias"],
                    "taxonomy_labels": ["skos:communications"]
                }),
            )
            .await
            .unwrap();

        assert_eq!(result.as_array().unwrap().len(), 2);
        assert_eq!(intake.records.lock().unwrap().len(), 1);
        assert_eq!(
            intake.records.lock().unwrap()[0].source_id,
            "signal:aliases:snapshot-2"
        );
        assert_eq!(intake.enqueued.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn query_resource_record_evidence_requires_intake() {
        let tool = QueryResourceTool::new(Arc::new(PanicQueryProvider));
        let ctx = mock_context();

        let err = tool
            .execute(
                ctx,
                json!({
                    "action": "query",
                    "connector_id": "signal",
                    "resource": "aliases",
                    "record_evidence": true
                }),
            )
            .await
            .expect_err("record_evidence should fail without intake");

        assert!(err.to_string().contains("requires evidence intake"));
    }

    #[tokio::test]
    async fn test_query_unknown_resource() {
        let tool = QueryResourceTool::new(mock_provider());
        let ctx = mock_context();

        let result = tool
            .execute(
                ctx,
                json!({
                    "action": "query",
                    "connector_id": "signal",
                    "resource": "nonexistent"
                }),
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_missing_action() {
        let tool = QueryResourceTool::new(mock_provider());
        let ctx = mock_context();

        let result = tool.execute(ctx, json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_query_missing_connector_id() {
        let tool = QueryResourceTool::new(mock_provider());
        let ctx = mock_context();

        let result = tool
            .execute(
                ctx,
                json!({
                    "action": "query",
                    "resource": "aliases"
                }),
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_empty_connectors() {
        let provider: Arc<dyn ConnectorResourceProvider> =
            Arc::new(MockProvider { connectors: vec![] });
        let tool = QueryResourceTool::new(provider);
        let ctx = mock_context();

        let result = tool
            .execute(ctx, json!({"action": "list_resources"}))
            .await
            .unwrap();

        let connectors = result["connectors"].as_array().unwrap();
        assert!(connectors.is_empty());
        assert!(result["message"].as_str().is_some());
    }

    #[tokio::test]
    async fn test_list_resources_includes_capabilities() {
        let tool = QueryResourceTool::new(mock_provider());
        let ctx = mock_context();

        let result = tool
            .execute(ctx, json!({"action": "list_resources"}))
            .await
            .unwrap();

        let connectors = result["connectors"].as_array().unwrap();
        assert_eq!(connectors.len(), 1);

        let capabilities = connectors[0]["capabilities"].as_array().unwrap();
        assert_eq!(capabilities.len(), 1);
        assert_eq!(capabilities[0]["name"], "send_message");
        assert_eq!(capabilities[0]["type"], "capability");
        assert_eq!(capabilities[0]["method"], "POST");
    }

    #[tokio::test]
    async fn test_invoke_capability() {
        let tool = QueryResourceTool::new(mock_provider());
        let ctx = mock_context();

        let result = tool
            .execute(
                ctx,
                json!({
                    "action": "invoke",
                    "connector_id": "signal",
                    "capability": "send_message",
                    "payload": {"text": "hello", "recipient": "+1234567890"}
                }),
            )
            .await
            .unwrap();

        assert_eq!(result["success"], true);
    }

    #[tokio::test]
    async fn connector_invoke_invokes_capability_without_read_controls() {
        let tool = ConnectorInvokeTool::new(mock_provider());
        let ctx = mock_context();

        let result = tool
            .execute(
                ctx,
                json!({
                    "connector_id": "signal",
                    "capability": "send_message",
                    "payload": {"text": "hello", "recipient": "+1234567890"},
                    "record_evidence": true,
                    "resource": "aliases"
                }),
            )
            .await
            .unwrap();

        assert_eq!(result["success"], true);
        let schema = tool.parameters_schema().unwrap();
        assert!(schema["properties"].get("payload").is_some());
        assert!(schema["properties"].get("resource").is_none());
        assert!(schema["properties"].get("record_evidence").is_none());
    }

    #[tokio::test]
    async fn test_invoke_unknown_capability() {
        let tool = QueryResourceTool::new(mock_provider());
        let ctx = mock_context();

        let result = tool
            .execute(
                ctx,
                json!({
                    "action": "invoke",
                    "connector_id": "signal",
                    "capability": "nonexistent"
                }),
            )
            .await;

        assert!(result.is_err());
    }
}
