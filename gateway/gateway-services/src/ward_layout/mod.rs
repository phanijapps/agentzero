//! Versioned, data-only Ward Layout Contract loading.

mod create;
mod lint;
mod loader;
mod rules;
mod schema;

pub use loader::{
    load_bounded_vault_utf8_file, load_ward_agent_template, load_ward_archetype_bundle,
    load_ward_layout, load_ward_layout_bytes, seed_default_ward_agent_template,
    seed_default_ward_archetypes, seed_default_ward_layout_template, BoundedFileError, LayoutError,
    LoadedWardArchetype, LoadedWardLayout, SeedOutcome, WardStarterFile,
    MAX_WARD_ARCHETYPE_STARTER_DEPTH, MAX_WARD_ARCHETYPE_STARTER_FILES,
    MAX_WARD_ARCHETYPE_STARTER_FILE_BYTES, MAX_WARD_ARCHETYPE_STARTER_TOTAL_BYTES,
    WARD_AGENT_TEMPLATE_MAX_BYTES,
};
pub use rules::{CompiledWardLayout, NodeFormat, NodeKind, RuleError, RuleNode};
pub use schema::WardLayoutDocument;

impl WardLayoutDocument {
    pub fn body_value(&self, key: &str) -> Option<&serde_yaml::Value> {
        self.body.get(key)
    }
}
pub use create::{
    create_ward_from_archetype, create_ward_from_template, publish_tree_no_replace,
    rollback_created_ward, CreatedWard, WardCreateError,
};
pub use lint::{lint_ward, FindingCategory, LintFinding, WardLintReport};
