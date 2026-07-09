# gateway-templates

System prompt assembly for AgentZero agents. Assembles SOUL.md + INSTRUCTIONS.md + OS.md + shards from `config/` (user customizable), falling back to embedded defaults.

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

1. `config/SOUL.md` — identity/personality (created from `soul_starter.md` if missing)
2. `config/INSTRUCTIONS.md` — execution rules (created from `instructions_starter.md` if missing)
3. `config/OS.md` — platform commands (auto-generated for current OS if missing)
4. Required shards (`config/shards/` override embedded defaults): `first_turn_protocol`, `tooling_skills`, `memory_learning`, `planning_autonomy`
5. Extra user shards (any additional `.md` in `config/shards/`)
6. Runtime environment info (vault path, venv status)

**Fast chat prompt** uses: SOUL.md + `chat_instructions.md` + OS.md + `chat_protocol` + `tooling_skills` shards only.

## Embedded Templates

```
templates/
├── soul_starter.md              # Default SOUL.md
├── instructions_starter.md      # Default INSTRUCTIONS.md
├── chat_instructions.md         # Default chat-mode instructions
├── system_prompt.md             # Emergency fallback
├── os_linux.md / os_macos.md / os_windows.md
├── distillation_prompt.md       # Session distillation prompt
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

- User files in `config/` take priority over embedded defaults.
- Embedded shards are written to `config/shards/` on first run so users can edit them.
- Extra user `.md` files in `config/shards/` are appended after required shards.
