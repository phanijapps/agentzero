//! Minimal ANSI styling. Direct escape codes — no terminal-rendering crate.
//!
//! Palette (Direction B — Cool Terminal):
//! - accent / violet-400  : 167, 139, 250  → prompt, welcome border, user
//! - secondary / sky-400  :  56, 189, 248  → assistant
//! - amber                : 251, 191,  36  → system / slash output
//! - slate-500            : 100, 116, 139  → muted dim
//! - emerald-500          :  34, 197,  94  → success
//! - red-400              : 248, 113, 113  → error
//!
//! All helpers no-op when `color` is false (returns the plain text).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Amber,
    Dim,
    BoldAccent,
    BoldSecondary,
    Error,
}

const RESET: &str = "\x1b[0m";

const FG_AMBER: &str = "\x1b[38;2;251;191;36m"; // amber-400
const FG_MUTED: &str = "\x1b[38;2;100;116;139m"; // slate-500
const FG_ERROR: &str = "\x1b[38;2;248;113;113m"; // red-400

fn open(style: Style) -> &'static str {
    match style {
        Style::Amber => FG_AMBER,
        Style::Dim => FG_MUTED,
        Style::BoldAccent => "\x1b[1;38;2;167;139;250m",
        Style::BoldSecondary => "\x1b[1;38;2;56;189;248m",
        Style::Error => FG_ERROR,
    }
}

pub fn paint(text: &str, color: bool, style: Style) -> String {
    if !color {
        return text.to_string();
    }
    format!("{}{}{}", open(style), text, RESET)
}

pub fn tool_marker(text: &str, color: bool, style: Style) -> String {
    paint(text, color, style)
}

pub fn label(text: &str, color: bool, style: Style) -> String {
    paint(text, color, style)
}

/// Print a one-shot welcome banner. Called once at REPL startup.
pub fn welcome_banner(daemon_url: &str, session_id: &str, color: bool) -> String {
    let title = format!("ZBOT v{}", env!("CARGO_PKG_VERSION"));
    let mut out = String::new();

    out.push_str(&paint(&title, color, Style::BoldAccent));
    out.push('\n');
    out.push_str(&paint("daemon  ", color, Style::Dim));
    out.push_str(daemon_url);
    out.push('\n');
    out.push_str(&paint("session ", color, Style::Dim));
    out.push_str(session_id);
    out.push('\n');
    out.push_str(&paint(
        "↵ to send  ·  /help for commands  ·  ⌃C to quit",
        color,
        Style::Dim,
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_strips_escapes() {
        let s = paint("hello", false, Style::Amber);
        assert_eq!(s, "hello");
        assert!(!s.contains("\x1b["));
    }

    #[test]
    fn color_adds_escapes() {
        let s = paint("hello", true, Style::Amber);
        assert!(s.contains("\x1b["));
        assert!(s.ends_with(RESET));
    }
}
