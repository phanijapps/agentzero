//! Single-turn streaming. Shared between interactive REPL and one-shot mode.
//!
//! Sends `Invoke` over the existing WS connection, then writes every Token
//! frame to stdout as it arrives. Tool calls go to stderr (so stdout stays
//! clean when piped). Returns when the turn completes or errors.
//!
//! Append-only: every byte we write to the terminal stays there. No
//! re-rendering, no cursor manipulation, no border math.

use anyhow::{anyhow, Result};
use gateway_ws_protocol::ServerMessage;
use std::io::{self, Write};

use crate::events::EventStream;
use crate::style::{self, Style};

/// Token accounting for one turn.
#[derive(Debug, Default, Clone, Copy)]
pub struct TurnSummary {
    pub tokens_in: u64,
    pub tokens_out: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct StreamConfig {
    /// Enable ANSI colors. Off when stdout is not a TTY or `$NO_COLOR` is set.
    pub color: bool,
    /// Indent each line of streamed output by this many spaces. The interactive
    /// REPL uses `2`; one-shot uses `0` (clean pipe output).
    pub indent: usize,
}

/// Run one turn. Streams tokens to stdout, tool markers to stderr.
/// Caller must have already `Subscribe`d on `events` for `conversation_id`.
pub async fn run_turn(
    events: &mut EventStream,
    conversation_id: &str,
    session_id: &str,
    message: &str,
    cfg: StreamConfig,
) -> Result<TurnSummary> {
    events.invoke(
        "root",
        conversation_id,
        Some(session_id.to_string()),
        message.to_string(),
    )?;

    let mut summary = TurnSummary::default();
    let mut had_token = false;
    let mut at_line_start = true;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let indent = " ".repeat(cfg.indent);

    let mut error_text: Option<String> = None;

    loop {
        let Some(msg) = events.recv().await else {
            if error_text.is_none() {
                error_text = Some("ws closed before turn completed".into());
            }
            break;
        };
        match msg {
            ServerMessage::Token { delta, .. } => {
                had_token = true;
                // Indent each new line; emit the rest as-is.
                for (i, segment) in delta.split('\n').enumerate() {
                    if i > 0 {
                        out.write_all(b"\n")?;
                        at_line_start = true;
                    }
                    if segment.is_empty() {
                        continue;
                    }
                    if at_line_start && cfg.indent > 0 {
                        out.write_all(indent.as_bytes())?;
                        at_line_start = false;
                    }
                    out.write_all(segment.as_bytes())?;
                }
                out.flush()?;
            }
            ServerMessage::ToolCall { tool, .. } => {
                let line = format!(
                    "\n{}{}{}\n",
                    indent,
                    style::tool_marker(&format!("▸ {tool}"), cfg.color, Style::Dim),
                    "",
                );
                let _ = io::stderr().write_all(line.as_bytes());
                at_line_start = true;
            }
            ServerMessage::ToolResult {
                error: Some(err), ..
            } => {
                let line = format!(
                    "{}{}{}\n",
                    indent,
                    style::tool_marker(&format!("✗ {err}"), cfg.color, Style::Error),
                    "",
                );
                let _ = io::stderr().write_all(line.as_bytes());
            }
            ServerMessage::ToolResult { error: None, .. } => {}
            ServerMessage::TokenUsage {
                tokens_in,
                tokens_out,
                ..
            } => {
                summary.tokens_in = tokens_in;
                summary.tokens_out = tokens_out;
            }
            ServerMessage::TurnComplete { .. } | ServerMessage::AgentCompleted { .. } => {
                break;
            }
            ServerMessage::Error { message, .. } => {
                error_text = Some(message);
                break;
            }
            _ => { /* ignore Thinking, Heartbeat, Iteration, etc. */ }
        }
    }

    if had_token {
        out.write_all(b"\n")?;
        out.flush()?;
    }

    if let Some(err) = error_text {
        return Err(anyhow!("{err}"));
    }
    Ok(summary)
}
