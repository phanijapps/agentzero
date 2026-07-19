//! # Runner
//!
//! Session orchestration. Decomposed from a 3,067-LOC god module into
//! six focused units. **Read `AGENTS.md` in this directory before
//! adding code here.**

use std::path::Path;

use agent_runtime::{prepare_tool_result_for_context, ToolResultContextConfig};
use agent_tools::ToolSettings;

mod continuation_watcher;
pub(super) mod core;
mod delegation_dispatcher;
mod execution_stream;
mod invoke_bootstrap;
mod session_invoker;

pub use continuation_watcher::ContinuationWatcher;
pub use core::*;
pub use delegation_dispatcher::DelegationDispatcher;
#[cfg(any(test, feature = "test-stubs"))]
pub use session_invoker::StubSessionInvoker;
pub use session_invoker::{ContinuationSpawner, DelegationSpawner, SessionSpawner};

pub(crate) fn prompt_safe_tool_result_config(
    tool_settings: &ToolSettings,
    vault_dir: &Path,
) -> ToolResultContextConfig {
    ToolResultContextConfig {
        offload_large_results: tool_settings.offload_large_results,
        offload_threshold_chars: tool_settings.offload_threshold_tokens * 4,
        offload_dir: Some(vault_dir.join("temp")),
        ..ToolResultContextConfig::default()
    }
}

pub(crate) fn prompt_safe_tool_content(
    tool_name: &str,
    result: &str,
    context_result: Option<&str>,
    error: Option<&str>,
    config: &ToolResultContextConfig,
) -> String {
    if let Some(context_result) = context_result {
        return context_result.to_string();
    }

    match error {
        Some(err) => format!("Error: {}", err),
        None => prepare_tool_result_for_context(tool_name, result.to_string(), config),
    }
}

#[cfg(test)]
mod prompt_safe_tool_result_tests {
    use super::*;

    #[test]
    fn prompt_safe_tool_content_offloads_large_success_result() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let settings = ToolSettings {
            offload_large_results: true,
            offload_threshold_tokens: 1,
            ..ToolSettings::default()
        };
        let config = prompt_safe_tool_result_config(&settings, tmp.path());
        let raw = "large-result-body".repeat(100);

        let content = prompt_safe_tool_content("shell/bad:name", &raw, None, None, &config);

        assert!(content.contains("Tool result was too large for context"));
        assert!(!content.contains(&raw));
        let offload_dir = tmp.path().join("temp");
        let entries: Vec<_> = std::fs::read_dir(&offload_dir)
            .expect("offload dir exists")
            .collect::<Result<_, _>>()
            .expect("read offload entries");
        assert_eq!(entries.len(), 1);
        let saved = std::fs::read_to_string(entries[0].path()).expect("read offload file");
        assert_eq!(saved, raw);
    }

    #[test]
    fn prompt_safe_tool_content_preserves_error_shape() {
        let config = ToolResultContextConfig::default();

        let content = prompt_safe_tool_content("shell", "", None, Some("boom"), &config);

        assert_eq!(content, "Error: boom");
    }

    #[test]
    fn prompt_safe_tool_content_prefers_runtime_context_result() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let settings = ToolSettings {
            offload_large_results: true,
            offload_threshold_tokens: 1,
            ..ToolSettings::default()
        };
        let config = prompt_safe_tool_result_config(&settings, tmp.path());
        let raw = "large-result-body".repeat(100);

        let content =
            prompt_safe_tool_content("shell", &raw, Some("already prompt-safe"), None, &config);

        assert_eq!(content, "already prompt-safe");
        assert!(!tmp.path().join("temp").exists());
    }
}
