//! # Event Logging
//!
//! Execution log helpers for stream event processing.
//!
//! Each public function builds an [`ExecutionLog`] entry and routes it through
//! the batch writer if one is present, otherwise writes directly via the log service.

use api_logs::{ExecutionLog, LogCategory, LogLevel};

use super::stream_context::StreamContext;

// ============================================================================
// PRIVATE ROUTING HELPER
// ============================================================================

/// Route a log entry through the batch writer if available, else write directly.
fn log_entry(ctx: &StreamContext, entry: ExecutionLog) {
    if let Some(writer) = &ctx.batch_writer {
        writer.log(entry);
    } else {
        let _ = ctx.log_service.log(entry);
    }
}

// ============================================================================
// PUBLIC LOG FUNCTIONS
// ============================================================================

/// Log a delegation event.
pub fn log_delegation(ctx: &StreamContext, child_agent: &str, task: &str) {
    let entry = ExecutionLog::new(
        &ctx.execution_id,
        &ctx.session_id,
        &ctx.agent_id,
        LogLevel::Info,
        LogCategory::Delegation,
        format!("Delegating to {}", child_agent),
    )
    .with_metadata(serde_json::json!({
        "child_agent": child_agent,
        "task": task,
    }));
    log_entry(ctx, entry);
}

/// Log a tool call start event.
pub fn log_tool_call(
    ctx: &StreamContext,
    tool_id: &str,
    tool_name: &str,
    args: &serde_json::Value,
) {
    let entry = ExecutionLog::new(
        &ctx.execution_id,
        &ctx.session_id,
        &ctx.agent_id,
        LogLevel::Info,
        LogCategory::ToolCall,
        format!("Calling tool: {}", tool_name),
    )
    .with_metadata(serde_json::json!({
        "tool_id": tool_id,
        "tool_name": tool_name,
        "args": args,
    }));
    log_entry(ctx, entry);
}

/// Log a tool result event.
pub fn log_tool_result(
    ctx: &StreamContext,
    tool_id: &str,
    result: &str,
    error: &Option<String>,
    duration_ms: Option<i64>,
) {
    let blocked_by_hook = is_blocked_by_hook_result(result, error);
    let level = tool_result_log_level(error, blocked_by_hook);

    // Truncate result for logging
    let truncated = if result.len() > 500 {
        format!("{}...", agent_primitives::truncate_str(result, 500))
    } else {
        result.to_string()
    };

    let entry = ExecutionLog::new(
        &ctx.execution_id,
        &ctx.session_id,
        &ctx.agent_id,
        level,
        LogCategory::ToolResult,
        tool_result_message(error, blocked_by_hook),
    )
    .with_metadata(tool_result_metadata(
        tool_id,
        &truncated,
        error,
        blocked_by_hook,
    ));
    let entry = match duration_ms {
        Some(duration_ms) => entry.with_duration(duration_ms),
        None => entry,
    };
    log_entry(ctx, entry);
}

fn is_blocked_by_hook_result(result: &str, error: &Option<String>) -> bool {
    error.as_deref() == Some("blocked_by_hook") || result == "[blocked by hook]"
}

fn tool_result_log_level(error: &Option<String>, blocked_by_hook: bool) -> LogLevel {
    if error.is_some() || blocked_by_hook {
        LogLevel::Warn
    } else {
        LogLevel::Info
    }
}

fn tool_result_message(error: &Option<String>, blocked_by_hook: bool) -> &'static str {
    if blocked_by_hook {
        "Tool blocked by hook"
    } else if error.is_some() {
        "Tool returned error"
    } else {
        "Tool completed"
    }
}

fn tool_result_metadata(
    tool_id: &str,
    result: &str,
    error: &Option<String>,
    blocked_by_hook: bool,
) -> serde_json::Value {
    serde_json::json!({
        "tool_id": tool_id,
        "result": result,
        "error": error,
        "blocked_by_hook": blocked_by_hook,
    })
}

/// Log an error event.
pub fn log_error(ctx: &StreamContext, error: &str) {
    let entry = ExecutionLog::new(
        &ctx.execution_id,
        &ctx.session_id,
        &ctx.agent_id,
        LogLevel::Error,
        LogCategory::Error,
        error,
    );
    log_entry(ctx, entry);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_hook_result_is_warn_with_structured_metadata() {
        let error = Some("blocked_by_hook".to_string());

        assert!(is_blocked_by_hook_result("[blocked by hook]", &error));
        assert_eq!(tool_result_log_level(&error, true), LogLevel::Warn);
        assert_eq!(tool_result_message(&error, true), "Tool blocked by hook");

        let metadata = tool_result_metadata("call-1", "[blocked by hook]", &error, true);
        assert_eq!(metadata["blocked_by_hook"], true);
        assert_eq!(metadata["error"], "blocked_by_hook");
    }

    #[test]
    fn successful_tool_result_stays_info() {
        let error = None;

        assert!(!is_blocked_by_hook_result("ok", &error));
        assert_eq!(tool_result_log_level(&error, false), LogLevel::Info);
        assert_eq!(tool_result_message(&error, false), "Tool completed");
    }
}
