use crate::a2a::{
    A2aDelegationContext, A2aDelegationError, A2aDelegationService, LocalA2aActorKind,
};
use agent_primitives::{AgentError, Result, Tool, ToolContext};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct ListZbotsTool {
    service: Arc<dyn A2aDelegationService>,
}

impl ListZbotsTool {
    pub fn new(service: Arc<dyn A2aDelegationService>) -> Self {
        Self { service }
    }
}

pub(super) fn delegation_context(ctx: &dyn ToolContext) -> Result<A2aDelegationContext> {
    fn state(ctx: &dyn ToolContext, key: &str) -> Result<String> {
        ctx.get_state(key)
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AgentError::Tool(format!("{key} is required in tool context")))
    }
    let actor_kind = match state(ctx, "app:actor_kind")?.as_str() {
        "root" => LocalA2aActorKind::Root,
        "ward_agent" => LocalA2aActorKind::Ward,
        _ => {
            return Err(AgentError::Tool(
                "A2A delegation is not authorized".to_owned(),
            ))
        }
    };
    Ok(A2aDelegationContext {
        actor_kind,
        agent_id: ctx.agent_name().to_owned(),
        session_id: state(ctx, "session_id")?,
        execution_id: state(ctx, "execution_id")?,
        conversation_id: ctx.session_id().to_owned(),
        request_id: ctx.function_call_id(),
    })
}

pub(super) fn delegation_error(error: A2aDelegationError) -> Value {
    let (status, message) = match error {
        A2aDelegationError::InvalidRequest => ("invalid_request", "The request is invalid."),
        A2aDelegationError::NotAuthorized => (
            "not_authorized",
            "This execution cannot delegate to a zBot.",
        ),
        A2aDelegationError::PeerUnavailable => {
            ("peer_unavailable", "The trusted zBot is unavailable.")
        }
        A2aDelegationError::TemporarilyUnavailable => (
            "temporarily_unavailable",
            "A2A delegation is temporarily unavailable.",
        ),
    };
    json!({"status": status, "message": message})
}

#[async_trait]
impl Tool for ListZbotsTool {
    fn name(&self) -> &'static str {
        "list_zbots"
    }

    fn description(&self) -> &'static str {
        "List explicitly trusted zBots that are configured for outbound A2A delegation."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, _args: Value) -> Result<Value> {
        match self
            .service
            .list_peers(&delegation_context(ctx.as_ref())?)
            .await
        {
            Ok(peers) => Ok(json!({"status": "ok", "peers": peers})),
            Err(error) => Ok(delegation_error(error)),
        }
    }
}
