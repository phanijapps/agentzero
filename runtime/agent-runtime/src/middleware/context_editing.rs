// ============================================================================
// CONTEXT EDITING MIDDLEWARE
// Manage conversation context by clearing older tool call outputs
//
// Inspired by LangChain's context editing middleware:
// https://docs.langchain.com/oss/javascript/langchain/middleware/built-in#context-editing
// ============================================================================

//! # Context Editing Middleware
//!
//! Manage conversation context by clearing older tool call outputs.

use crate::middleware::config::ContextEditingConfig;
use crate::middleware::token_counter::{estimate_message_tokens, estimate_total_tokens};
use crate::middleware::traits::{
    ExecutionState, MiddlewareContext, MiddlewareEffect, PreProcessMiddleware,
};
use crate::types::{ChatMessage, StreamEvent, ToolCall};
use agent_primitives::types::Part;
use serde_json::json;

/// Compress an assistant message into a one-line summary.
///
/// Extracts tool names and file paths from `tool_calls` arguments.
/// Pure pattern matching — no LLM call.
///
/// Format: `[Turn N: tool1(file1), tool2(file2)]` or `[Turn N: <truncated reasoning>]`
#[cfg(test)]
pub(crate) fn compress_assistant_message(msg: &ChatMessage, turn_number: usize) -> String {
    if let Some(ref tool_calls) = msg.tool_calls {
        let summaries: Vec<String> = tool_calls
            .iter()
            .map(|tc| {
                let path = extract_file_path(&tc.arguments);
                match path {
                    Some(p) => format!("{}({})", tc.name, p),
                    None => tc.name.clone(),
                }
            })
            .collect();

        format!("[Turn {}: {}]", turn_number, summaries.join(", "))
    } else if !msg.content.is_empty() {
        let text = msg.text_content();
        let truncated: String = text.chars().take(60).collect();
        let ellipsis = if text.len() > 60 { "..." } else { "" };
        format!("[Turn {turn_number}: {truncated}{ellipsis}]")
    } else {
        format!("[Turn {turn_number}]")
    }
}

