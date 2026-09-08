//! Rig adapter boundary metadata.
//!
//! The implementation adapter lands in later migration tasks. This module owns
//! the dependency pin so Rig stays confined to `agent-runtime`.

mod checkpoint_tail;
pub mod client;
pub mod config;
mod context_inputs;
mod context_policy;
pub mod engine;
pub mod factory;
pub mod model;
mod progress_policy;
mod resources;
pub mod structured;
pub mod tool;
mod tool_hook;
mod tool_results;
mod turn_events;
mod turn_signal;

#[cfg(test)]
mod capability_tests;
#[cfg(test)]
mod mcp_capability_tests;
#[cfg(test)]
mod mcp_lifecycle_tests;

pub use client::LlmCompletionClient;
pub use config::{RigAgentConfig, RigConfigError, RigModelConfig};
pub use structured::prompt_typed;
pub use tool::{RigToolAdapter, SharedToolContext};

/// SDK tracing roots that record raw payloads before host policy hooks.
pub(crate) const PAYLOAD_DIAGNOSTIC_TARGETS: &[&str] = &["rig", "rig_core"];

// Re-exported through the adapter boundary so gateway crates can use Rig
// extractors (typed structured output) over LlmCompletionClient without
// depending on Rig directly. Rig itself stays confined to this crate.
pub use rig::client::CompletionClient;

/// Rig package source selected for the migration.
pub const RIG_REPOSITORY: &str = "https://github.com/0xplaygrounds/rig";

/// Rig Git revision selected for the migration.
pub const RIG_REVISION: &str = "6b1991bfb246411dd75839c8611e801a2309d33c";

/// Rig package version at the selected revision.
pub const RIG_VERSION: &str = "0.39.0";

/// Reviewable Rig dependency pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RigDependencyPin {
    /// Git repository URL.
    pub repository: &'static str,
    /// Exact Git revision.
    pub revision: &'static str,
    /// Crate version at the exact revision.
    pub version: &'static str,
}

/// Return the active Rig dependency pin.
#[must_use]
pub const fn dependency_pin() -> RigDependencyPin {
    RigDependencyPin {
        repository: RIG_REPOSITORY,
        revision: RIG_REVISION,
        version: RIG_VERSION,
    }
}

/// Build a Rig agent with tools and get a typed, schema-enforced response.
/// Single call: the model uses tools, then returns T via response_format.
pub async fn agent_with_tools_and_schema<T>(
    llm_client: std::sync::Arc<dyn crate::llm::LlmClient>,
    model: String,
    system_prompt: impl Into<String>,
    tools: Vec<Box<dyn rig::tool::ToolDyn>>,
    user_message: &str,
) -> Result<T, String>
where
    T: schemars::JsonSchema + serde::de::DeserializeOwned + Send + 'static,
{
    use rig::client::CompletionClient;
    use rig::completion::TypedPrompt;

    let client = LlmCompletionClient::new(llm_client);
    let agent = client
        .agent(model)
        .preamble(&system_prompt.into())
        .tools(tools)
        .default_max_turns(10)
        .build();

    agent
        .prompt_typed::<T>(user_message)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn dependency_pin_matches_manifest_decision() {
        let pin = dependency_pin();
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let manifest = fs::read_to_string(manifest_dir.join("Cargo.toml"))
            .expect("agent-runtime manifest should be readable");
        assert!(
            manifest.contains(&format!("git = \"{}\"", pin.repository)),
            "agent-runtime manifest should pin Rig repository {}",
            pin.repository
        );
        assert!(
            manifest.contains(&format!("rev = \"{}\"", pin.revision)),
            "agent-runtime manifest should pin Rig revision {}",
            pin.revision
        );
        assert!(
            manifest.contains(&format!("version = \"{}\"", pin.version)),
            "agent-runtime manifest should declare Rig version {}",
            pin.version
        );

        let workspace_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("agent-runtime should be two levels under the workspace");
        let lockfile = fs::read_to_string(workspace_root.join("Cargo.lock"))
            .expect("Cargo.lock should be readable");
        let rig_package = lockfile
            .split("[[package]]")
            .find(|section| section.contains("name = \"rig\""))
            .expect("Cargo.lock should contain the rig package");
        assert!(
            rig_package.contains(&format!("version = \"{}\"", pin.version)),
            "Cargo.lock Rig package should use version {}",
            pin.version
        );
        assert!(
            rig_package.contains(pin.repository),
            "Cargo.lock Rig package should use repository {}",
            pin.repository
        );
        assert!(
            rig_package.contains(&format!("rev={}#", pin.revision))
                && rig_package.contains(&format!("#{}", pin.revision)),
            "Cargo.lock Rig package should use exact revision {}",
            pin.revision
        );
    }

    #[test]
    fn rig_crate_is_available_to_runtime_adapter() {
        let type_name = std::any::type_name::<rig::completion::ToolDefinition>();
        assert!(type_name.contains("ToolDefinition"));
    }
}
