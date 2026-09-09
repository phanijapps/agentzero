// ============================================================================
// END-OF-DAY SUMMARY GENERATION
// Generate summaries of daily sessions for context continuity
// ============================================================================

use crate::types::SessionMessage;
use crate::Result;
use tracing::info;

/// Generate an end-of-day summary for a session
///
/// This would use an LLM to generate a concise summary of the day's
/// conversation, capturing key points, decisions, and context for
/// continuing the next day.
pub async fn generate_summary(messages: &[SessionMessage]) -> Result<String> {
    info!("Generating summary for {} messages", messages.len());

    // TODO: Integrate with LLM to generate actual summary
    // For now, return a placeholder

    if messages.is_empty() {
        Ok("No messages in this session.".to_string())
    } else {
        Ok(format!(
            "Session summary placeholder: {} messages processed.",
            messages.len()
        ))
    }
}

/// Summary generation options
pub struct SummaryOptions {
    /// Maximum length of the summary in characters
    pub max_length: usize,

    /// Whether to include tool calls in the summary
    pub include_tool_calls: bool,

    /// Whether to include user messages only
    pub user_messages_only: bool }

impl Default for SummaryOptions {
    fn default() -> Self {
        Self {
            max_length: 500,
            include_tool_calls: false,
            user_messages_only: false }
    }
}

/// Generate a summary with custom options
pub async fn generate_summary_with_options(
    messages: &[SessionMessage],
    options: &SummaryOptions,
) -> Result<String> {
    info!(
        "Generating summary with options: max_length={}, include_tool_calls={}, user_messages_only={}",
        options.max_length,
        options.include_tool_calls,
        options.user_messages_only
    );

    // Filter messages based on options
    let filtered_messages: Vec<_> = messages
        .iter()
        .filter(|m| {
            if options.user_messages_only {
                m.role == "user"
            } else {
                true
            }
        })
        .collect();

    // TODO: Implement actual summary generation with LLM
    Ok(format!(
        "Summary with options: {} messages considered.",
        filtered_messages.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SessionMessage;

    #[tokio::test]
    async fn test_generate_summary_empty() {
        let messages: Vec<SessionMessage> = Vec::new();
        let summary = generate_summary(&messages).await.unwrap();

        assert_eq!(summary, "No messages in this session.");
    }

    #[tokio::test]
    async fn test_generate_summary_with_messages() {
        let session_id = "test_session".to_string();
        let messages = vec![
            SessionMessage::user_message(session_id.clone(), "Hello".to_string()),
            SessionMessage::assistant_message(session_id.clone(), "Hi there!".to_string()),
        ];

        let summary = generate_summary(&messages).await.unwrap();

        assert!(summary.contains("2 messages"));
    }
}
