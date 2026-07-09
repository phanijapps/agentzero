# Ward Scaffolding — Component Overview

Wards are scaffolded minimally. `AGENTS.md` is seeded with the ward name as a
heading. `memory-bank/ward.md`, `memory-bank/structure.md`,
`memory-bank/core_docs.md`, and `core/` are created empty. The agent curates
all content during sessions — no code ever rewrites these files.

## Related Files

| File | Purpose |
|------|---------|
| `runtime/agent-tools/src/tools/ward.rs` | `WardTool` — creates ward dir, minimal AGENTS.md seed, empty memory-bank + core scaffolds |
| `gateway/gateway-execution/src/middleware/ward_scaffold.rs` | `scaffold_ward()` — creates skill-declared directories and writes the same minimal AGENTS.md seed |
| `gateway/gateway-services/src/skills.rs` | `WardSetup` type — drives directory list from skill frontmatter |
