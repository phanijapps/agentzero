//! Tool schema normalization and hardening.

use serde_json::{json, Value};

/// Canonical model-visible name encoding shared by both migration paths.
pub(crate) fn normalize_tool_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Harden a tool parameter schema for stricter LLM compliance.
/// Adds `"additionalProperties": false` if absent.
/// Ensures `"required"` array exists (empty if missing).
pub(crate) fn harden_tool_schema(mut schema: Value) -> Value {
    if let Some(obj) = schema.as_object_mut() {
        if obj.get("type").and_then(|v| v.as_str()) == Some("object") {
            obj.entry("additionalProperties")
                .or_insert(Value::Bool(false));
            obj.entry("required").or_insert_with(|| json!([]));
        }
    }
    schema
}

/// Normalize MCP tool parameters to OpenAI format.
///
/// OpenAI requires `type: "object"` at the root. MCP tools may omit it.
pub(crate) fn normalize_mcp_parameters(params: Option<Value>) -> Value {
    match params {
        None => json!({"type": "object", "properties": {}}),
        Some(p) => {
            if p.get("type").is_some() {
                p
            } else {
                json!({
                    "type": "object",
                    "properties": p
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harden_adds_additional_properties_false() {
        let schema = json!({"type": "object", "properties": {"x": {"type": "string"}}});
        let hardened = harden_tool_schema(schema);
        assert_eq!(hardened["additionalProperties"], json!(false));
        assert_eq!(hardened["required"], json!([]));
    }

    #[test]
    fn harden_does_not_override_existing_additional_properties() {
        let schema = json!({"type": "object", "additionalProperties": true, "properties": {}});
        let hardened = harden_tool_schema(schema);
        assert_eq!(hardened["additionalProperties"], json!(true));
    }

    #[test]
    fn harden_noop_on_non_object_schema() {
        let schema = json!({"type": "array"});
        let result = harden_tool_schema(schema.clone());
        assert_eq!(result, schema);
    }

    #[test]
    fn normalize_none_params_gives_empty_object() {
        let result = normalize_mcp_parameters(None);
        assert_eq!(result["type"], json!("object"));
        assert!(result["properties"].is_object());
    }

    #[test]
    fn normalize_wraps_params_without_type() {
        let params = json!({"name": {"type": "string"}});
        let result = normalize_mcp_parameters(Some(params.clone()));
        assert_eq!(result["type"], json!("object"));
        assert_eq!(result["properties"], params);
    }

    #[test]
    fn normalize_passes_through_params_with_type() {
        let params = json!({"type": "object", "properties": {"name": {"type": "string"}}});
        let result = normalize_mcp_parameters(Some(params.clone()));
        assert_eq!(result, params);
    }
}
