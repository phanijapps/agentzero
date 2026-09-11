//! Canonical example calls — the single source for the "Example:" line.
//!
//! Each tool whose description carries an example call defines it here.
//! Two consumers render the same string:
//! 1. the tool description (in-context pattern for weak-schema local models)
//! 2. the failure-feedback nudge in `agent-runtime`'s progress policy,
//!    which appends the example on the second identical failure
//!
//! Adding a tool here? Wire both consumers or the affordance silently
//! does nothing for it.

/// `ward` — the top historical fumble surface (~237 shape errors).
pub const WARD_EXAMPLE_CALL: &str = r#"{"action": "use", "name": "financial-analysis"}"#;

/// `update_plan` — plan-array shape.
pub const UPDATE_PLAN_EXAMPLE_CALL: &str =
    r#"{"plan": [{"step": "fetch data", "status": "pending"}]}"#;

/// `multimodal_analyze` — content-array shape (top coerced fumble).
pub const MULTIMODAL_EXAMPLE_CALL: &str =
    r#"{"content": [{"type": "image", "source": "/path/img.png"}], "prompt": "describe"}"#;

/// `present_surface` — component array with the required props.
pub const PRESENT_SURFACE_EXAMPLE_CALL: &str = r#"{"title": "Results", "components": [{"id": "t1", "type": "DataTable", "props": {"columns": ["a"], "rows_path": "/rows"}}]}"#;
