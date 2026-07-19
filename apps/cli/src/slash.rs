//! Slash command parsing.
//!
//! Anything starting with `/` is a slash command, not a chat message.
//! Parsing is intentionally permissive — unknown commands are returned
//! as `Unknown` so the UI can show a hint instead of silently dropping.

/// A parsed slash command. The trailing argument (when present) is
/// already trimmed of surrounding whitespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    Help,
    New,
    Sessions,
    Wards,
    Memory(String),
    Quit,
    Unknown(String),
}

/// Parse a raw line into a `SlashCommand`.
///
/// Returns `None` when the input is not a slash command (does not
/// start with `/`).
pub fn parse(input: &str) -> Option<SlashCommand> {
    let rest = input.trim().strip_prefix('/')?;
    let mut parts = rest.splitn(2, char::is_whitespace);
    let head = parts.next().unwrap_or("");
    let arg = parts.next().map(str::trim).unwrap_or("");
    Some(match head {
        "help" | "?" | "h" => SlashCommand::Help,
        "new" => SlashCommand::New,
        "sessions" | "ls" => SlashCommand::Sessions,
        "wards" => SlashCommand::Wards,
        "memory" | "recall" | "m" => SlashCommand::Memory(arg.to_string()),
        "quit" | "q" | "exit" => SlashCommand::Quit,
        other => SlashCommand::Unknown(other.to_string()),
    })
}

pub const HELP_TEXT: &str = "\
slash commands
  /help         show this help
  /new          clear the chat session and start fresh
  /sessions     list recent conversations
  /wards        list wards
  /memory <q>   quick recall — no chat turn cost
  /quit         exit (also Ctrl+C / Ctrl+D)";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_commands() {
        assert_eq!(parse("/help"), Some(SlashCommand::Help));
        assert_eq!(parse("/?"), Some(SlashCommand::Help));
        assert_eq!(parse("/new"), Some(SlashCommand::New));
        assert_eq!(parse("/sessions"), Some(SlashCommand::Sessions));
        assert_eq!(parse("/wards"), Some(SlashCommand::Wards));
        assert_eq!(parse("/quit"), Some(SlashCommand::Quit));
        assert_eq!(parse("/q"), Some(SlashCommand::Quit));
    }

    #[test]
    fn memory_carries_query() {
        assert_eq!(
            parse("/memory rust async"),
            Some(SlashCommand::Memory("rust async".into()))
        );
        assert_eq!(
            parse("/m rust async"),
            Some(SlashCommand::Memory("rust async".into()))
        );
    }

    #[test]
    fn memory_empty_query_keeps_empty_string() {
        assert_eq!(parse("/memory"), Some(SlashCommand::Memory(String::new())));
    }

    #[test]
    fn unknown_keeps_head() {
        assert_eq!(parse("/foo bar"), Some(SlashCommand::Unknown("foo".into())));
    }

    #[test]
    fn plain_message_returns_none() {
        assert_eq!(parse("hello"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn leading_whitespace_tolerated() {
        assert_eq!(parse("   /help"), Some(SlashCommand::Help));
    }
}
