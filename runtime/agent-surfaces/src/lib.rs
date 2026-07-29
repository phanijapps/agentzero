//! Portable, catalog-constrained work surfaces for zbot clients.
//!
//! This crate deliberately has no gateway, renderer, channel, or MCP dependency.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const ZBOT_WORK_SURFACE_CATALOG: &str = "zbot/work-surface/v1";
pub const MAX_COMPONENTS: usize = 64;
pub const MAX_SURFACE_BYTES: usize = 64 * 1024;
pub const MAX_BOUND_ITEMS: usize = 200;
pub const MAX_TABLE_COLUMNS: usize = 20;
pub const MAX_CHART_SERIES: usize = 8;
pub const MAX_PIE_SLICES: usize = 50;
pub const MAX_FIELD_KEY_BYTES: usize = 64;
pub const MAX_RENDERED_STRING_BYTES: usize = 4_096;
pub const MAX_BOUND_DEPTH: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct A2uiCapabilities {
    pub version: String,
    #[serde(default)]
    pub catalogs: BTreeSet<String>,
}

impl A2uiCapabilities {
    #[must_use]
    pub fn supports(&self, catalog_id: &str) -> bool {
        self.catalogs.contains(catalog_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct WorkSurface {
    pub surface_id: String,
    pub catalog_id: String,
    pub components: Vec<SurfaceComponent>,
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct SurfaceComponent {
    pub id: String,
    #[serde(rename = "type")]
    pub component_type: ComponentType,
    #[serde(default)]
    pub props: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum ComponentType {
    DecisionMatrix,
    EvidenceTable,
    AssumptionRegister,
    PlanChecklist,
    ApprovalGate,
    OpenLoops,
    MetricCard,
    ProgressBar,
    StatusBadge,
    Callout,
    KeyValueList,
    DataTable,
    Timeline,
    LineChart,
    BarChart,
    PieChart,
}

/// Whether every component is safe for durable, automatic restoration.
///
/// This exhaustive match intentionally fails compilation when the catalog adds
/// a new component, forcing an explicit persistence decision.
#[must_use]
pub fn is_persistable_surface(surface: &WorkSurface) -> bool {
    surface
        .components
        .iter()
        .all(|component| match component.component_type {
            ComponentType::ApprovalGate => false,
            ComponentType::DecisionMatrix
            | ComponentType::EvidenceTable
            | ComponentType::AssumptionRegister
            | ComponentType::PlanChecklist
            | ComponentType::OpenLoops
            | ComponentType::MetricCard
            | ComponentType::ProgressBar
            | ComponentType::StatusBadge
            | ComponentType::Callout
            | ComponentType::KeyValueList
            | ComponentType::DataTable
            | ComponentType::Timeline
            | ComponentType::LineChart
            | ComponentType::BarChart
            | ComponentType::PieChart => true,
        })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SurfaceActionRequest {
    pub action_id: SurfaceActionId,
    pub surface_id: String,
    pub target: String,
    pub expected_state: Option<String>,
    #[serde(default)]
    pub context: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceActionId {
    Inspect,
    PlanRequestRevision,
    LedgerApprove,
    LedgerBlock,
    LedgerComplete,
}

impl SurfaceActionId {
    #[must_use]
    pub const fn is_write_capable(self) -> bool {
        matches!(
            self,
            Self::LedgerApprove | Self::LedgerBlock | Self::LedgerComplete
        )
    }
}

pub trait SurfaceValidator: Send + Sync {
    fn validate(&self, surface: &WorkSurface) -> Result<(), SurfaceValidationError>;
}

#[derive(Debug, Default)]
pub struct ZbotWorkSurfaceCatalog;

impl SurfaceValidator for ZbotWorkSurfaceCatalog {
    fn validate(&self, surface: &WorkSurface) -> Result<(), SurfaceValidationError> {
        if surface.catalog_id != ZBOT_WORK_SURFACE_CATALOG {
            return Err(SurfaceValidationError::UnsupportedCatalog(
                surface.catalog_id.clone(),
            ));
        }
        if surface.surface_id.is_empty() || surface.surface_id.len() > 128 {
            return Err(SurfaceValidationError::InvalidSurfaceId);
        }
        if surface.components.len() > MAX_COMPONENTS {
            return Err(SurfaceValidationError::TooManyComponents);
        }
        validate_value_depth(&surface.data)?;
        for value in surface
            .components
            .iter()
            .flat_map(|component| component.props.values())
        {
            validate_value_depth(value)?;
        }
        if serde_json::to_vec(surface).map_or(true, |bytes| bytes.len() > MAX_SURFACE_BYTES) {
            return Err(SurfaceValidationError::PayloadTooLarge);
        }
        let mut ids = BTreeSet::new();
        for component in &surface.components {
            if component.id.is_empty() || component.id.len() > 128 || !ids.insert(&component.id) {
                return Err(SurfaceValidationError::InvalidComponentId(
                    component.id.clone(),
                ));
            }
            let allowed = allowed_properties(component.component_type);
            if let Some(property) = component
                .props
                .keys()
                .find(|key| !allowed.iter().any(|allowed| *allowed == *key))
            {
                return Err(SurfaceValidationError::UnsupportedProperty {
                    component: component.component_type,
                    property: property.clone(),
                });
            }
            validate_component_bindings(component)?;
            validate_component_properties(component)?;
            if is_expanded_component(component.component_type) {
                validate_bound_data(component, &surface.data)?;
            }
        }
        Ok(())
    }
}

fn validate_component_bindings(component: &SurfaceComponent) -> Result<(), SurfaceValidationError> {
    for (property, value) in &component.props {
        if property.ends_with("_path") {
            let Some(path) = value.as_str() else {
                return Err(SurfaceValidationError::InvalidBinding {
                    property: property.clone(),
                });
            };
            if !is_json_pointer(path) {
                return Err(SurfaceValidationError::InvalidBinding {
                    property: property.clone(),
                });
            }
        }
    }

    if component.component_type == ComponentType::ApprovalGate {
        let Some(action_id) = component.props.get("action_id").and_then(Value::as_str) else {
            return Err(SurfaceValidationError::InvalidActionReference);
        };
        if serde_json::from_value::<SurfaceActionId>(Value::String(action_id.to_owned())).is_err()
            || component
                .props
                .get("target")
                .and_then(Value::as_str)
                .is_none()
        {
            return Err(SurfaceValidationError::InvalidActionReference);
        }
    }
    Ok(())
}

fn validate_component_properties(
    component: &SurfaceComponent,
) -> Result<(), SurfaceValidationError> {
    for property in required_properties(component.component_type) {
        if !component.props.contains_key(*property) {
            return Err(SurfaceValidationError::MissingProperty {
                component: component.component_type,
                property: (*property).into(),
            });
        }
    }

    if is_expanded_component(component.component_type) {
        for property in ["title", "label"] {
            if let Some(value) = component.props.get(property) {
                if value.as_str().is_none_or(|value| value.len() > 128) {
                    return Err(invalid_property(component, property));
                }
            }
        }
    }

    match component.component_type {
        ComponentType::ProgressBar => {
            if let Some(value) = component.props.get("max") {
                if value
                    .as_f64()
                    .is_none_or(|value| !value.is_finite() || value <= 0.0)
                {
                    return Err(invalid_property(component, "max"));
                }
            }
        }
        ComponentType::Callout => {
            if let Some(value) = component.props.get("tone") {
                if !matches!(
                    value.as_str(),
                    Some("info" | "success" | "warning" | "error")
                ) {
                    return Err(invalid_property(component, "tone"));
                }
            }
        }
        ComponentType::DataTable => {
            if let Some(value) = component.props.get("columns") {
                let Some(columns) = value.as_array() else {
                    return Err(invalid_property(component, "columns"));
                };
                if columns.len() > MAX_TABLE_COLUMNS {
                    return Err(SurfaceValidationError::TooManyColumns);
                }
                let mut unique = BTreeSet::new();
                for column in columns {
                    validate_field_key(component, "columns", column)?;
                    if !unique.insert(column.as_str().expect("field key was validated")) {
                        return Err(invalid_property(component, "columns"));
                    }
                }
            }
        }
        ComponentType::LineChart | ComponentType::BarChart => {
            validate_field_key(
                component,
                "x_key",
                component
                    .props
                    .get("x_key")
                    .expect("required properties were checked"),
            )?;
            let Some(series) = component.props.get("series").and_then(Value::as_array) else {
                return Err(invalid_property(component, "series"));
            };
            if series.is_empty() || series.len() > MAX_CHART_SERIES {
                return Err(SurfaceValidationError::TooManySeries);
            }
            let mut unique = BTreeSet::new();
            for key in series {
                validate_field_key(component, "series", key)?;
                if !unique.insert(key.as_str().expect("field key was validated")) {
                    return Err(invalid_property(component, "series"));
                }
            }
        }
        ComponentType::PieChart => {
            for property in ["name_key", "value_key"] {
                validate_field_key(
                    component,
                    property,
                    component
                        .props
                        .get(property)
                        .expect("required properties were checked"),
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn required_properties(component: ComponentType) -> &'static [&'static str] {
    match component {
        ComponentType::MetricCard | ComponentType::ProgressBar | ComponentType::StatusBadge => {
            &["value_path"]
        }
        ComponentType::Callout => &["message_path"],
        ComponentType::KeyValueList | ComponentType::Timeline => &["items_path"],
        ComponentType::DataTable => &["rows_path"],
        ComponentType::LineChart | ComponentType::BarChart => &["data_path", "x_key", "series"],
        ComponentType::PieChart => &["data_path", "name_key", "value_key"],
        _ => &[],
    }
}

fn is_expanded_component(component: ComponentType) -> bool {
    matches!(
        component,
        ComponentType::MetricCard
            | ComponentType::ProgressBar
            | ComponentType::StatusBadge
            | ComponentType::Callout
            | ComponentType::KeyValueList
            | ComponentType::DataTable
            | ComponentType::Timeline
            | ComponentType::LineChart
            | ComponentType::BarChart
            | ComponentType::PieChart
    )
}

fn invalid_property(component: &SurfaceComponent, property: &str) -> SurfaceValidationError {
    SurfaceValidationError::InvalidProperty {
        component: component.component_type,
        property: property.into(),
    }
}

fn validate_field_key(
    component: &SurfaceComponent,
    property: &str,
    value: &Value,
) -> Result<(), SurfaceValidationError> {
    let Some(key) = value.as_str() else {
        return Err(invalid_property(component, property));
    };
    if key.is_empty() || key.len() > MAX_FIELD_KEY_BYTES {
        return Err(SurfaceValidationError::FieldKeyTooLong {
            property: property.into(),
        });
    }
    Ok(())
}

fn validate_bound_data(
    component: &SurfaceComponent,
    data: &Value,
) -> Result<(), SurfaceValidationError> {
    for (property, path) in component
        .props
        .iter()
        .filter(|(property, _)| property.ends_with("_path"))
    {
        let Some(path) = path.as_str() else {
            continue;
        };
        let Some(value) = data.pointer(path) else {
            continue;
        };
        if component.component_type == ComponentType::DataTable
            && property == "rows_path"
            && !component.props.contains_key("columns")
            && value
                .as_array()
                .and_then(|rows| rows.first())
                .and_then(Value::as_object)
                .is_some_and(|row| row.len() > MAX_TABLE_COLUMNS)
        {
            return Err(SurfaceValidationError::TooManyColumns);
        }
        if component.component_type == ComponentType::PieChart
            && property == "data_path"
            && value
                .as_array()
                .is_some_and(|items| items.len() > MAX_PIE_SLICES)
        {
            return Err(SurfaceValidationError::TooManySlices);
        }
        validate_bound_value(property, value)?;
    }
    Ok(())
}

fn validate_bound_value(property: &str, value: &Value) -> Result<(), SurfaceValidationError> {
    validate_bound_value_at_depth(property, value, 0)
}

fn validate_bound_value_at_depth(
    property: &str,
    value: &Value,
    depth: usize,
) -> Result<(), SurfaceValidationError> {
    if depth > MAX_BOUND_DEPTH {
        return Err(SurfaceValidationError::SurfaceValueTooDeep);
    }
    match value {
        Value::String(value) if value.len() > MAX_RENDERED_STRING_BYTES => {
            Err(SurfaceValidationError::RenderedStringTooLong)
        }
        Value::Array(items) => {
            if items.len() > MAX_BOUND_ITEMS {
                return Err(SurfaceValidationError::BoundCollectionTooLarge {
                    property: property.into(),
                });
            }
            for item in items {
                validate_bound_value_at_depth(property, item, depth + 1)?;
            }
            Ok(())
        }
        Value::Object(fields) => {
            if fields.len() > MAX_BOUND_ITEMS {
                return Err(SurfaceValidationError::BoundCollectionTooLarge {
                    property: property.into(),
                });
            }
            for (key, item) in fields {
                if key.len() > MAX_FIELD_KEY_BYTES {
                    return Err(SurfaceValidationError::FieldKeyTooLong {
                        property: property.into(),
                    });
                }
                validate_bound_value_at_depth(property, item, depth + 1)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn validate_value_depth(value: &Value) -> Result<(), SurfaceValidationError> {
    let mut pending = vec![(value, 0_usize)];
    while let Some((value, depth)) = pending.pop() {
        if depth > MAX_BOUND_DEPTH {
            return Err(SurfaceValidationError::SurfaceValueTooDeep);
        }
        match value {
            Value::Array(items) => {
                pending.extend(items.iter().map(|item| (item, depth + 1)));
            }
            Value::Object(fields) => {
                pending.extend(fields.values().map(|item| (item, depth + 1)));
            }
            _ => {}
        }
    }
    Ok(())
}

fn is_json_pointer(path: &str) -> bool {
    path.is_empty() || (path.starts_with('/') && path.split('/').skip(1).all(is_pointer_token))
}

fn is_pointer_token(token: &str) -> bool {
    let mut chars = token.chars();
    while let Some(ch) = chars.next() {
        if ch == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}

fn allowed_properties(component: ComponentType) -> &'static [&'static str] {
    match component {
        ComponentType::DecisionMatrix => &["title", "criteria_path", "options_path"],
        ComponentType::EvidenceTable => &["title", "evidence_path"],
        ComponentType::AssumptionRegister => &["title", "assumptions_path"],
        ComponentType::PlanChecklist => &["title", "plan_path"],
        ComponentType::ApprovalGate => &["title", "action_id", "target", "expected_state"],
        ComponentType::OpenLoops => &["title", "items_path"],
        ComponentType::MetricCard => &["title", "label", "value_path", "detail_path"],
        ComponentType::ProgressBar => &["title", "label", "value_path", "max"],
        ComponentType::StatusBadge => &["title", "label", "value_path"],
        ComponentType::Callout => &["title", "message_path", "tone"],
        ComponentType::KeyValueList => &["title", "items_path"],
        ComponentType::DataTable => &["title", "rows_path", "columns"],
        ComponentType::Timeline => &["title", "items_path"],
        ComponentType::LineChart | ComponentType::BarChart => {
            &["title", "data_path", "x_key", "series"]
        }
        ComponentType::PieChart => &["title", "data_path", "name_key", "value_key"],
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SurfaceValidationError {
    #[error("unsupported catalog: {0}")]
    UnsupportedCatalog(String),
    #[error("surface id must be non-empty and at most 128 bytes")]
    InvalidSurfaceId,
    #[error("a surface may contain at most {MAX_COMPONENTS} components")]
    TooManyComponents,
    #[error("surface payload exceeds {MAX_SURFACE_BYTES} bytes")]
    PayloadTooLarge,
    #[error("invalid or duplicate component id: {0}")]
    InvalidComponentId(String),
    #[error("unsupported property {property} for component {component:?}")]
    UnsupportedProperty {
        component: ComponentType,
        property: String,
    },
    #[error("invalid JSON pointer binding for property {property}")]
    InvalidBinding { property: String },
    #[error("missing required property {property} for component {component:?}")]
    MissingProperty {
        component: ComponentType,
        property: String,
    },
    #[error("invalid property {property} for component {component:?}")]
    InvalidProperty {
        component: ComponentType,
        property: String,
    },
    #[error("bound collection at {property} exceeds {MAX_BOUND_ITEMS} items")]
    BoundCollectionTooLarge { property: String },
    #[error("table columns exceed {MAX_TABLE_COLUMNS}")]
    TooManyColumns,
    #[error("chart series must contain 1 through {MAX_CHART_SERIES} field keys")]
    TooManySeries,
    #[error("pie chart slices exceed {MAX_PIE_SLICES}")]
    TooManySlices,
    #[error("field key at {property} must contain 1 through {MAX_FIELD_KEY_BYTES} bytes")]
    FieldKeyTooLong { property: String },
    #[error("rendered string exceeds {MAX_RENDERED_STRING_BYTES} bytes")]
    RenderedStringTooLong,
    #[error("surface value exceeds {MAX_BOUND_DEPTH} nested levels")]
    SurfaceValueTooDeep,
    #[error("approval gate must reference a catalog action and string target")]
    InvalidActionReference,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn numbered_records(count: usize) -> Value {
        Value::Array(
            (0..count)
                .map(|index| json!({"name": format!("item-{index}"), "value": index}))
                .collect(),
        )
    }

    fn numbered_object(count: usize) -> Value {
        Value::Object(
            (0..count)
                .map(|index| (format!("key-{index}"), json!(format!("value-{index}"))))
                .collect(),
        )
    }

    fn valid_surface() -> WorkSurface {
        WorkSurface {
            surface_id: "decision-1".into(),
            catalog_id: ZBOT_WORK_SURFACE_CATALOG.into(),
            components: vec![SurfaceComponent {
                id: "matrix".into(),
                component_type: ComponentType::DecisionMatrix,
                props: BTreeMap::from([("criteria_path".into(), json!("/criteria"))]),
            }],
            data: json!({"criteria": []}),
        }
    }

    #[test]
    fn validates_the_initial_catalog() {
        assert_eq!(ZbotWorkSurfaceCatalog.validate(&valid_surface()), Ok(()));
    }

    #[test]
    fn rejects_unknown_catalog_and_properties() {
        let mut surface = valid_surface();
        surface.catalog_id = "unknown/v1".into();
        assert!(matches!(
            ZbotWorkSurfaceCatalog.validate(&surface),
            Err(SurfaceValidationError::UnsupportedCatalog(_))
        ));
        surface.catalog_id = ZBOT_WORK_SURFACE_CATALOG.into();
        surface.components[0]
            .props
            .insert("on_click".into(), json!("shell"));
        assert!(matches!(
            ZbotWorkSurfaceCatalog.validate(&surface),
            Err(SurfaceValidationError::UnsupportedProperty { .. })
        ));
    }

    #[test]
    fn rejects_duplicate_ids_and_oversized_component_lists() {
        let mut surface = valid_surface();
        surface.components.push(surface.components[0].clone());
        assert!(matches!(
            ZbotWorkSurfaceCatalog.validate(&surface),
            Err(SurfaceValidationError::InvalidComponentId(_))
        ));
        surface.components = (0..=MAX_COMPONENTS)
            .map(|index| SurfaceComponent {
                id: format!("component-{index}"),
                component_type: ComponentType::OpenLoops,
                props: BTreeMap::new(),
            })
            .collect();
        assert_eq!(
            ZbotWorkSurfaceCatalog.validate(&surface),
            Err(SurfaceValidationError::TooManyComponents)
        );
    }

    #[test]
    fn rejects_unsafe_bindings_and_unknown_actions() {
        let mut surface = valid_surface();
        surface.components[0]
            .props
            .insert("criteria_path".into(), json!("criteria"));
        assert!(matches!(
            ZbotWorkSurfaceCatalog.validate(&surface),
            Err(SurfaceValidationError::InvalidBinding { .. })
        ));

        surface.components[0].component_type = ComponentType::ApprovalGate;
        surface.components[0].props = BTreeMap::from([
            ("action_id".into(), json!("shell")),
            ("target".into(), json!("anything")),
        ]);
        assert_eq!(
            ZbotWorkSurfaceCatalog.validate(&surface),
            Err(SurfaceValidationError::InvalidActionReference)
        );
    }

    // STUB: AC4, AC5, AC7, AC8
    #[test]
    fn catalog_expansion_accepts_only_bounded_descriptors() {
        for component_type in [
            "MetricCard",
            "ProgressBar",
            "StatusBadge",
            "Callout",
            "KeyValueList",
            "DataTable",
            "Timeline",
            "LineChart",
            "BarChart",
            "PieChart",
        ] {
            assert!(
                serde_json::from_value::<ComponentType>(json!(component_type)).is_ok(),
                "{component_type} must be part of the portable catalog"
            );
        }

        let valid_expansion: WorkSurface = serde_json::from_value(json!({
            "surface_id": "valid-expansion",
            "catalog_id": ZBOT_WORK_SURFACE_CATALOG,
            "components": [
                {"id": "metric", "type": "MetricCard", "props": {"value_path": "/metric", "detail_path": "/detail"}},
                {"id": "progress", "type": "ProgressBar", "props": {"value_path": "/progress", "max": 100}},
                {"id": "status", "type": "StatusBadge", "props": {"value_path": "/status"}},
                {"id": "callout", "type": "Callout", "props": {"message_path": "/message", "tone": "info"}},
                {"id": "pairs", "type": "KeyValueList", "props": {"items_path": "/pairs"}},
                {"id": "table", "type": "DataTable", "props": {"rows_path": "/rows", "columns": ["name", "value"]}},
                {"id": "timeline", "type": "Timeline", "props": {"items_path": "/timeline"}},
                {"id": "line", "type": "LineChart", "props": {"data_path": "/points", "x_key": "name", "series": ["value"]}},
                {"id": "bar", "type": "BarChart", "props": {"data_path": "/points", "x_key": "name", "series": ["value"]}},
                {"id": "pie", "type": "PieChart", "props": {"data_path": "/slices", "name_key": "name", "value_key": "value"}}
            ],
            "data": {
                "metric": 42,
                "detail": "steady",
                "progress": 75,
                "status": "healthy",
                "message": "Ready",
                "pairs": {"region": "east"},
                "rows": [{"name": "api", "value": 1}],
                "timeline": [{"title": "Started"}],
                "points": [{"name": "Mon", "value": 1}],
                "slices": [{"name": "API", "value": 1}]
            }
        }))
        .expect("valid expanded descriptor must deserialize");
        assert_eq!(
            ZbotWorkSurfaceCatalog.validate(&valid_expansion),
            Ok(()),
            "every new component must accept a valid under-budget descriptor"
        );

        let invalid_contract_cases = [
            (
                "metric",
                "html",
                json!("<strong>unsafe</strong>"),
                "unsupported",
            ),
            (
                "progress",
                "value_path",
                json!("progress"),
                "invalid JSON pointer",
            ),
            ("status", "action", json!("approve"), "unsupported"),
            (
                "callout",
                "message_path",
                json!("message"),
                "invalid JSON pointer",
            ),
            (
                "pairs",
                "items_path",
                json!("pairs"),
                "invalid JSON pointer",
            ),
            ("table", "formatter", json!("javascript"), "unsupported"),
            (
                "timeline",
                "items_path",
                json!("timeline"),
                "invalid JSON pointer",
            ),
            ("line", "data_path", json!("points"), "invalid JSON pointer"),
            (
                "line",
                "series",
                json!(["value", "value"]),
                "invalid property",
            ),
            ("bar", "colors", json!(["red"]), "unsupported"),
            ("pie", "data_path", json!("slices"), "invalid JSON pointer"),
        ];
        for (component_id, property, value, expected_reason) in invalid_contract_cases {
            let mut candidate = valid_expansion.clone();
            candidate
                .components
                .iter_mut()
                .find(|component| component.id == component_id)
                .expect("fixture component must exist")
                .props
                .insert(property.into(), value);
            let error = ZbotWorkSurfaceCatalog
                .validate(&candidate)
                .expect_err("invalid expanded property must be rejected");
            assert!(
                error.to_string().contains(expected_reason),
                "{component_id}.{property} must fail for {expected_reason}, got: {error}"
            );
        }

        let mut surface = valid_surface();
        surface.components[0] = SurfaceComponent {
            id: "loops".into(),
            component_type: ComponentType::OpenLoops,
            props: BTreeMap::from([("items_path".into(), json!("/items"))]),
        };
        surface.data =
            json!({"items": (0..=200).map(|index| format!("item-{index}")).collect::<Vec<_>>()});
        assert!(
            ZbotWorkSurfaceCatalog.validate(&surface).is_ok(),
            "the original six components must retain their pre-expansion data behavior"
        );

        let mut table_with_declared_columns = valid_expansion.clone();
        table_with_declared_columns.data["rows"] = json!([{
            "name": "Ada",
            "score": 10,
            "unused-01": 1,
            "unused-02": 2,
            "unused-03": 3,
            "unused-04": 4,
            "unused-05": 5,
            "unused-06": 6,
            "unused-07": 7,
            "unused-08": 8,
            "unused-09": 9,
            "unused-10": 10,
            "unused-11": 11,
            "unused-12": 12,
            "unused-13": 13,
            "unused-14": 14,
            "unused-15": 15,
            "unused-16": 16,
            "unused-17": 17,
            "unused-18": 18,
            "unused-19": 19
        }]);
        assert!(
            ZbotWorkSurfaceCatalog
                .validate(&table_with_declared_columns)
                .is_ok(),
            "unused row fields must not count against declared DataTable columns"
        );

        let over_budget_cases = [
            (
                "bound collection",
                json!({
                    "surface_id": "timeline-budget",
                    "catalog_id": ZBOT_WORK_SURFACE_CATALOG,
                    "components": [{"id": "timeline", "type": "Timeline", "props": {"items_path": "/items"}}],
                    "data": {"items": (0..=200).map(|index| json!({"title": format!("item-{index}")})).collect::<Vec<_>>()}
                }),
            ),
            (
                "bound collection",
                json!({
                    "surface_id": "key-value-budget",
                    "catalog_id": ZBOT_WORK_SURFACE_CATALOG,
                    "components": [{"id": "pairs", "type": "KeyValueList", "props": {"items_path": "/items"}}],
                    "data": {"items": numbered_object(201)}
                }),
            ),
            (
                "bound collection",
                json!({
                    "surface_id": "table-rows-budget",
                    "catalog_id": ZBOT_WORK_SURFACE_CATALOG,
                    "components": [{"id": "table", "type": "DataTable", "props": {"rows_path": "/rows"}}],
                    "data": {"rows": numbered_records(201)}
                }),
            ),
            (
                "bound collection",
                json!({
                    "surface_id": "line-data-budget",
                    "catalog_id": ZBOT_WORK_SURFACE_CATALOG,
                    "components": [{"id": "line", "type": "LineChart", "props": {
                        "data_path": "/points", "x_key": "name", "series": ["value"]
                    }}],
                    "data": {"points": numbered_records(201)}
                }),
            ),
            (
                "bound collection",
                json!({
                    "surface_id": "bar-data-budget",
                    "catalog_id": ZBOT_WORK_SURFACE_CATALOG,
                    "components": [{"id": "bar", "type": "BarChart", "props": {
                        "data_path": "/points", "x_key": "name", "series": ["value"]
                    }}],
                    "data": {"points": numbered_records(201)}
                }),
            ),
            (
                "columns",
                json!({
                    "surface_id": "table-derived-columns-budget",
                    "catalog_id": ZBOT_WORK_SURFACE_CATALOG,
                    "components": [{"id": "table", "type": "DataTable", "props": {"rows_path": "/rows"}}],
                    "data": {"rows": [numbered_object(21)]}
                }),
            ),
            (
                "columns",
                json!({
                    "surface_id": "table-columns-budget",
                    "catalog_id": ZBOT_WORK_SURFACE_CATALOG,
                    "components": [{"id": "table", "type": "DataTable", "props": {
                        "rows_path": "/rows",
                        "columns": (0..=20).map(|index| format!("column-{index}")).collect::<Vec<_>>()
                    }}],
                    "data": {"rows": []}
                }),
            ),
            (
                "series",
                json!({
                    "surface_id": "series-budget",
                    "catalog_id": ZBOT_WORK_SURFACE_CATALOG,
                    "components": [{"id": "line", "type": "LineChart", "props": {
                        "data_path": "/points",
                        "x_key": "x",
                        "series": (0..=8).map(|index| format!("series-{index}")).collect::<Vec<_>>()
                    }}],
                    "data": {"points": []}
                }),
            ),
            (
                "slices",
                json!({
                    "surface_id": "slice-budget",
                    "catalog_id": ZBOT_WORK_SURFACE_CATALOG,
                    "components": [{"id": "pie", "type": "PieChart", "props": {
                        "data_path": "/slices",
                        "name_key": "name",
                        "value_key": "value"
                    }}],
                    "data": {"slices": (0..=50).map(|index| json!({"name": format!("slice-{index}"), "value": 1})).collect::<Vec<_>>()}
                }),
            ),
            (
                "field key",
                json!({
                    "surface_id": "field-key-budget",
                    "catalog_id": ZBOT_WORK_SURFACE_CATALOG,
                    "components": [{"id": "bar", "type": "BarChart", "props": {
                        "data_path": "/points",
                        "x_key": "x".repeat(65),
                        "series": ["value"]
                    }}],
                    "data": {"points": []}
                }),
            ),
            (
                "rendered string",
                json!({
                    "surface_id": "string-budget",
                    "catalog_id": ZBOT_WORK_SURFACE_CATALOG,
                    "components": [{"id": "metric", "type": "MetricCard", "props": {"value_path": "/value"}}],
                    "data": {"value": "x".repeat(4097)}
                }),
            ),
        ];
        for (expected_reason, descriptor) in over_budget_cases {
            let candidate: WorkSurface =
                serde_json::from_value(descriptor).expect("expanded descriptor must deserialize");
            let error = ZbotWorkSurfaceCatalog
                .validate(&candidate)
                .expect_err("over-budget descriptor must be rejected");
            assert!(
                error.to_string().contains(expected_reason),
                "{} must fail for its {expected_reason} budget, got: {error}",
                candidate.surface_id,
            );
        }

        let mut oversized = valid_surface();
        oversized.data = json!({"value": "x".repeat(MAX_SURFACE_BYTES)});
        assert_eq!(
            ZbotWorkSurfaceCatalog.validate(&oversized),
            Err(SurfaceValidationError::PayloadTooLarge)
        );
    }

    #[test]
    fn whole_surface_depth_is_bounded_before_serialization() {
        let mut nested = json!("leaf");
        for _ in 0..=MAX_BOUND_DEPTH {
            nested = Value::Array(vec![nested]);
        }
        let candidate: WorkSurface = serde_json::from_value(json!({
            "surface_id": "deep-data",
            "catalog_id": ZBOT_WORK_SURFACE_CATALOG,
            "components": [{
                "id": "callout",
                "type": "Callout",
                "props": {"message_path": "/value"}
            }],
            "data": {"value": "safe", "unreferenced": nested}
        }))
        .expect("descriptor");

        assert_eq!(
            ZbotWorkSurfaceCatalog.validate(&candidate),
            Err(SurfaceValidationError::SurfaceValueTooDeep)
        );
    }

    #[test]
    fn persisted_surface_allowlist_excludes_approval_gate() {
        // STUB: AC2/AC5 — persistence is display-only even when live catalog
        // validation permits an ApprovalGate.
        let mut display = valid_surface();
        assert!(is_persistable_surface(&display));
        display.components[0].component_type = ComponentType::ApprovalGate;
        assert!(!is_persistable_surface(&display));
    }
}
