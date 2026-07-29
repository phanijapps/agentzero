# Plan: Ward Agent Doctrine Template

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Use the existing template directory and secure Ward-layout loading patterns.
The bundled Markdown is seeded once into the vault, read only when creating a
new ward, and rendered through two literal substitutions. Doctrine loading at
Ward-agent synthesis becomes bounded and fail-soft without touching any routing
or orchestration path.

## Design (LLD)

- `VaultPaths::ward_agent_template()` owns the canonical vault path.
- `ward_layout::loader` owns `DEFAULT_WARD_AGENT_TEMPLATE`, the 12 KiB ceiling,
  a shared vault-anchored bounded UTF-8 file reader, and create-new seeding.
- `ward_layout::create` first detects whether the compiled root declares a
  required literal `AGENTS.md`; only then does it load and render the persona
  template, always before Ward publication.
- `invoke::setup` reuses the services-layer bounded reader and maps failures to
  fixed redacted diagnostics. Prompt composition itself
  keeps the established identity → system context → delimited doctrine order.
- No new dependency, persistence type, public Ward API, or generalized
  templating abstraction is introduced.

## Tasks

### T1: Seed and validate the editable template

**Depends on:** none

**Touches:** `gateway/templates/ward-agent.md`, `gateway/gateway-services/src/paths.rs`, `gateway/gateway-services/src/ward_layout/loader.rs`, `gateway/gateway-services/src/ward_layout/mod.rs`, `gateway/gateway-services/src/lib.rs`, `gateway/src/state/mod.rs`

**Tests:**

- `ward_agent_template_path_is_canonical` (AC1), stub: true
- `seed_default_ward_agent_template_creates_then_preserves` (AC1, AC4), stub: true
- `load_ward_agent_template_rejects_unsafe_or_oversized_files` (AC3), stub: true
- `bounded_reader_rejects_hard_links_and_special_files` (AC3), stub: true

```rust
// STUB: AC1
#[test]
fn seed_default_ward_agent_template_creates_then_preserves() {
    let vault = tempfile::tempdir().unwrap();
    let paths = VaultPaths::new(vault.path().to_path_buf());
    assert_eq!(seed_default_ward_agent_template(&paths).unwrap(), SeedOutcome::Created);
    std::fs::write(paths.ward_agent_template(), "# User template\n").unwrap();
    assert_eq!(seed_default_ward_agent_template(&paths).unwrap(), SeedOutcome::Preserved);
    assert_eq!(std::fs::read_to_string(paths.ward_agent_template()).unwrap(), "# User template\n");
}
```

**Approach:** Add the embedded Markdown and extract the layout loader's bounded,
single-link checks into one vault-anchored reader. On Linux traverse components
descriptor-relatively with no-follow flags; on other targets use canonical
pre/post confinement plus opened-file identity checks and validate a newly
created seed target before writing bytes. Reuse create-new seed semantics.

**Done when:** focused `gateway-services` loader/path tests pass.

### T2: Render doctrine only for new template-backed wards

**Depends on:** T1

**Touches:** `gateway/gateway-services/src/ward_layout/create.rs`

**Tests:**

- `scaffolds_agent_instructions_from_user_template` (AC2), stub: true
- `template_limit_failure_does_not_publish_partial_ward` (AC3), stub: true
- `layout_without_agents_does_not_load_invalid_persona_template` (AC3), stub: true
- `existing_ward_is_not_modified` (AC4), stub: true

```rust
// STUB: AC2
#[test]
fn scaffolds_agent_instructions_from_user_template() {
    let (paths, _vault) = seeded_layout_fixture();
    std::fs::write(paths.ward_agent_template(), "# {{display_name}}\nID={{ward_id}}\n").unwrap();
    let created = create_ward_from_template(&paths, "market-research").unwrap();
    assert_eq!(std::fs::read_to_string(created.path.join("AGENTS.md")).unwrap(), "# Market Research\nID=market-research\n");
}
```

**Approach:** Detect a required literal root `AGENTS.md`, then load and render
once before publication. Pass the rendered bytes only to that scaffold branch;
keep every other Markdown scaffold unchanged.

**Done when:** focused creation tests prove atomic failure and preservation.

### T3: Bound Ward-agent doctrine loading without changing routing

**Depends on:** T2

**Touches:** `gateway/gateway-execution/src/invoke/setup.rs`

**Tests:**

- `load_ward_doctrine_preserves_complete_valid_content` (AC5), stub: true
- `load_ward_doctrine_rejects_oversized_without_partial_content` (AC6), stub: true
- `load_ward_doctrine_rejects_symlink_hardlink_special_and_non_utf8` (AC6), stub: true
- `valid_doctrine_reaches_prompt_with_only_outer_trim` (AC5), stub: true
- `load_ward_doctrine_treats_missing_as_empty_without_diagnostic` (AC7), stub: true

```rust
// STUB: AC6
#[test]
fn load_ward_doctrine_rejects_oversized_without_partial_content() {
    let paths = temporary_vault_paths();
    let ward = paths.ward_dir("oversized");
    std::fs::create_dir_all(&ward).unwrap();
    let payload = "secret-marker".repeat(2048);
    std::fs::write(ward.join("AGENTS.md"), &payload).unwrap();
    let loaded = load_ward_doctrine(&paths, "oversized");
    assert!(loaded.doctrine.is_empty());
    let diagnostic = loaded.diagnostic.unwrap();
    assert!(!diagnostic.contains("secret-marker"));
    assert!(diagnostic.contains("WARD_DOCTRINE_UNAVAILABLE:too_large"));
}
```

**Approach:** Reuse the services-layer bounded reader and append only a fixed
`WARD_DOCTRINE_UNAVAILABLE:<reason>` diagnostic (maximum 160 ASCII bytes) to
the platform-owned identity. Log only the code and Ward name. Keep valid
doctrine composition and missing-file behavior unchanged.

**Done when:** focused `gateway-execution` setup tests pass.

### T4: Prove isolation and run release gates

**Depends on:** T3

**Touches:** `docs/specs/ward-agent-doctrine-template/**`, `docs/specs/README.md`

**Tests:** no stub (goal-based).

**Approach:** Compare protected-path hashes captured before implementation,
inspect the final diff, run focused and workspace Rust gates, and run post-build
adversarial and security reviews.

**Done when:** AC8 is proven, reviews approve, and all applicable gates pass.

## Verification

- `cargo test -p gateway-services --quiet`: 230 unit + 9 integration tests passed.
- `cargo test -p gateway-execution --lib --quiet`: 538 tests passed.
- `cargo test -p gateway-execution --test ward_agent_spawn_tests --quiet`: 5 tests passed.
- `cargo check --workspace --quiet`: passed.
- `cargo clippy -p gateway-services -p gateway-execution -p gateway --all-targets -- -D warnings`: passed.
- `cargo fmt --all --check` and `git diff --check`: passed.
- Protected path hashes: byte-identical to the pre-implementation baseline.
- Windows target check: attempted, but unrelated native `onig_sys`, `ring`, and
  `libsqlite3-sys` build scripts require an MSVC cross-toolchain unavailable on
  this Linux host; Windows identity logic uses stable `MetadataExt` APIs.
- Post-build adversarial review: approved.
- Post-build security review: approved.
