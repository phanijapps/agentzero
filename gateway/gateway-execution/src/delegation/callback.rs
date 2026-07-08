//! # Delegation Callbacks
//!
//! Callback formatting and handling for delegation completion.

use super::context::DelegationContext;
use gateway_events::{EventBus, GatewayEvent};
use serde_json::Value;
use std::sync::Arc;

// ============================================================================
// CALLBACK FORMATTING
// ============================================================================

/// Format an agent ID into a display name.
///
/// Converts kebab-case to Title Case (e.g., "research-agent" -> "Research Agent").
pub fn format_agent_display_name(agent_id: &str) -> String {
    agent_id
        .split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

/// Extract the RESULT line from a subagent response.
/// Looks for "RESULT: APPROVED", "RESULT: DEFECTS", "RESULT: OUT_OF_SCOPE",
/// or "RESULT: CAPABILITY_MISSING" near the end.
fn extract_result_line(response: &str) -> Option<&str> {
    response.lines().rev().take(20).find(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("RESULT: APPROVED")
            || trimmed.starts_with("RESULT: DEFECTS")
            || trimmed.starts_with("RESULT: OUT_OF_SCOPE")
            || trimmed.starts_with("RESULT: CAPABILITY_MISSING")
    })
}

/// Extract defect lines after the RESULT: DEFECTS marker.
fn extract_defects(response: &str) -> String {
    let mut in_defects = false;
    let mut defects = Vec::new();

    for line in response.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("RESULT: DEFECTS") {
            in_defects = true;
            continue;
        }
        if in_defects && trimmed.starts_with("- ") {
            defects.push(trimmed.to_string());
        }
    }

    defects.join("\n")
}

/// Format the response body: pretty-print if JSON, otherwise return as-is.
fn format_response_content(response: &str) -> String {
    let trimmed = response.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(response) {
            return format!(
                "```json\n{}\n```",
                serde_json::to_string_pretty(&json_val).unwrap_or_else(|_| response.to_string())
            );
        }
    }
    response.to_string()
}

/// Build the HTML comment envelope injected at the top of callback messages
/// when `output_schema` is set and the response is valid JSON.
fn format_structured_envelope(agent_id: &str, data: &Value, schema_valid: bool) -> String {
    let envelope = serde_json::json!({
        "ok": true,
        "agent": agent_id,
        "data": data,
        "schema_valid": schema_valid
    });
    format!(
        "<!-- structured-result {} -->",
        serde_json::to_string(&envelope).unwrap_or_default()
    )
}

/// Extract the structured result envelope from a callback message.
///
/// Returns `Some(Value)` when the message begins with `<!-- structured-result ... -->`.
/// Returns `None` for plain-text callback messages.
pub fn extract_structured_result(message: &str) -> Option<Value> {
    let prefix = "<!-- structured-result ";
    let suffix = " -->";
    let trimmed = message.trim_start();
    if !trimmed.starts_with(prefix) {
        return None;
    }
    let after_prefix = &trimmed[prefix.len()..];
    let end = after_prefix.rfind(suffix)?;
    let json_str = &after_prefix[..end];
    serde_json::from_str(json_str).ok()
}