/// Extract a file path from tool call arguments.
///
/// Looks for common keys: "path", "`file_path`", "file", "filename".
#[cfg(test)]
pub(crate) fn extract_file_path(args: &serde_json::Value) -> Option<String> {
    let path_keys = ["path", "file_path", "file", "filename"];
    if let Some(obj) = args.as_object() {
        for key in &path_keys {
            if let Some(val) = obj.get(*key) {
                if let Some(s) = val.as_str() {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

/// Compress old assistant messages in-place.
///
/// Messages in the "old" portion (before `keep_recent`) are compressed to
/// one-line summaries. Recent messages are left intact.
///
/// `keep_recent` is the number of messages from the end to preserve unchanged.
#[cfg(test)]
pub(crate) fn compress_old_assistant_messages(messages: &mut [ChatMessage], keep_recent: usize) {
    let total = messages.len();
    if total <= keep_recent {
        return;
    }

    let compress_boundary = total.saturating_sub(keep_recent);
    let mut turn_counter = 0;

    for message in &mut messages[..compress_boundary] {
        if message.role == "assistant" {
            turn_counter += 1;

            // Skip already-compressed messages. The `is_summary` flag is
            // authoritative — set by `compress_assistant_message` on the
            // output. The earlier implementation sniffed
            // `text.starts_with("[Turn ")` which broke if the summary
            // prefix ever changed.
            if message.is_summary {
                continue;
            }

            let compressed = compress_assistant_message(message, turn_counter);
            message.content = vec![Part::Text { text: compressed }];
            message.is_summary = true;
            // Keep tool_calls intact — the LLM API requires them for tool result pairing
        }
    }
}

/// Build an index mapping `tool_call_id` → `tool_name` from all assistant messages.
/// This is O(n) over messages, enabling O(1) lookups instead of O(n) backwards search.
fn build_tool_call_index(messages: &[ChatMessage]) -> std::collections::HashMap<String, String> {
    let mut index = std::collections::HashMap::new();
    for msg in messages {
        if msg.role == "assistant" {
            if let Some(ref tool_calls) = msg.tool_calls {
                for tc in tool_calls {
                    index.insert(tc.id.clone(), tc.name.clone());
                }
            }
        }
    }
    index
}

/// Context editing middleware
///
/// Clears older tool call outputs when token limits are reached,
/// while preserving the most recent tool results.
pub struct ContextEditingMiddleware {
    /// Configuration
    config: ContextEditingConfig,
}

impl ContextEditingMiddleware {
    /// Create a new context editing middleware
    #[must_use]
    pub fn new(config: ContextEditingConfig) -> Self {
        Self { config }
    }

    /// Find tool result messages that should be cleared
    #[allow(dead_code)]
    fn find_tool_results_to_clear(&self, messages: &[ChatMessage]) -> Vec<usize> {
        self.find_tool_results_to_clear_with_cascade(messages, &ExecutionState::default())
    }

    /// Find tool result messages that should be cleared, with cascade unloading for skills.
    ///
    /// When a skill's SKILL.md is selected for clearing, all of its resource files
    /// are also cleared (cascade unload) if `cascade_unload` is enabled. This ensures
    /// skills and their resources are treated as a unit.
    fn find_tool_results_to_clear_with_cascade(
        &self,
        messages: &[ChatMessage],
        execution_state: &ExecutionState,
    ) -> Vec<usize> {
        let tool_call_index = build_tool_call_index(messages);
        let mut tool_result_indices = Vec::new();
        let mut tool_call_id_to_idx: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        // Find all tool role messages (these contain tool results)
        for (idx, message) in messages.iter().enumerate() {
            if message.role == "tool" {
                // Check if this tool is in the exclude list
                if let Some(tool_call_id) = &message.tool_call_id {
                    // Find the corresponding tool call to get the tool name
                    let tool_name = tool_call_index.get(tool_call_id).cloned();

                    if let Some(name) = tool_name {
                        if self.config.exclude_tools.contains(&name) {
                            continue; // Don't clear this tool
                        }
                    }

                    // Track the mapping from tool_call_id to message index
                    tool_call_id_to_idx.insert(tool_call_id.clone(), idx);
                }

                tool_result_indices.push(idx);
            }
        }

        // Determine how many to keep vs clear (basic logic)
        let to_keep = self.config.keep_tool_results;

        let mut indices_to_clear = if tool_result_indices.len() > to_keep {
            // Clear all but the most recent N tool results
            tool_result_indices[..tool_result_indices.len() - to_keep].to_vec()
        } else {
            return Vec::new(); // Nothing to clear
        };

        // Apply cascade unloading for skills (only if cascade_unload is enabled):
        // For any skill that will have its main SKILL.md cleared, also clear all its resources
        if self.config.cascade_unload {
            let mut cascade_indices: Vec<usize> = Vec::new();

            for &idx in &indices_to_clear {
                if let Some(message) = messages.get(idx) {
                    if let Some(tool_call_id) = &message.tool_call_id {
                        // Check if this is a main skill load
                        for (skill_name, info) in &execution_state.loaded_skills {
                            if &info.tool_call_id == tool_call_id {
                                // This skill's SKILL.md is being cleared - cascade to resources
                                tracing::debug!(
                                    "Cascade unloading resources for skill '{}' ({} resources)",
                                    skill_name,
                                    info.resource_tool_call_ids.len()
                                );

                                for resource_id in &info.resource_tool_call_ids {
                                    if let Some(&resource_idx) =
                                        tool_call_id_to_idx.get(resource_id)
                                    {
                                        // Only add if not already in the clear list
                                        if !indices_to_clear.contains(&resource_idx)
                                            && !cascade_indices.contains(&resource_idx)
                                        {
                                            cascade_indices.push(resource_idx);
                                        }
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
            }

            // Merge cascade indices into the clear list
            indices_to_clear.extend(cascade_indices);

            // Sort by index to maintain order
            indices_to_clear.sort_unstable();
            indices_to_clear.dedup();
        }

        indices_to_clear
    }

    /// Clear tool results by replacing content with placeholder.
    /// Uses skill-aware placeholders when the tool result is from a skill load,
    /// respecting the `skill_aware_placeholders` config option.
    fn clear_tool_results(
        &self,
        messages: &mut [ChatMessage],
        indices_to_clear: &[usize],
        execution_state: &ExecutionState,
    ) -> Vec<String> {
        let mut unloaded_skills = Vec::new();

        for idx in indices_to_clear {
            if let Some(message) = messages.get_mut(*idx) {
                if message.role == "tool" {
                    let tool_call_id = message.tool_call_id.as_deref().unwrap_or("");

                    // Check if this is a skill-related tool call (only if skill_aware_placeholders is enabled)
                    if self.config.skill_aware_placeholders {
                        if let Some((skill_name, is_main_skill)) =
                            self.find_skill_for_tool_call(tool_call_id, execution_state)
                        {
                            if is_main_skill {
                                // This is a SKILL.md load - use skill-specific placeholder
                                message.content = vec![Part::Text {
                                    text: self.format_skill_placeholder(&skill_name),
                                }];
                                unloaded_skills.push(skill_name);
                            } else {
                                // This is a resource file under a skill
                                message.content = vec![Part::Text {
                                    text: self.format_resource_placeholder(&skill_name),
                                }];
                            }
                            // Optionally clear tool call inputs from the assistant message
                            if self.config.clear_tool_inputs {
                                self.clear_tool_call_inputs(messages, *idx);
                            }
                            continue;
                        }
                    }

                    // Regular tool result or skill_aware_placeholders is disabled - use generic placeholder
                    message.content = vec![Part::Text {
                        text: self.config.placeholder.clone(),
                    }];

                    // Optionally clear tool call inputs from the assistant message
                    if self.config.clear_tool_inputs {
                        self.clear_tool_call_inputs(messages, *idx);
                    }
                }
            }
        }

        unloaded_skills
    }

    /// Format skill placeholder message using custom template if provided.
    fn format_skill_placeholder(&self, skill_name: &str) -> String {
        if let Some(template) = &self.config.skill_placeholder_template {
            template.replace("{skill_name}", skill_name)
        } else {
            format!(
                "[Skill '{skill_name}' was loaded and unloaded. Reload with load_skill(skill=\"{skill_name}\") if needed.]"
            )
        }
    }

    /// Format resource placeholder message using custom template if provided.
    fn format_resource_placeholder(&self, skill_name: &str) -> String {
        if let Some(template) = &self.config.resource_placeholder_template {
            template.replace("{skill_name}", skill_name)
        } else {
            format!("[Resource from skill '{skill_name}' was unloaded.]")
        }
    }

    /// Find if a tool call ID corresponds to a skill or skill resource.
    /// Returns (`skill_name`, `is_main_skill`) where `is_main_skill` is true for SKILL.md loads.
    fn find_skill_for_tool_call(
        &self,
        tool_call_id: &str,
        execution_state: &ExecutionState,
    ) -> Option<(String, bool)> {
        for (skill_name, info) in &execution_state.loaded_skills {
            // Check if it's the main skill load
            if info.tool_call_id == tool_call_id {
                return Some((skill_name.clone(), true));
            }
            // Check if it's a resource under this skill
            if info
                .resource_tool_call_ids
                .contains(&tool_call_id.to_string())
            {
                return Some((skill_name.clone(), false));
            }
        }
        None
    }

    /// Clear tool call inputs (arguments) from assistant messages
    fn clear_tool_call_inputs(&self, messages: &mut [ChatMessage], tool_result_idx: usize) {
        // First, extract the tool_call_id we need to find
        let tool_call_id_to_clear = messages
            .get(tool_result_idx)
            .and_then(|msg| msg.tool_call_id.as_ref())
            .cloned();

        if let Some(tool_call_id) = tool_call_id_to_clear {
            // Now we can mutably iterate through messages
            for message in messages.iter_mut() {
                if let Some(tool_calls) = &mut message.tool_calls {
                    for tc in tool_calls.iter_mut() {
                        if tc.id == tool_call_id {
                            // Replace with a new tool call with empty arguments
                            *tc = ToolCall::new(tc.id.clone(), tc.name.clone(), json!({}));
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Calculate how many tokens would be reclaimed
    fn calculate_tokens_to_reclaim(
        &self,
        messages: &[ChatMessage],
        indices: &[usize],
        model: &str,
    ) -> usize {
        indices
            .iter()
            .filter_map(|idx| messages.get(*idx))
            .map(|m| estimate_message_tokens(m, model))
            .sum()
    }
}

impl Default for ContextEditingMiddleware {
    fn default() -> Self {
        Self::new(ContextEditingConfig::default())
    }
}

#[async_trait::async_trait]
impl PreProcessMiddleware for ContextEditingMiddleware {
    fn name(&self) -> &'static str {
        "context_editing"
    }

    fn clone_box(&self) -> Box<dyn PreProcessMiddleware> {
        Box::new(Self {
            config: self.config.clone(),
        })
    }

    fn enabled(&self) -> bool {
        self.config.enabled
    }

    async fn process(
        &self,
        messages: Vec<ChatMessage>,
        context: &MiddlewareContext,
    ) -> Result<MiddlewareEffect, String> {
        // Check if we should trigger context editing
        let current_tokens = estimate_total_tokens(&messages, &context.model);

        if current_tokens < self.config.trigger_tokens {
            return Ok(MiddlewareEffect::Proceed);
        }

        // Find tool results to clear (with cascade unloading for skills and their resources)
        let indices_to_clear =
            self.find_tool_results_to_clear_with_cascade(&messages, &context.execution_state);

        if indices_to_clear.is_empty() {
            return Ok(MiddlewareEffect::Proceed);
        }

        // Check if we meet the minimum reclaim threshold
        let tokens_to_reclaim =
            self.calculate_tokens_to_reclaim(&messages, &indices_to_clear, &context.model);
        if tokens_to_reclaim < self.config.min_reclaim {
            return Ok(MiddlewareEffect::Proceed);
        }

        // Take ownership of messages (no clone needed — `messages` is already owned)
        let mut modified_messages = messages;

        // Clear the tool results (skill-aware: uses meaningful placeholders for skill loads)
        let unloaded_skills = self.clear_tool_results(
            &mut modified_messages,
            &indices_to_clear,
            &context.execution_state,
        );

        // Log the context editing action
        if unloaded_skills.is_empty() {
            tracing::info!(
                agent_id = %context.agent_id,
                cleared_count = indices_to_clear.len(),
                tokens_reclaimed = tokens_to_reclaim,
                "Context editing: cleared tool results"
            );
        } else {
            tracing::info!(
                agent_id = %context.agent_id,
                cleared_count = indices_to_clear.len(),
                skills_unloaded = ?unloaded_skills,
                tokens_reclaimed = tokens_to_reclaim,
                "Context editing: cleared tool results including skills"
            );
        }

        // Create event about context editing
        let event_content = if unloaded_skills.is_empty() {
            format!(
                "[Cleared {} tool results (reclaimed ~{} tokens)]",
                indices_to_clear.len(),
                tokens_to_reclaim
            )
        } else {
            format!(
                "[Cleared {} tool results including {} skill(s): {} (reclaimed ~{} tokens)]",
                indices_to_clear.len(),
                unloaded_skills.len(),
                unloaded_skills.join(", "),
                tokens_to_reclaim
            )
        };

        let event = StreamEvent::Token {
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            content: event_content,
        };

        Ok(MiddlewareEffect::EmitAndModify {
            event,
            messages: modified_messages,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_messages_with_tool_calls() -> Vec<ChatMessage> {
        let tool1 = ToolCall::new(
            "call_1".to_string(),
            "search".to_string(),
            json!({"query": "test"}),
        );

        let tool2 = ToolCall::new(
            "call_2".to_string(),
            "calculator".to_string(),
            json!({"expression": "1+1"}),
        );

        vec![
            ChatMessage {
                role: "user".to_string(),
                content: vec![Part::Text { text: "Search for something".to_string() }],
                tool_calls: None,
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text { text: String::new() }],
                tool_calls: Some(vec![tool1, tool2]),
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text { text: "Search result: lots of text here that should be cleared when context editing kicks in".to_string() }],
                tool_calls: None,
                tool_call_id: Some("call_1".to_string()),
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text { text: "2".to_string() }],
                tool_calls: None,
                tool_call_id: Some("call_2".to_string()),
                is_summary: false,
            },
        ]
    }

    #[test]
    fn test_tool_call_id_index_lookup() {
        let messages = create_test_messages_with_tool_calls();
        let index = build_tool_call_index(&messages);
        assert_eq!(index.get("call_1"), Some(&"search".to_string()));
        assert_eq!(index.get("call_2"), Some(&"calculator".to_string()));
        assert_eq!(index.get("nonexistent"), None);
    }

    #[test]
    fn test_find_tool_results_to_clear() {
        let config = ContextEditingConfig {
            enabled: true,
            trigger_tokens: 100,
            keep_tool_results: 1,
            min_reclaim: 0,
            clear_tool_inputs: false,
            exclude_tools: vec![],
            placeholder: "[cleared]".to_string(),
            ..Default::default()
        };

        let middleware = ContextEditingMiddleware::new(config);
        let messages = create_test_messages_with_tool_calls();

        let indices = middleware.find_tool_results_to_clear(&messages);
        // Should clear the first tool result, keep the last one
        assert_eq!(indices.len(), 1);
        assert_eq!(indices[0], 2); // Index of first tool result
    }

    #[test]
    fn test_exclude_tools() {
        let config = ContextEditingConfig {
            enabled: true,
            trigger_tokens: 100,
            keep_tool_results: 0, // Clear all
            min_reclaim: 0,
            clear_tool_inputs: false,
            exclude_tools: vec!["search".to_string()], // Don't clear search results
            placeholder: "[cleared]".to_string(),
            ..Default::default()
        };

        let middleware = ContextEditingMiddleware::new(config);
        let messages = create_test_messages_with_tool_calls();

        let indices = middleware.find_tool_results_to_clear(&messages);
        // Should only clear calculator, not search
        assert_eq!(indices.len(), 1);
        assert_eq!(indices[0], 3); // Index of calculator result
    }

    #[test]
    fn test_exclude_load_skill_preserves_skill_results() {
        let skill_tool_call = ToolCall::new(
            "call_skill".to_string(),
            "load_skill".to_string(),
            json!({"skill": "rust-development"}),
        );
        let search_tool_call = ToolCall::new(
            "call_search".to_string(),
            "search".to_string(),
            json!({"query": "rust ownership"}),
        );
        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text {
                    text: String::new(),
                }],
                tool_calls: Some(vec![skill_tool_call, search_tool_call]),
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: "# Rust Development\n\nUse ownership-aware patterns.".to_string(),
                }],
                tool_calls: None,
                tool_call_id: Some("call_skill".to_string()),
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: "Search result body".to_string(),
                }],
                tool_calls: None,
                tool_call_id: Some("call_search".to_string()),
                is_summary: false,
            },
        ];
        let config = ContextEditingConfig {
            enabled: true,
            trigger_tokens: 100,
            keep_tool_results: 0,
            min_reclaim: 0,
            exclude_tools: vec!["load_skill".to_string()],
            ..Default::default()
        };
        let middleware = ContextEditingMiddleware::new(config);

        let indices = middleware.find_tool_results_to_clear(&messages);

        assert_eq!(indices, vec![2]);
    }

    #[test]
    fn test_clear_tool_results() {
        let config = ContextEditingConfig {
            enabled: true,
            trigger_tokens: 100,
            keep_tool_results: 1,
            min_reclaim: 0,
            clear_tool_inputs: false,
            exclude_tools: vec![],
            placeholder: "[cleared]".to_string(),
            ..Default::default()
        };

        let middleware = ContextEditingMiddleware::new(config);
        let mut messages = create_test_messages_with_tool_calls();
        let execution_state = ExecutionState::default(); // No skills loaded

        let indices = middleware.find_tool_results_to_clear(&messages);
        middleware.clear_tool_results(&mut messages, &indices, &execution_state);

        // Check that the first tool result was cleared
        assert_eq!(messages[2].text_content(), "[cleared]");
        // Check that the second tool result was NOT cleared
        assert_eq!(messages[3].text_content(), "2");
    }

    #[tokio::test]
    async fn process_clears_tool_results_without_compressing_assistant_prose() {
        let config = ContextEditingConfig {
            enabled: true,
            trigger_tokens: 1,
            keep_tool_results: 1,
            min_reclaim: 0,
            clear_tool_inputs: false,
            exclude_tools: vec![],
            placeholder: "[cleared]".to_string(),
            ..Default::default()
        };
        let middleware = ContextEditingMiddleware::new(config);
        let messages = vec![
            ChatMessage::user("Please inspect two files.".to_string()),
            ChatMessage::assistant(
                "I will inspect the first file and then compare it with the second.".to_string(),
            ),
            ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text {
                    text: String::new(),
                }],
                tool_calls: Some(vec![
                    ToolCall::new(
                        "call_1".to_string(),
                        "read_file".to_string(),
                        json!({"path": "src/old.rs"}),
                    ),
                    ToolCall::new(
                        "call_2".to_string(),
                        "read_file".to_string(),
                        json!({"path": "src/recent.rs"}),
                    ),
                ]),
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: "old file contents".repeat(50),
                }],
                tool_calls: None,
                tool_call_id: Some("call_1".to_string()),
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: "recent file contents".to_string(),
                }],
                tool_calls: None,
                tool_call_id: Some("call_2".to_string()),
                is_summary: false,
            },
        ];
        let ctx = make_ctx("gpt-4o-mini");

        let effect = middleware.process(messages, &ctx).await.unwrap();

        match effect {
            MiddlewareEffect::EmitAndModify { messages, .. } => {
                assert_eq!(
                    messages[1].text_content(),
                    "I will inspect the first file and then compare it with the second."
                );
                assert!(!messages[1].is_summary);
                assert_eq!(messages[3].text_content(), "[cleared]");
                assert_eq!(messages[4].text_content(), "recent file contents");
            }
            other => panic!("expected EmitAndModify, got {other:?}"),
        }
    }

    #[test]
    fn test_clear_skill_results_with_placeholder() {
        use crate::middleware::traits::SkillInfo;
        use std::collections::HashMap;

        let config = ContextEditingConfig {
            enabled: true,
            trigger_tokens: 100,
            keep_tool_results: 0, // Clear all
            min_reclaim: 0,
            clear_tool_inputs: false,
            exclude_tools: vec![],
            placeholder: "[cleared]".to_string(),
            ..Default::default()
        };

        // Create messages with a skill load
        let skill_tool_call = ToolCall::new(
            "call_skill_1".to_string(),
            "load_skill".to_string(),
            json!({"skill": "rust-development"}),
        );

        let messages_with_skill = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text {
                    text: String::new(),
                }],
                tool_calls: Some(vec![skill_tool_call]),
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: "# Rust Development\n\nLots of instructions here...".to_string(),
                }],
                tool_calls: None,
                tool_call_id: Some("call_skill_1".to_string()),
                is_summary: false,
            },
        ];

        // Create execution state that knows about the skill
        let mut loaded_skills = HashMap::new();
        loaded_skills.insert(
            "rust-development".to_string(),
            SkillInfo {
                name: "rust-development".to_string(),
                tool_call_id: "call_skill_1".to_string(),
                resource_tool_call_ids: vec![],
            },
        );
        let execution_state = ExecutionState { loaded_skills };

        let middleware = ContextEditingMiddleware::new(config);
        let mut messages = messages_with_skill;

        let indices = middleware.find_tool_results_to_clear(&messages);
        middleware.clear_tool_results(&mut messages, &indices, &execution_state);

        // Check that the skill result has a skill-specific placeholder
        assert!(messages[1].text_content().contains("rust-development"));
        assert!(messages[1].text_content().contains("load_skill"));
    }

    #[test]
    fn test_skill_aware_placeholders_disabled() {
        use crate::middleware::traits::SkillInfo;
        use std::collections::HashMap;

        // Disable skill-aware placeholders
        let config = ContextEditingConfig {
            enabled: true,
            trigger_tokens: 100,
            keep_tool_results: 0,
            skill_aware_placeholders: false, // Disabled!
            placeholder: "[generic placeholder]".to_string(),
            ..Default::default()
        };

        let skill_tool_call = ToolCall::new(
            "call_skill_1".to_string(),
            "load_skill".to_string(),
            json!({"skill": "rust-development"}),
        );

        let messages_with_skill = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text {
                    text: String::new(),
                }],
                tool_calls: Some(vec![skill_tool_call]),
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: "# Rust Development\n\nContent...".to_string(),
                }],
                tool_calls: None,
                tool_call_id: Some("call_skill_1".to_string()),
                is_summary: false,
            },
        ];

        let mut loaded_skills = HashMap::new();
        loaded_skills.insert(
            "rust-development".to_string(),
            SkillInfo {
                name: "rust-development".to_string(),
                tool_call_id: "call_skill_1".to_string(),
                resource_tool_call_ids: vec![],
            },
        );
        let execution_state = ExecutionState { loaded_skills };

        let middleware = ContextEditingMiddleware::new(config);
        let mut messages = messages_with_skill;

        let indices = middleware.find_tool_results_to_clear(&messages);
        middleware.clear_tool_results(&mut messages, &indices, &execution_state);

        // Should use generic placeholder, not skill-specific
        assert_eq!(messages[1].text_content(), "[generic placeholder]");
        assert!(!messages[1].text_content().contains("rust-development"));
    }

    #[test]
    fn test_custom_skill_placeholder_template() {
        use crate::middleware::traits::SkillInfo;
        use std::collections::HashMap;

        let config = ContextEditingConfig {
            enabled: true,
            trigger_tokens: 100,
            keep_tool_results: 0,
            skill_aware_placeholders: true,
            skill_placeholder_template: Some("[SKILL UNLOADED: {skill_name}]".to_string()),
            ..Default::default()
        };

        let skill_tool_call = ToolCall::new(
            "call_skill_1".to_string(),
            "load_skill".to_string(),
            json!({"skill": "my-skill"}),
        );

        let messages_with_skill = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text {
                    text: String::new(),
                }],
                tool_calls: Some(vec![skill_tool_call]),
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: "# My Skill\n\nContent...".to_string(),
                }],
                tool_calls: None,
                tool_call_id: Some("call_skill_1".to_string()),
                is_summary: false,
            },
        ];

        let mut loaded_skills = HashMap::new();
        loaded_skills.insert(
            "my-skill".to_string(),
            SkillInfo {
                name: "my-skill".to_string(),
                tool_call_id: "call_skill_1".to_string(),
                resource_tool_call_ids: vec![],
            },
        );
        let execution_state = ExecutionState { loaded_skills };

        let middleware = ContextEditingMiddleware::new(config);
        let mut messages = messages_with_skill;

        let indices = middleware.find_tool_results_to_clear(&messages);
        middleware.clear_tool_results(&mut messages, &indices, &execution_state);

        // Should use custom template
        assert_eq!(messages[1].text_content(), "[SKILL UNLOADED: my-skill]");
    }

    #[test]
    fn test_cascade_unload_disabled() {
        use crate::middleware::traits::SkillInfo;
        use std::collections::HashMap;

        // Disable cascade unload
        let config = ContextEditingConfig {
            enabled: true,
            trigger_tokens: 100,
            keep_tool_results: 1,  // Keep only the last one
            cascade_unload: false, // Disabled!
            ..Default::default()
        };

        // Skill load followed by resource load
        let skill_tool_call = ToolCall::new(
            "call_skill_1".to_string(),
            "load_skill".to_string(),
            json!({"skill": "my-skill"}),
        );
        let resource_tool_call = ToolCall::new(
            "call_resource_1".to_string(),
            "load_skill".to_string(),
            json!({"file": "@skill:my-skill/example.rs"}),
        );

        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text {
                    text: String::new(),
                }],
                tool_calls: Some(vec![skill_tool_call]),
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: "# My Skill Content".to_string(),
                }],
                tool_calls: None,
                tool_call_id: Some("call_skill_1".to_string()),
                is_summary: false,
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text {
                    text: String::new(),
                }],
                tool_calls: Some(vec![resource_tool_call]),
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: "fn example() { }".to_string(),
                }],
                tool_calls: None,
                tool_call_id: Some("call_resource_1".to_string()),
                is_summary: false,
            },
        ];

        let mut loaded_skills = HashMap::new();
        loaded_skills.insert(
            "my-skill".to_string(),
            SkillInfo {
                name: "my-skill".to_string(),
                tool_call_id: "call_skill_1".to_string(),
                resource_tool_call_ids: vec!["call_resource_1".to_string()],
            },
        );
        let execution_state = ExecutionState { loaded_skills };

        let middleware = ContextEditingMiddleware::new(config);

        // With cascade disabled, only the first tool result should be marked for clearing
        // (because keep_tool_results=1 keeps the last one)
        let indices =
            middleware.find_tool_results_to_clear_with_cascade(&messages, &execution_state);

        // Should only clear index 1 (the skill), not cascade to index 3 (the resource)
        assert_eq!(indices.len(), 1);
        assert!(indices.contains(&1)); // Skill at index 1
        assert!(!indices.contains(&3)); // Resource at index 3 should NOT be cascaded
    }

    #[test]
    fn test_cascade_unload_enabled() {
        use crate::middleware::traits::SkillInfo;
        use std::collections::HashMap;

        // Enable cascade unload (default)
        let config = ContextEditingConfig {
            enabled: true,
            trigger_tokens: 100,
            keep_tool_results: 0, // Clear all
            cascade_unload: true,
            ..Default::default()
        };

        // Skill load followed by resource load, then another tool
        let skill_tool_call = ToolCall::new(
            "call_skill_1".to_string(),
            "load_skill".to_string(),
            json!({"skill": "my-skill"}),
        );
        let resource_tool_call = ToolCall::new(
            "call_resource_1".to_string(),
            "load_skill".to_string(),
            json!({"file": "@skill:my-skill/example.rs"}),
        );

        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text {
                    text: String::new(),
                }],
                tool_calls: Some(vec![skill_tool_call]),
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: "# My Skill Content".to_string(),
                }],
                tool_calls: None,
                tool_call_id: Some("call_skill_1".to_string()),
                is_summary: false,
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text {
                    text: String::new(),
                }],
                tool_calls: Some(vec![resource_tool_call]),
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: "fn example() { }".to_string(),
                }],
                tool_calls: None,
                tool_call_id: Some("call_resource_1".to_string()),
                is_summary: false,
            },
        ];

        let mut loaded_skills = HashMap::new();
        loaded_skills.insert(
            "my-skill".to_string(),
            SkillInfo {
                name: "my-skill".to_string(),
                tool_call_id: "call_skill_1".to_string(),
                resource_tool_call_ids: vec!["call_resource_1".to_string()],
            },
        );
        let execution_state = ExecutionState { loaded_skills };

        let middleware = ContextEditingMiddleware::new(config);

        // With cascade enabled, both skill and resource should be cleared
        let indices =
            middleware.find_tool_results_to_clear_with_cascade(&messages, &execution_state);

        // Should clear both the skill (index 1) and the resource (index 3)
        assert_eq!(indices.len(), 2);
        assert!(indices.contains(&1)); // Skill at index 1
        assert!(indices.contains(&3)); // Resource at index 3 cascaded
    }

    #[test]
    fn test_compress_assistant_message_with_tool_calls() {
        let tool1 = ToolCall::new(
            "call_a".to_string(),
            "write_file".to_string(),
            json!({"path": "src/main.rs", "content": "fn main() {}"}),
        );
        let tool2 = ToolCall::new(
            "call_b".to_string(),
            "read_file".to_string(),
            json!({"path": "src/lib.rs"}),
        );
        let msg = ChatMessage {
            role: "assistant".to_string(),
            content: vec![Part::Text {
                text: "Let me create the main file and read the lib file for context.".to_string(),
            }],
            tool_calls: Some(vec![tool1, tool2]),
            tool_call_id: None,
            is_summary: false,
        };

        let compressed = compress_assistant_message(&msg, 3);
        assert!(compressed.starts_with("[Turn 3:"));
        assert!(compressed.contains("write_file"));
        assert!(compressed.contains("read_file"));
        assert!(compressed.contains("src/main.rs"));
        assert!(compressed.contains("src/lib.rs"));
        assert!(compressed.len() < msg.content.len() + 200);
    }

    #[test]
    fn test_compress_assistant_message_no_tool_calls() {
        let msg = ChatMessage {
            role: "assistant".to_string(),
            content: vec![Part::Text { text: "I'll help you with that. Let me think about the best approach for implementing this feature. We need to consider several factors including performance and maintainability.".to_string() }],
            tool_calls: None,
            tool_call_id: None,
            is_summary: false,
        };

        let compressed = compress_assistant_message(&msg, 5);
        assert!(compressed.starts_with("[Turn 5:"));
        assert!(compressed.len() < 100);
    }

    #[test]
    fn test_compress_old_assistant_messages() {
        let tool1 = ToolCall::new(
            "call_1".to_string(),
            "write_file".to_string(),
            json!({"path": "src/main.rs", "content": "lots of code"}),
        );
        let tool2 = ToolCall::new(
            "call_2".to_string(),
            "read_file".to_string(),
            json!({"path": "src/lib.rs"}),
        );

        let mut messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: vec![Part::Text { text: "Create the files".to_string() }],
                tool_calls: None,
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text { text: "I'll create main.rs and read lib.rs for you. Let me start with the main file.".to_string() }],
                tool_calls: Some(vec![tool1, tool2]),
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text { text: "[cleared]".to_string() }],
                tool_calls: None,
                tool_call_id: Some("call_1".to_string()),
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text { text: "[cleared]".to_string() }],
                tool_calls: None,
                tool_call_id: Some("call_2".to_string()),
                is_summary: false,
            },
            ChatMessage {
                role: "user".to_string(),
                content: vec![Part::Text { text: "Now add tests".to_string() }],
                tool_calls: None,
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text { text: "I'll add tests now.".to_string() }],
                tool_calls: None,
                tool_call_id: None,
                is_summary: false,
            },
        ];

        let original_assistant_content = messages[1].text_content();
        compress_old_assistant_messages(&mut messages, 4);

        // Old assistant message (index 1) should be compressed
        assert!(messages[1].text_content().starts_with("[Turn"));
        assert!(messages[1].text_content().contains("write_file"));
        assert!(messages[1].text_content() != original_assistant_content);

        // Recent assistant message (index 5) should NOT be compressed
        assert_eq!(messages[5].text_content(), "I'll add tests now.");
    }

    #[test]
    fn test_compress_extracts_file_paths() {
        let tool = ToolCall::new(
            "call_x".to_string(),
            "edit_file".to_string(),
            json!({"path": "core/data_fetcher.py", "old_text": "x", "new_text": "y"}),
        );
        let msg = ChatMessage {
            role: "assistant".to_string(),
            content: vec![Part::Text {
                text: String::new(),
            }],
            tool_calls: Some(vec![tool]),
            tool_call_id: None,
            is_summary: false,
        };

        let compressed = compress_assistant_message(&msg, 1);
        assert!(compressed.contains("core/data_fetcher.py"));
    }

    // ========================================================================
    // Integration tests — real-tokenizer trigger accuracy.
    //
    // These drive the full `process()` path with a `MiddlewareContext` and
    // assert that the trigger fires/doesn't-fire based on REAL token counts.
    // Before Phase 1, `estimate_total_tokens` returned chars/4, so these
    // fixtures would have hit the wrong thresholds — that's the bug this
    // phase eliminates.
    // ========================================================================

    fn build_code_heavy_conversation(n_turns: usize) -> Vec<ChatMessage> {
        // Python code chosen because `chars/4` notoriously undercounts it by
        // ~25% — each short identifier and punctuation token is one real
        // token but <4 characters. With real tokenizer, these tool results
        // exceed the threshold; with the old heuristic they would not.
        let code = "def fetch_user(user_id: int) -> Optional[User]:\n    \
                    if not user_id:\n        return None\n    \
                    return db.query(User).filter_by(id=user_id).first()\n\n\
                    class UserRepository:\n    \
                    def __init__(self, conn): self.conn = conn\n    \
                    def get(self, uid): return self.conn.execute(\
                    'SELECT * FROM users WHERE id=?', (uid,)).fetchone()\n";
        let mut messages = Vec::with_capacity(n_turns * 2);
        for i in 0..n_turns {
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text {
                    text: String::new(),
                }],
                tool_calls: Some(vec![ToolCall::new(
                    format!("call_{i}"),
                    "read_file".to_string(),
                    json!({ "path": format!("src/module_{i}.py") }),
                )]),
                tool_call_id: None,
                is_summary: false,
            });
            messages.push(ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: code.to_string(),
                }],
                tool_calls: None,
                tool_call_id: Some(format!("call_{i}")),
                is_summary: false,
            });
        }
        messages
    }

    fn make_ctx(model: &str) -> MiddlewareContext {
        MiddlewareContext::new(
            "agent-test".to_string(),
            None,
            "openai".to_string(),
            model.to_string(),
        )
    }

    /// Conversation exceeds the trigger in REAL tokens but would fall
    /// short under the old chars/4 heuristic — proves Phase 1 flips the
    /// outcome from "miss" to "fire".
    #[tokio::test]
    async fn real_tokenizer_fires_trigger_that_chars_heuristic_would_miss() {
        let messages = build_code_heavy_conversation(20);
        let real_tokens = crate::middleware::token_counter::estimate_total_tokens(
            &messages,
            "claude-3-5-sonnet-20241022",
        );
        let chars_over_four: usize = messages
            .iter()
            .map(|m| m.text_content().len() / 4)
            .sum::<usize>()
            + messages.len() * 4;

        // The trigger sits ABOVE the old chars/4 estimate but BELOW the
        // real token count. Under chars/4 this would be a silent miss.
        assert!(
            chars_over_four < real_tokens,
            "fixture must exhibit the code undercount: chars/4={chars_over_four}, real={real_tokens}"
        );
        let trigger = chars_over_four + (real_tokens - chars_over_four) / 2;

        let config = ContextEditingConfig {
            enabled: true,
            trigger_tokens: trigger,
            keep_tool_results: 3,
            min_reclaim: 0,
            ..Default::default()
        };
        let middleware = ContextEditingMiddleware::new(config);
        let ctx = make_ctx("claude-3-5-sonnet-20241022");

        let effect = middleware
            .process(messages, &ctx)
            .await
            .expect("process ok");

        match effect {
            MiddlewareEffect::EmitAndModify { .. } | MiddlewareEffect::ModifiedMessages(_) => {}
            MiddlewareEffect::Proceed | MiddlewareEffect::EmitEvent(_) => {
                panic!(
                    "expected trigger to fire at real-tokenizer threshold \
                     (trigger={trigger}, real={real_tokens}); got Proceed/EmitEvent — \
                     chars/4 regression"
                );
            }
        }
    }

    /// Conversation sits safely below the trigger — no over-firing.
    #[tokio::test]
    async fn real_tokenizer_does_not_over_trigger_below_threshold() {
        let messages = build_code_heavy_conversation(2);
        let real_tokens = crate::middleware::token_counter::estimate_total_tokens(
            &messages,
            "claude-3-5-sonnet-20241022",
        );

        let config = ContextEditingConfig {
            enabled: true,
            // Trigger well above the conversation's real token count.
            trigger_tokens: real_tokens * 10,
            keep_tool_results: 3,
            min_reclaim: 0,
            ..Default::default()
        };
        let middleware = ContextEditingMiddleware::new(config);
        let ctx = make_ctx("claude-3-5-sonnet-20241022");

        let effect = middleware
            .process(messages, &ctx)
            .await
            .expect("process ok");

        assert!(
            matches!(effect, MiddlewareEffect::Proceed),
            "middleware must not trigger when real tokens are far below threshold"
        );
    }
}
