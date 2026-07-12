# gateway-templates

System prompt assembly for AgentZero agents. Assembles the canonical agent
contracts and prompt modules from `config/` (user customizable), falling back
to embedded defaults.

## Build & Test

```bash
cargo test -p gateway-templates    # 10 tests
```

## Public API

```rust
pub fn load_system_prompt_from_paths(paths: &Arc<VaultPaths>) -> String;
pub fn load_system_prompt(data_dir: &Path) -> String;          // legacy path-based
pub fn load_chat_prompt_from_paths(paths: &Arc<VaultPaths>) -> String;
pub fn default_system_prompt() -> String;                      // fallback
```

`Templates` — `rust-embed` struct giving access to all embedded template files.

## Assembly Order (full prompt)

1. `config/agent/SOUL.md` — identity/personality (created from `soul_starter.md` if missing)
2. `config/agent/INSTRUCTIONS.md` — execution rules (created from `instructions_starter.md` if missing)
3. `config/agent/OS.md` — platform commands (auto-generated for current OS if missing)
4. Required prompt modules (`config/agent-prompts/` override embedded defaults): `first-turn-protocol`, `tooling-skills`, `memory-learning`, `planning-autonomy`
5. Extra user prompt modules (any additional `.md` in `config/agent-prompts/`)
6. Runtime environment info (vault path, venv status)

**Fast chat prompt** uses: SOUL.md + `chat-instructions.md` + OS.md + `chat-protocol` + `tooling-skills` prompt modules only.

## Embedded Templates

```
templates/
├── soul_starter.md              # Default SOUL.md
├── instructions_starter.md      # Default INSTRUCTIONS.md
├── chat_instructions.md         # Default chat-mode instructions
├── system_prompt.md             # Emergency fallback
├── os_linux.md / os_macos.md / os_windows.md
├── distillation_prompt.md       # Internal embedded session distillation prompt
└── shards/
    ├── first_turn_protocol.md
    ├── tooling_skills.md
    ├── memory_learning.md
    ├── planning_autonomy.md
    ├── chat_protocol.md
    ├── safety.md
    └── session_ctx.md
```

## Notes

- User files in `config/agent/` and `config/agent-prompts/` take priority over embedded defaults.
- Embedded template asset names intentionally remain Rust-style because they are compile-time resources; user-facing vault paths are lowercase-kebab names.
- Extra user `.md` files in `config/agent-prompts/` are appended after required modules.
