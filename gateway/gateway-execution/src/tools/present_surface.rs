use std::sync::Arc;

use agent_primitives::{AgentError, Result, Tool, ToolContext};
use agent_surfaces::{
    ComponentType, SurfaceComponent, SurfaceValidationError, SurfaceValidator, WorkSurface,
    ZbotWorkSurfaceCatalog, ZBOT_WORK_SURFACE_CATALOG,
};
use async_trait::async_trait;
use serde_json::{json, Value};

/// Publishes a bounded, display-only native work surface through the runtime's
/// existing tool-result marker path.
pub struct PresentSurfaceTool;

impl PresentSurfaceTool {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for PresentSurfaceTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for PresentSurfaceTool {
    fn name(&self) -> &'static str {
        "present_surface"
    }

    fn description(&self) -> &'static str {
        "Use when a user-facing canonical response contains comparative, metric, status, \
         record, timeline, table, or chart-shaped data that is materially easier to \
         understand visually. Publish one coherent display-only surface, then still use \
         respond to send the complete canonical response. Do not use for simple facts, \
         short prose, code-only answers, or clarification questions. Never include secrets, \
         system prompts, developer instructions, hidden reasoning, unrelated connector data, \
         or unrelated tool data; surface data must already be appropriate for the canonical \
         response. Reuse a stable surface_id with update=true instead of creating duplicates. \
         Every component requires a unique id, a supported type, and a props object. \
         Component props: DecisionMatrix(criteria_path, options_path); \
         EvidenceTable(evidence_path); AssumptionRegister(assumptions_path); \
         PlanChecklist(plan_path); OpenLoops(items_path); \
         MetricCard(value_path, optional label/detail_path); \
         ProgressBar(value_path, optional label/max); StatusBadge(value_path, optional label); \
         Callout(message_path, optional tone); KeyValueList(items_path); \
         DataTable(rows_path, optional columns — an array of plain string field keys \
         into each row, e.g. [\"trailingPE\"], never objects); Timeline(items_path); \
         LineChart/BarChart(data_path, x_key, series), where data_path is a JSON \
         pointer such as /points and series is an array of field-name strings such \
         as [\"value\"]; PieChart(data_path, name_key, value_key). \
         Every component supports optional title and title_path. Prefer a specific \
         contextual title, or title_path when the heading should come from data; \
         never rely on generic component type labels like LineChart or Callout."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "surface_id": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 128,
                    "description": "Stable identifier. Reuse it with update=true to replace the surface."
                },
                "components": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 64,
                    "description": "Display-only zbot component descriptors. Bind content through JSON-pointer props into data.",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["id", "type", "props"],
                        "properties": {
                            "id": {"type": "string", "minLength": 1, "maxLength": 128},
                            "type": {
                                "enum": [
                                    "DecisionMatrix",
                                    "EvidenceTable",
                                    "AssumptionRegister",
                                    "PlanChecklist",
                                    "OpenLoops",
                                    "MetricCard",
                                    "ProgressBar",
                                    "StatusBadge",
                                    "Callout",
                                    "KeyValueList",
                                    "DataTable",
                                    "Timeline",
                                    "LineChart",
                                    "BarChart",
                                    "PieChart"
                                ]
                            },
                            "props": {
                                "type": "object",
                                "description": "Component-specific declarative props only. Paths are JSON pointers into data. Field-key arrays (DataTable columns, chart series) are arrays of plain strings, never objects."
                            }
                        }
                    }
                },
                "data": {
                    "type": "object",
                    "description": "Only user-facing values referenced by component JSON pointers."
                },
                "update": {
                    "type": "boolean",
                    "default": false,
                    "description": "True to replace the existing surface with the same surface_id."
                }
            },
            "required": ["surface_id", "components", "data"]
        }))
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let Some(object) = args.as_object() else {
            return Err(tool_error("surface descriptor must be an object"));
        };
        if object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "surface_id" | "components" | "data" | "update"
            )
        }) {
            return Err(tool_error(
                "surface descriptor contains an unsupported field",
            ));
        }

        let surface_id = object
            .get("surface_id")
            .and_then(Value::as_str)
            .ok_or_else(|| tool_error("surface_id must be a string"))?
            .to_owned();
        let components = object
            .get("components")
            .cloned()
            .and_then(|value| serde_json::from_value::<Vec<SurfaceComponent>>(value).ok())
            .ok_or_else(|| tool_error("components have an invalid shape or component type"))?;
        if components.is_empty() {
            return Err(tool_error("components must not be empty"));
        }
        if components
            .iter()
            .any(|component| component.component_type == ComponentType::ApprovalGate)
        {
            return Err(tool_error("surface contains a non-display component"));
        }
        let data = object
            .get("data")
            .filter(|value| value.is_object())
            .cloned()
            .ok_or_else(|| tool_error("data must be an object"))?;
        let update = match object.get("update") {
            Some(value) => value
                .as_bool()
                .ok_or_else(|| tool_error("update must be a boolean"))?,
            None => false,
        };

        let surface = WorkSurface {
            surface_id,
            catalog_id: ZBOT_WORK_SURFACE_CATALOG.to_owned(),
            components,
            data,
        };
        ZbotWorkSurfaceCatalog
            .validate(&surface)
            .map_err(surface_validation_error)?;

        if update {
            Ok(json!({
                "__work_surface_updated": true,
                "surface": surface
            }))
        } else {
            Ok(json!({
                "__work_surface": true,
                "surface": surface
            }))
        }
    }
}