/// Format a successful callback message as markdown.
pub fn format_callback_message(
    agent_id: &str,
    response: &str,
    conversation_id: &str,
    output_schema: Option<&Value>,
) -> String {
    let agent_display_name = format_agent_display_name(agent_id);

    let response_content = if response.is_empty() {
        "_No response generated._".to_string()
    } else {
        format_response_content(response)
    };

    // Detect structured review result for fast root decision-making
    let action_hint = if let Some(result_line) = extract_result_line(response) {
        if result_line.contains("APPROVED") {
            "\n\n**Action:** This node APPROVED. Proceed to the next node in the execution plan."
                .to_string()
        } else if result_line.contains("DEFECTS") {
            let defects = extract_defects(response);
            format!(
                "\n\n**Action:** DEFECTS found. Re-delegate to coding agent with these defects:\n{}",
                defects
            )
        } else if result_line.contains("OUT_OF_SCOPE") {
            "\n\n**Action:** This ward reported the task is outside its domain \
             (out of scope). Do NOT relay this result to the user — delegate \
             the original task to `planner-agent` instead; it will route to or \
             build the right ward."
                .to_string()
        } else if result_line.contains("CAPABILITY_MISSING") {
            "\n\n**Action:** This ward reported the task IS in its domain but \
             it lacks a required capability. Do NOT relay this result to the \
             user — delegate the original task to `planner-agent`, stating \
             that this existing ward is the correct one and only the missing \
             capability must be ADDED to it. The planner must augment the \
             existing ward, not create a new one."
                .to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let markdown_body = format!(
        "## From {}\n\n{}{}\n\n---\n_Conversation: `{}`_\n\n\
         [Recall] Delegation completed. Consider recalling to absorb any new learnings.",
        agent_display_name, response_content, action_hint, conversation_id
    );

    // Prepend structured envelope when schema is set and response is valid JSON
    if output_schema.is_some() {
        if let Ok(json_val) = serde_json::from_str::<Value>(response.trim()) {
            // Check if validate_delegation_response already flagged it as invalid
            let schema_valid = json_val
                .get("_schema_valid")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let envelope = format_structured_envelope(agent_id, &json_val, schema_valid);
            return format!("{envelope}\n{markdown_body}");
        }
    }

    markdown_body
}

/// Format an error callback message as markdown.
pub fn format_error_callback_message(agent_id: &str, error: &str, conversation_id: &str) -> String {
    let agent_display_name = format_agent_display_name(agent_id);

    format!(
        "## Delegation Failed\n\n**Agent:** {}\n**Error:** {}\n\n---\n_Conversation: `{}`_",
        agent_display_name, error, conversation_id
    )
}

// ============================================================================
// RESPONSE VALIDATION
// ============================================================================

/// Validate a delegation response against an output schema.
///
/// Performs lenient validation:
/// - If no schema is provided, the response is returned as-is.
/// - If the response is valid JSON, it is re-serialized (normalized) and returned.
/// - If the response is not valid JSON, it is wrapped as `{ "summary": "...", "_schema_valid": false }`
///   so the parent agent always receives valid JSON without losing the child's work.
///
/// Full JSON Schema validation is future work; this currently only checks that
/// the response parses as JSON.
pub fn validate_delegation_response(response: &str, schema: &Option<Value>) -> String {
    if schema.is_none() {
        return response.to_string();
    }
    match serde_json::from_str::<Value>(response.trim()) {
        Ok(json) => {
            // Valid JSON — return normalized form
            serde_json::to_string(&json).unwrap_or_else(|_| response.to_string())
        }
        Err(_) => {
            // Not JSON — wrap as summary with validation flag
            let wrapped = serde_json::json!({
                "summary": response.trim(),
                "_schema_valid": false
            });
            serde_json::to_string(&wrapped).unwrap_or_else(|_| response.to_string())
        }
    }
}

// ============================================================================
// CALLBACK SENDING
// ============================================================================

/// Send a callback message to the parent session stream.
///
/// Returns true if the message was sent successfully.
pub async fn send_callback_to_parent(
    messages: &dyn zbot_conversation::MessageStore,
    event_bus: &EventBus,
    session_id: &str,
    parent_execution_id: &str,
    message: &str,
) -> bool {
    let callback = zbot_conversation::Message {
        id: format!("msg-{}", uuid::Uuid::new_v4()),
        execution_id: Some(parent_execution_id.to_string()),
        session_id: session_id.to_string(),
        role: "system".to_string(),
        content: message.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        token_count: message.len() as i64 / 4,
        tool_calls: None,
        tool_call_id: None,
        seq: 0,
    };

    match messages.append(&callback) {
        Ok(_) => {
            // Emit event so frontend can refresh
            event_bus
                .publish(GatewayEvent::MessageAdded {
                    session_id: session_id.to_string(),
                    execution_id: parent_execution_id.to_string(),
                    role: "system".to_string(),
                    content: message.to_string(),
                    conversation_id: Some(parent_execution_id.to_string()),
                })
                .await;
            true
        }
        Err(e) => {
            tracing::error!(
                parent_execution = %parent_execution_id,
                session_id = %session_id,
                "Failed to add callback message to session: {}", e
            );
            false
        }
    }
}

/// Handle successful delegation completion with callback.
///
/// When the delegation has an `output_schema`, the response is validated
/// (or wrapped) before being forwarded to the parent.
#[allow(clippy::too_many_arguments)]
pub async fn handle_delegation_success(
    delegation_ctx: Option<&DelegationContext>,
    messages: &dyn zbot_conversation::MessageStore,
    event_bus: &EventBus,
    session_id: &str,
    parent_execution_id: &str,
    child_agent_id: &str,
    child_conversation_id: &str,
    response: &str,
) {
    if let Some(ctx) = delegation_ctx {
        if ctx.callback_on_complete {
            // Validate response against output_schema (if present)
            let validated_response = validate_delegation_response(response, &ctx.output_schema);
            let callback_msg = format_callback_message(
                child_agent_id,
                &validated_response,
                child_conversation_id,
                ctx.output_schema.as_ref(),
            );

            if send_callback_to_parent(
                messages,
                event_bus,
                session_id,
                parent_execution_id,
                &callback_msg,
            )
            .await
            {
                tracing::info!(
                    parent_execution = %parent_execution_id,
                    child_agent = %child_agent_id,
                    has_output_schema = ctx.output_schema.is_some(),
                    "Sent callback to parent execution"
                );
            }
        }
    }
}

/// Handle delegation failure with error callback.
pub async fn handle_delegation_failure(
    messages: &dyn zbot_conversation::MessageStore,
    event_bus: &EventBus,
    session_id: &str,
    parent_execution_id: &str,
    child_agent_id: &str,
    child_conversation_id: &str,
    error: &str,
) {
    let error_msg = format_error_callback_message(child_agent_id, error, child_conversation_id);

    if send_callback_to_parent(
        messages,
        event_bus,
        session_id,
        parent_execution_id,
        &error_msg,
    )
    .await
    {
        tracing::warn!(
            parent_execution = %parent_execution_id,
            child_agent = %child_agent_id,
            error = %error,
            "Sent error callback to parent execution"
        );
    }
}

/// Handle subagent completion and send callback to parent.
///
/// This is called when a delegated subagent completes its task.
/// If callback_on_complete is true, it sends a message to the parent
/// conversation with the result.
pub async fn handle_subagent_completion(
    event_bus: Arc<EventBus>,
    delegation: &DelegationContext,
    session_id: &str,
    child_execution_id: &str,
    child_agent_id: &str,
    child_conversation_id: &str,
    result: Option<String>,
) {
    // Emit delegation completed event
    event_bus
        .publish(GatewayEvent::DelegationCompleted {
            session_id: session_id.to_string(),
            parent_execution_id: delegation.parent_execution_id.clone(),
            child_execution_id: child_execution_id.to_string(),
            parent_agent_id: delegation.parent_agent_id.clone(),
            child_agent_id: child_agent_id.to_string(),
            result,
            parent_conversation_id: Some(delegation.parent_conversation_id.clone()),
            child_conversation_id: Some(child_conversation_id.to_string()),
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingMessageStore {
        appended: Mutex<Vec<zbot_conversation::Message>>,
    }

    impl zbot_conversation::MessageStore for RecordingMessageStore {
        fn append(&self, msg: &zbot_conversation::Message) -> Result<()> {
            self.appended.lock().unwrap().push(msg.clone());
            Ok(())
        }

        fn replay(
            &self,
            _session_id: &str,
            _after_seq: Option<i64>,
            _limit: usize,
        ) -> Result<Vec<zbot_conversation::Message>> {
            Ok(Vec::new())
        }

        fn tool_sequence_for_session(&self, _session_id: &str) -> Result<Vec<String>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn test_format_agent_display_name() {
        assert_eq!(
            format_agent_display_name("research-agent"),
            "Research Agent"
        );
        assert_eq!(format_agent_display_name("code-reviewer"), "Code Reviewer");
        assert_eq!(format_agent_display_name("simple"), "Simple");
    }

    #[tokio::test]
    async fn send_callback_to_parent_appends_system_message_via_message_store() {
        let store = RecordingMessageStore::default();
        let bus = EventBus::new();

        assert!(send_callback_to_parent(&store, &bus, "sess-1", "exec-1", "child result").await);

        let appended = store.appended.lock().unwrap();
        assert_eq!(appended.len(), 1);
        let msg = &appended[0];
        assert_eq!(msg.session_id, "sess-1");
        assert_eq!(msg.execution_id.as_deref(), Some("exec-1"));
        assert_eq!(msg.role, "system");
        assert_eq!(msg.content, "child result");
        assert!(msg.tool_calls.is_none());
        assert!(msg.tool_call_id.is_none());
    }

    #[test]
    fn test_extract_result_approved() {
        let response = "Everything looks good.\n\nRESULT: APPROVED";
        assert!(extract_result_line(response).unwrap().contains("APPROVED"));
    }

    #[test]
    fn test_extract_result_defects() {
        let response = "Found issues.\n\nRESULT: DEFECTS\n- core/fetch.py: Missing error handling (severity: medium)";
        assert!(extract_result_line(response).unwrap().contains("DEFECTS"));
    }

    #[test]
    fn test_extract_result_none() {
        let response = "Just a normal response without structured result.";
        assert!(extract_result_line(response).is_none());
    }

    #[test]
    fn test_extract_result_out_of_scope() {
        let response =
            "Checked the request.\nRESULT: OUT_OF_SCOPE — financial task, not a travel ward";
        assert!(extract_result_line(response)
            .unwrap()
            .contains("OUT_OF_SCOPE"));
    }

    #[test]
    fn test_callback_out_of_scope_hints_replan() {
        let msg = format_callback_message(
            "ward:travel-planning",
            "RESULT: OUT_OF_SCOPE — this is a financial task",
            "conv-1",
            None,
        );
        assert!(msg.contains("**Action:**"));
        assert!(msg.contains("planner-agent"));
    }

    #[test]
    fn test_extract_result_capability_missing() {
        let response = "Checked the request.\nRESULT: CAPABILITY_MISSING — no dealer-inventory MCP";
        assert!(extract_result_line(response)
            .unwrap()
            .contains("CAPABILITY_MISSING"));
    }

    #[test]
    fn test_callback_capability_missing_hints_augment() {
        let msg = format_callback_message(
            "ward:automotive-research",
            "RESULT: CAPABILITY_MISSING — no dealer-inventory MCP",
            "conv-2",
            None,
        );
        assert!(msg.contains("**Action:**"));
        assert!(msg.contains("planner-agent"));
        assert!(msg.contains("augment the existing ward"));
    }

    #[test]
    fn test_extract_defects() {
        let response = "Review done.\n\nRESULT: DEFECTS\n- file.py: Bug (severity: high)\n- data.json: Missing field (severity: medium)";
        let defects = extract_defects(response);
        assert!(defects.contains("file.py: Bug"));
        assert!(defects.contains("data.json: Missing field"));
    }

    #[test]
    fn test_callback_with_approved() {
        let msg = format_callback_message(
            "code-agent",
            "Code looks clean.\n\nRESULT: APPROVED",
            "conv-123",
            None,
        );
        assert!(msg.contains("APPROVED"));
        assert!(msg.contains("Proceed to the next node"));
    }

    #[test]
    fn test_callback_with_defects() {
        let msg = format_callback_message(
            "data-analyst",
            "Found issues.\n\nRESULT: DEFECTS\n- output.json: Wrong values (severity: high)",
            "conv-123",
            None,
        );
        assert!(msg.contains("DEFECTS found"));
        assert!(msg.contains("Re-delegate to coding agent"));
        assert!(msg.contains("Wrong values"));
    }

    #[test]
    fn test_callback_without_result() {
        let msg = format_callback_message(
            "research-agent",
            "Here are the results of my research.",
            "conv-123",
            None,
        );
        assert!(!msg.contains("Action:"));
    }

    #[test]
    fn structured_envelope_present_when_schema_set() {
        use serde_json::json;
        let schema = json!({"type": "object"});
        let response = r#"{"score": 0.9}"#;
        let msg = format_callback_message("code-agent", response, "conv-123", Some(&schema));
        assert!(
            msg.starts_with("<!-- structured-result"),
            "envelope must be first"
        );
        assert!(msg.contains("\"schema_valid\":true"));
        assert!(msg.contains("\"agent\":\"code-agent\""));
    }

    #[test]
    fn no_envelope_when_no_schema() {
        let msg = format_callback_message("code-agent", "plain text result", "conv-123", None);
        assert!(!msg.contains("<!-- structured-result"));
    }

    #[test]
    fn extract_structured_result_parses_envelope() {
        use serde_json::json;
        let schema = json!({"type": "object"});
        let msg =
            format_callback_message("code-agent", r#"{"score": 0.9}"#, "conv-123", Some(&schema));
        let extracted = extract_structured_result(&msg).expect("should parse");
        assert_eq!(extracted["data"]["score"], json!(0.9));
        assert_eq!(extracted["schema_valid"], json!(true));
    }

    #[test]
    fn schema_invalid_when_response_was_not_json() {
        use serde_json::json;
        let schema = json!({"type": "object"});
        // Simulate what validate_delegation_response does with non-JSON:
        let validated = serde_json::json!({"summary": "not json at all", "_schema_valid": false});
        let validated_str = serde_json::to_string(&validated).unwrap();
        let msg = format_callback_message("code-agent", &validated_str, "conv-123", Some(&schema));
        // Envelope IS present (validated_str is valid JSON)
        assert!(msg.starts_with("<!-- structured-result"));
        // But schema_valid is false
        let extracted = extract_structured_result(&msg).unwrap();
        assert_eq!(extracted["schema_valid"], json!(false));
    }

    #[test]
    fn parent_extract_without_markdown_parsing() {
        use serde_json::json;
        let schema = json!({"type": "object"});
        let response = r#"{"score": 0.9, "label": "good"}"#;
        let msg = format_callback_message("analysis-agent", response, "conv-xyz", Some(&schema));
        let result = extract_structured_result(&msg).unwrap();
        assert_eq!(result["data"]["score"].as_f64().unwrap(), 0.9_f64);
        assert_eq!(result["agent"].as_str().unwrap(), "analysis-agent");
    }
}
