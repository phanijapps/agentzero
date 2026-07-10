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
        }
        Ok(())
    }
}

fn allowed_properties(component: ComponentType) -> &'static [&'static str] {
    match component {
        ComponentType::DecisionMatrix => &["title", "criteria_path", "options_path"],
        ComponentType::EvidenceTable => &["title", "evidence_path"],
        ComponentType::AssumptionRegister => &["title", "assumptions_path"],
        ComponentType::PlanChecklist => &["title", "plan_path"],
        ComponentType::ApprovalGate => &["title", "action_id", "target", "expected_state"],
        ComponentType::OpenLoops => &["title", "items_path"],
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