fn tool_error(message: &str) -> AgentError {
    AgentError::Tool(message.to_owned())
}

/// Surface a validation failure with the detail the model needs to correct
/// its next call. Every `SurfaceValidationError` Display string names only
/// structural identifiers (component type, property name, catalog, id) —
/// property VALUES never appear, so the thiserror rendering is safe to pass
/// through verbatim. `InvalidComponentId` echoes a caller-authored id, not
/// data; its length is bounded by the 64 KiB surface payload gate that runs
/// before the id check. The previous generic strings ("invalid component
/// property") hid the target and produced five identical failed retries
/// (session sess-a0788ab4).
fn surface_validation_error(error: SurfaceValidationError) -> AgentError {
    tool_error(&error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::tools::context::ToolContext as ConcreteCtx;

    fn test_ctx() -> Arc<dyn ToolContext> {
        Arc::new(ConcreteCtx::full_with_state(
            "root".into(),
            Some("c1".into()),
            vec![],
            Default::default(),
        ))
    }

    /// The exact shape the model sent five times in session sess-a0788ab4:
    /// DataTable columns as {key,label,format} objects instead of string keys.
    fn incident_payload() -> Value {
        json!({
            "surface_id": "valuation",
            "components": [{
                "id": "valuation-table",
                "type": "DataTable",
                "props": {
                    "title": "GOOGL vs Peers",
                    "rows_path": "/rows",
                    "columns": [
                        {"key": "ticker", "label": "Ticker", "format": ".1f"}
                    ]
                }
            }],
            "data": {"rows": [{"ticker": "GOOGL"}]}
        })
    }

    #[tokio::test]
    async fn column_objects_error_names_component_and_property() {
        let result = PresentSurfaceTool::new()
            .execute(test_ctx(), incident_payload())
            .await;
        let message = format!("{}", result.expect_err("column objects must reject"));
        assert!(
            message.contains("DataTable") && message.contains("columns"),
            "error must name component and property so the model can correct the next call: {message}"
        );
    }

    #[tokio::test]
    async fn errors_never_carry_property_values() {
        // A title VALUE over the 128-char limit with a sentinel: the error
        // must name the property but never echo the value.
        const SENTINEL: &str = "SENTINEL-VALUE-MUST-NOT-LEAK";
        let mut payload = incident_payload();
        payload["components"][0]["props"]["title"] =
            json!(format!("{SENTINEL}{}", "x".repeat(200)));
        let result = PresentSurfaceTool::new().execute(test_ctx(), payload).await;
        let message = format!("{}", result.expect_err("oversized title must reject"));
        assert!(message.contains("title"), "names the property: {message}");
        assert!(
            !message.contains(SENTINEL),
            "property values must never surface in errors: {message}"
        );
    }
}
