//! Interactive REPL — readline + append-only stdout.
//!
//! The loop is dead simple:
//! 1. Read a line via `rustyline` (gives us history, arrow keys, etc.)
//! 2. If it starts with `/`, dispatch as a slash command
//! 3. Otherwise, send via WS and stream the assistant's response straight
//!    to stdout
//! 4. Repeat
//!
//! No iocraft, no re-renders, no border math. Every byte printed stays
//! printed. Resize behaves like any other CLI program.

use anyhow::{Context, Result};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use serde_json::Value;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::client::{ChatInit, DaemonClient};
use crate::events::EventStream;
use crate::slash::{self, SlashCommand};
use crate::stream::{run_turn, StreamConfig};
use crate::style::{self, Style};

const PROMPT_GLYPH: &str = "▸ ";

pub async fn run(
    chat: ChatInit,
    daemon_url: String,
    mut events: EventStream,
    client: DaemonClient,
    color: bool,
) -> Result<()> {
    events.subscribe(&chat.conversation_id)?;

    // Welcome banner (printed once, then untouched).
    println!(
        "{}",
        style::welcome_banner(&daemon_url, &chat.session_id, color)
    );
    println!();

    let mut session_id = chat.session_id.clone();
    let conv = chat.conversation_id.clone();

    let mut rl = DefaultEditor::new().context("init rustyline editor")?;
    let history_path = history_path();
    if let Some(p) = &history_path {
        let _ = rl.load_history(p);
    }

    let prompt = if color {
        format!("\x1b[1;38;2;167;139;250m{}\x1b[0m", PROMPT_GLYPH)
    } else {
        PROMPT_GLYPH.to_string()
    };

    let stream_cfg = StreamConfig { color, indent: 2 };

    loop {
        let line = match rl.readline(&prompt) {
            Ok(l) => l,
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => {
                println!();
                break;
            }
            Err(e) => {
                eprintln!("input error: {e}");
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let _ = rl.add_history_entry(trimmed);

        // Slash commands
        if let Some(cmd) = slash::parse(trimmed) {
            if matches!(cmd, SlashCommand::Quit) {
                break;
            }
            handle_slash(cmd, &client, &mut session_id, color).await;
            println!();
            continue;
        }

        // Chat turn — print the assistant header, then stream into it.
        println!();
        println!("{}", style::label("assistant", color, Style::BoldSecondary));
        if let Err(e) = run_turn(&mut events, &conv, &session_id, trimmed, stream_cfg).await {
            eprintln!("  {}", style::paint(&format!("⚠ {e}"), color, Style::Error));
        }
        println!();
    }

    if let Some(p) = history_path {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = rl.save_history(&p);
    }

    Ok(())
}

fn history_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("zbot").join("history"))
}

// =========================================================================
// Slash dispatch (HTTP-backed commands print directly to stdout)
// =========================================================================

async fn handle_slash(
    cmd: SlashCommand,
    client: &DaemonClient,
    session_id: &mut String,
    color: bool,
) {
    match cmd {
        SlashCommand::Help => {
            println!();
            print_system(slash::HELP_TEXT, color);
        }
        SlashCommand::Quit => {} // handled in loop
        SlashCommand::New => {
            println!();
            match client.clear_chat_session().await {
                Ok(()) => match client.init_chat_session().await {
                    Ok(chat) => {
                        *session_id = chat.session_id.clone();
                        print_system(
                            &format!(
                                "session cleared. new session: {}",
                                short_id(&chat.session_id)
                            ),
                            color,
                        );
                    }
                    Err(e) => print_error(&format!("session cleared, but init failed: {e}"), color),
                },
                Err(e) => print_error(&format!("clear failed: {e}"), color),
            }
        }
        SlashCommand::Sessions => {
            println!();
            match client.list_conversations().await {
                Ok(v) => print_system(&format_conversations(&v), color),
                Err(e) => print_error(&format!("list failed: {e}"), color),
            }
        }
        SlashCommand::Wards => {
            println!();
            match client.list_wards().await {
                Ok(v) => print_system(&format_wards(&v), color),
                Err(e) => print_error(&format!("list failed: {e}"), color),
            }
        }
        SlashCommand::Memory(q) => {
            println!();
            if q.is_empty() {
                print_system("usage: /memory <query>", color);
            } else {
                match client.memory_search(&q, 8).await {
                    Ok(v) => print_system(&format_memory(&v), color),
                    Err(e) => print_error(&format!("recall failed: {e}"), color),
                }
            }
        }
        SlashCommand::Unknown(name) => {
            println!();
            print_error(&format!("unknown command: /{name} — try /help"), color);
        }
    }
}

fn print_system(text: &str, color: bool) {
    println!("{}", style::label("system", color, Style::Amber));
    for line in text.lines() {
        println!("  {}", line);
    }
    let _ = io::stdout().flush();
}

fn print_error(text: &str, color: bool) {
    println!("{}", style::label("error", color, Style::Error));
    for line in text.lines() {
        println!("  {}", line);
    }
    let _ = io::stdout().flush();
}

fn short_id(id: &str) -> String {
    id.rsplit('-')
        .next()
        .unwrap_or(id)
        .chars()
        .take(8)
        .collect()
}

fn format_conversations(v: &Value) -> String {
    let arr = v.as_array().map(Vec::as_slice).unwrap_or(&[]);
    if arr.is_empty() {
        return "no conversations".into();
    }
    let mut out = String::from("recent conversations:\n");
    for c in arr.iter().take(10) {
        let id = c.get("id").and_then(Value::as_str).unwrap_or("?");
        let title = c
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("(untitled)");
        let updated = c
            .get("updatedAt")
            .or_else(|| c.get("updated_at"))
            .and_then(Value::as_str)
            .unwrap_or("");
        out.push_str(&format!("  {}  {}  {}\n", short_id(id), title, updated));
    }
    out.trim_end().to_string()
}

fn format_wards(v: &Value) -> String {
    let arr = v.as_array().map(Vec::as_slice).unwrap_or(&[]);
    if arr.is_empty() {
        return "no wards".into();
    }
    let mut out = String::from("wards:\n");
    for w in arr {
        let name = w
            .get("name")
            .or_else(|| w.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("?");
        let desc = w.get("description").and_then(Value::as_str).unwrap_or("");
        out.push_str(&format!("  {}  {}\n", name, truncate(desc, 60)));
    }
    out.trim_end().to_string()
}

fn format_memory(v: &Value) -> String {
    let items = if v.is_array() {
        v.as_array().unwrap().as_slice()
    } else if let Some(arr) = v.get("items").and_then(Value::as_array) {
        arr.as_slice()
    } else if let Some(arr) = v.get("results").and_then(Value::as_array) {
        arr.as_slice()
    } else {
        &[]
    };
    if items.is_empty() {
        return "no matches".into();
    }
    let mut out = String::from("recall:\n");
    for item in items.iter().take(8) {
        let content = item
            .get("content")
            .or_else(|| item.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("?");
        let category = item.get("category").and_then(Value::as_str).unwrap_or("");
        let cat = if category.is_empty() {
            String::new()
        } else {
            format!("[{category}] ")
        };
        out.push_str(&format!("  {cat}{}\n", truncate(content, 80)));
    }
    out.trim_end().to_string()
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(n).collect();
        format!("{truncated}…")
    }
}
