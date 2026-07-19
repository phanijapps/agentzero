# Ward-as-Agent P1 — Execution Path — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `delegate_to_agent(agent_id="ward:<name>")` spawn the ward as a real subagent that runs a plan+execute loop in its own ward directory with the full tool inventory.

**Architecture:** A `ward:<name>` agent-id is intercepted in `AgentLoader::load_or_create_specialist` and routed to a new `synthesize_ward_agent` method — the ward IS the agent, no on-disk `agents/<id>/` folder. Its system prompt is composed from a generated identity line + the standard system-context shards + the ward's `AGENTS.md` doctrine (no `ZBOT.md` — the ward-as-skill rename is dropped). `spawn_delegated_agent` is taught that a `ward:` delegation operates in *that* ward's directory regardless of the parent's active ward. The rest of the delegation pipeline (`delegate_to_agent`, dispatcher, `ExecutorBuilder`, `wait_for_result`, `AgentResultBus`) is reused unchanged.

**Tech Stack:** Rust (workspace crates `gateway-execution`, `gateway-services`), `tokio::test`, `tempfile::TempDir`.

**Reference:** design doc `docs/architecture/future-state/2026-05-21-ward-as-agent-design.md` (§4 Gap A). The parked branch `feat/ward-as-skill` commit `48335eaa` is a Phase 3a sketch — adapted here, with `ZBOT.md` removed.

**Scope (P1 only):** the warm-path execution path for an **already-existing** ward directory. NOT in P1: create-on-miss (cold path), the full L1–L5 generic prompt + first-turn protocol (P2), `list_agents` exposure / graduation gate (P3), `out_of_scope`/`capability_missing` return states (P4).

---

## File Structure

| File | Change | Responsibility |
|---|---|---|
| `gateway/gateway-execution/src/invoke/setup.rs` | modify | Add `compose_ward_agent_instructions` (free fn) + `synthesize_ward_agent` (AgentLoader method); add `ward:` branch to `load_or_create_specialist`. |
| `gateway/gateway-execution/src/delegation/spawn.rs` | modify | Add `effective_ward_id` (free fn); use it so a `ward:` delegation runs in its own ward dir. |
| `gateway/gateway-execution/tests/ward_agent_spawn_tests.rs` | create | Integration test: `load_or_create_specialist("ward:<name>")` returns a correctly synthesized ward-agent. |

---

## Task 1: `compose_ward_agent_instructions` free function

**Files:**
- Modify: `gateway/gateway-execution/src/invoke/setup.rs` (add free fn near the other free fns, e.g. after `build_specialist_instructions`)
- Test: same file, in its `#[cfg(test)] mod tests`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `setup.rs` (if the module does not exist, create `#[cfg(test)] mod tests { use super::*; use std::sync::Arc; use tempfile::TempDir; use gateway_services::VaultPaths; ... }`):

```rust
#[test]
fn compose_ward_agent_instructions_places_identity_then_doctrine() {
    let tmp = TempDir::new().unwrap();
    let paths: SharedVaultPaths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    paths.ensure_dirs_exist().unwrap();
    let out = compose_ward_agent_instructions(
        "You are the maritime ward-agent.",
        &paths,
        "maritime",
        "## Purpose\nVessel tracking.",
    );
    assert!(out.starts_with("You are the maritime ward-agent."));
    assert!(out.contains("# --- WARD DOCTRINE: maritime ---"));
    assert!(out.contains("Vessel tracking."));
}

#[test]
fn compose_ward_agent_instructions_omits_empty_doctrine() {
    let tmp = TempDir::new().unwrap();
    let paths: SharedVaultPaths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    paths.ensure_dirs_exist().unwrap();
    let out = compose_ward_agent_instructions("identity line", &paths, "maritime", "   ");
    assert!(!out.contains("WARD DOCTRINE"));
}
```

- [ ] **Step 2: Run the tests, verify they fail to compile**

Run: `cargo test -p gateway-execution --lib compose_ward_agent_instructions 2>&1 | tail -15`
Expected: compile error — `cannot find function compose_ward_agent_instructions`.

- [ ] **Step 3: Implement the function**

Add to `setup.rs` (module scope, alongside the other free helper fns):

```rust
/// Compose a ward-agent's system prompt: identity line → system-context
/// shards → ward doctrine (`AGENTS.md`). The doctrine is framed under a
/// delimited header so the model can tell identity from conventions; an
/// empty doctrine (fresh ward) omits the section. Free function so the
/// composition is unit-testable without the agent/provider plumbing.
fn compose_ward_agent_instructions(
    identity: &str,
    paths: &SharedVaultPaths,
    ward_name: &str,
    doctrine: &str,
) -> String {
    let mut instructions =
        append_system_context(identity.trim(), paths, SubagentRole::Executor);
    if !doctrine.trim().is_empty() {
        instructions.push_str(&format!(
            "\n\n# --- WARD DOCTRINE: {ward_name} ---\n\n{}\n",
            doctrine.trim()
        ));
    }
    instructions
}
```

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo test -p gateway-execution --lib compose_ward_agent_instructions 2>&1 | tail -10`
Expected: `test result: ok. 2 passed`.

- [ ] **Step 5: Commit**

```bash
git add gateway/gateway-execution/src/invoke/setup.rs
git commit -m "feat(ward-as-agent): compose_ward_agent_instructions — identity + shards + doctrine"
```

---

## Task 2 & 3: `synthesize_ward_agent` + the `ward:` prefix branch

These ship together — `synthesize_ward_agent` is unreachable until `load_or_create_specialist` routes to it, so one integration test covers both.

**Files:**
- Modify: `gateway/gateway-execution/src/invoke/setup.rs` — add `synthesize_ward_agent` as a method in `impl AgentLoader<'a>`; add the `ward:` branch at the top of `load_or_create_specialist` (current body at lines 194–236).
- Create: `gateway/gateway-execution/tests/ward_agent_spawn_tests.rs`

- [ ] **Step 1: Write the failing integration test**

Create `gateway/gateway-execution/tests/ward_agent_spawn_tests.rs`. Mirror the service-construction pattern of an existing integration test (`tests/session_state_tests.rs`) for `AgentService` / `ProviderResolver`; the ward-specific assertions are:

```rust
// Construct a vault with one ward directory + AGENTS.md, build an
// AgentLoader against it, and assert load_or_create_specialist("ward:<name>")
// synthesizes the ward-agent.
#[tokio::test]
async fn load_or_create_specialist_synthesizes_ward_agent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let paths = std::sync::Arc::new(gateway_services::VaultPaths::new(
        tmp.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().unwrap();

    // Seed a ward directory with doctrine.
    let ward_dir = paths.ward_dir("maritime");
    std::fs::create_dir_all(&ward_dir).unwrap();
    std::fs::write(
        ward_dir.join("AGENTS.md"),
        "# maritime\n## Purpose\nVessel tracking and AIS analysis.\n",
    )
    .unwrap();

    let loader = build_test_agent_loader(&paths); // see Step 1b
    let (agent, _provider) = loader
        .load_or_create_specialist("ward:maritime")
        .await
        .expect("ward-agent synthesis should succeed");

    assert_eq!(agent.id, "ward:maritime");
    assert_eq!(agent.agent_type.as_deref(), Some("ward"));
    assert!(agent.instructions.contains("ward-agent"));
    assert!(agent.instructions.contains("Vessel tracking and AIS analysis."));
    assert!(agent.instructions.contains("# --- WARD DOCTRINE: maritime ---"));
}

#[tokio::test]
async fn load_or_create_specialist_errors_when_ward_dir_missing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let paths = std::sync::Arc::new(gateway_services::VaultPaths::new(
        tmp.path().to_path_buf(),
    ));
    paths.ensure_dirs_exist().unwrap();
    let loader = build_test_agent_loader(&paths);
    let err = loader
        .load_or_create_specialist("ward:nonexistent")
        .await
        .expect_err("missing ward dir must error in P1");
    assert!(err.contains("nonexistent"));
}
```

Step 1b — `build_test_agent_loader(&paths)` helper: construct `AgentService` and `ProviderResolver` exactly as `session_state_tests.rs` constructs them (open that file and copy the setup), then `AgentLoader::new(&agent_service, &provider_service, paths.clone())`. A configured default provider must exist in the test vault (write a minimal `config/providers.json` with one provider, mirroring the existing test fixtures) — `synthesize_ward_agent` resolves the default provider.

- [ ] **Step 2: Run the test, verify it fails**

Run: `cargo test -p gateway-execution --test ward_agent_spawn_tests 2>&1 | tail -15`
Expected: FAIL — `load_or_create_specialist("ward:maritime")` falls through to the auto-create path and returns an agent whose `id` is the literal string `"ward:maritime"` with `agent_type` `"specialist"`, so `assert_eq!(agent.agent_type, Some("ward"))` fails.

- [ ] **Step 3: Add `synthesize_ward_agent`**

Add as a method inside `impl<'a> AgentLoader<'a>` in `setup.rs`:

```rust
/// Synthesize a ward-agent for a `ward:<name>` delegation target.
///
/// The ward IS the agent — no on-disk `agents/<id>/` folder. The system
/// prompt is a generated identity line + the standard system-context
/// shards + the ward's `AGENTS.md` doctrine. There is no `ZBOT.md`.
///
/// P1 scope: the ward directory must already exist; creating a ward is
/// the cold path's job. A missing directory is an error here.
fn synthesize_ward_agent(
    &self,
    ward_name: &str,
) -> Result<(gateway_services::agents::Agent, Provider), String> {
    let ward_dir = self.paths.ward_dir(ward_name);
    if !ward_dir.is_dir() {
        return Err(format!(
            "ward '{}' has no directory at {}",
            ward_name,
            ward_dir.display()
        ));
    }

    // Doctrine: the ward's AGENTS.md. Empty is acceptable (fresh ward).
    let doctrine =
        std::fs::read_to_string(ward_dir.join("AGENTS.md")).unwrap_or_default();

    let identity = format!(
        "You are the {ward} ward-agent. You own the {ward} domain. Given a \
         task, you plan and execute it to completion in this single \
         delegation, then return a result.\n",
        ward = ward_name
    );
    let instructions =
        compose_ward_agent_instructions(&identity, &self.paths, ward_name, &doctrine);

    let provider = self.provider_resolver.get_default()?;
    let model = provider.default_model().to_string();

    let agent = gateway_services::agents::Agent {
        id: format!("ward:{ward_name}"),
        name: format!("ward:{ward_name}"),
        display_name: format!("Ward Agent: {ward_name}"),
        description: format!("Ward-agent for the {ward_name} ward"),
        agent_type: Some("ward".to_string()),
        provider_id: provider.id.clone().unwrap_or_default(),
        model,
        temperature: 0.7,
        max_tokens: 8192,
        thinking_enabled: false,
        voice_recording_enabled: false,
        system_instruction: None,
        instructions,
        mcps: vec![],
        skills: vec![],
        middleware: None,
        created_at: None,
    };

    tracing::info!(
        ward = ward_name,
        instructions_bytes = agent.instructions.len(),
        doctrine_bytes = doctrine.len(),
        "Synthesized ward-agent for delegation"
    );
    Ok((agent, provider))
}
```

- [ ] **Step 4: Add the `ward:` branch to `load_or_create_specialist`**

In `setup.rs`, insert at the very top of `load_or_create_specialist`'s body — before `match self.agent_service.get(agent_id).await {`:

```rust
    // Ward-as-agent: a `ward:<name>` id synthesizes the agent from the
    // ward directory instead of loading `agents/<id>/`. The ward IS the
    // agent — no on-disk agent folder.
    if let Some(ward_name) = agent_id.strip_prefix("ward:") {
        return self.synthesize_ward_agent(ward_name);
    }
```

- [ ] **Step 5: Run the tests, verify they pass**

Run: `cargo test -p gateway-execution --test ward_agent_spawn_tests 2>&1 | tail -12`
Expected: `test result: ok. 2 passed`.

- [ ] **Step 6: Build + lint the crate**

Run: `cargo clippy -p gateway-execution --all-targets -- -D warnings 2>&1 | tail -8`
Expected: `Finished` with no warnings.

- [ ] **Step 7: Commit**

```bash
git add gateway/gateway-execution/src/invoke/setup.rs gateway/gateway-execution/tests/ward_agent_spawn_tests.rs
git commit -m "feat(ward-as-agent): synthesize ward-agent on ward: delegation"
```

---

## Task 4: ward-agent runs in its own ward directory

Without this, a `ward:maritime` delegation inherits the *parent's* ward (or none) and its `shell`/`write_file` tools scope to the wrong directory.

**Files:**
- Modify: `gateway/gateway-execution/src/delegation/spawn.rs` — add `effective_ward_id` free fn; use it where `session_ward_id` is bound (currently lines 214–218).
- Test: same file, in `#[cfg(test)] mod tests`.

- [ ] **Step 1: Write the failing tests**

Add to (or create) the `#[cfg(test)] mod tests` block in `spawn.rs`:

```rust
#[test]
fn effective_ward_id_uses_ward_prefix_over_parent() {
    assert_eq!(
        effective_ward_id("ward:maritime", Some("finance".to_string())),
        Some("maritime".to_string())
    );
}

#[test]
fn effective_ward_id_falls_back_to_parent_for_normal_agents() {
    assert_eq!(
        effective_ward_id("planner", Some("finance".to_string())),
        Some("finance".to_string())
    );
    assert_eq!(effective_ward_id("planner", None), None);
}
```

- [ ] **Step 2: Run the tests, verify they fail to compile**

Run: `cargo test -p gateway-execution --lib effective_ward_id 2>&1 | tail -12`
Expected: compile error — `cannot find function effective_ward_id`.

- [ ] **Step 3: Implement `effective_ward_id`**

Add to `spawn.rs` at module scope:

```rust
/// The ward directory a delegated agent should operate in. A `ward:<name>`
/// delegation always runs in its own ward, regardless of the parent's
/// active ward; any other agent inherits the parent session's ward.
fn effective_ward_id(child_agent_id: &str, parent_ward_id: Option<String>) -> Option<String> {
    match child_agent_id.strip_prefix("ward:") {
        Some(name) => Some(name.to_string()),
        None => parent_ward_id,
    }
}
```

- [ ] **Step 4: Wire it into `spawn_delegated_agent`**

In `spawn.rs`, replace the current `session_ward_id` binding (lines 214–218):

```rust
    // Look up active ward from parent session
    let session_ward_id = state_service
        .get_session(&request.session_id)
        .ok()
        .flatten()
        .and_then(|s| s.ward_id);
```

with:

```rust
    // Look up the parent session's ward, then resolve the ward this
    // delegation actually runs in: a `ward:<name>` target runs in its
    // own ward; everything else inherits the parent's.
    let parent_ward_id = state_service
        .get_session(&request.session_id)
        .ok()
        .flatten()
        .and_then(|s| s.ward_id);
    let session_ward_id = effective_ward_id(&request.child_agent_id, parent_ward_id);
```

No other change is needed — `session_ward_id` is already threaded to `builder.build(..., session_ward_id.as_deref())` (line ~319), which sets the executor's `ward_id` state so `shell`/`write_file` scope to `wards/<ward>/`. (The ward-context injection at lines 221–265 will now also fire for ward-agents — a redundant second copy of `AGENTS.md`; harmless in P1, removed in P2.)

- [ ] **Step 5: Run the tests, verify they pass**

Run: `cargo test -p gateway-execution --lib effective_ward_id 2>&1 | tail -10`
Expected: `test result: ok. 2 passed`.

- [ ] **Step 6: Build + lint**

Run: `cargo clippy -p gateway-execution --all-targets -- -D warnings 2>&1 | tail -8`
Expected: `Finished`, no warnings.

- [ ] **Step 7: Commit**

```bash
git add gateway/gateway-execution/src/delegation/spawn.rs
git commit -m "feat(ward-as-agent): ward delegations run in their own ward directory"
```

---

## Task 5: End-to-end smoke verification

No automated test — this verifies the full delegation pipeline against a running daemon.

**Files:** none (manual verification).

- [ ] **Step 1: Build the daemon**

Run: `cargo build -p daemon 2>&1 | tail -5`
Expected: `Finished`.

- [ ] **Step 2: Seed a test ward**

Ensure `~/Documents/zbot/wards/smoke-test/AGENTS.md` exists with:

```markdown
# smoke-test
## Purpose / Scope
IN — answering a single fixed test question.
## Handoff
Return: { status, summary }
```

- [ ] **Step 3: Run the daemon and delegate to the ward**

Start the daemon (`npm run daemon:watch`), then from a root session issue a task that triggers `delegate_to_agent(agent_id="ward:smoke-test", task="reply with the word READY", wait_for_result=true)`.

Expected:
- daemon logs `Synthesized ward-agent for delegation  ward=smoke-test`
- a child execution runs with cwd `wards/smoke-test/`
- the root receives the child's result (blocked on `wait_for_result`)

- [ ] **Step 4: Verify failure mode**

Delegate to `ward:does-not-exist`.
Expected: the child execution crashes with a clear error `ward 'does-not-exist' has no directory at …` — it does not hang or silently no-op.

- [ ] **Step 5: Commit (if any fixture files were added)**

Only if a tracked fixture was created; the `~/Documents/zbot` vault is not in the repo, so normally nothing to commit here.

---

## Self-Review

**Spec coverage (design doc §4 / §10 P1):**
- `ward:` prefix → `synthesize_ward_agent` → Task 2/3 ✔
- ward-agent runs with cwd = ward dir → Task 4 ✔
- full capability inventory → inherited unchanged from the standard `ExecutorBuilder` spawn path (no task needed; verified in Task 5) ✔
- `wait_for_result` contract → already supported by `delegate_to_agent`; no change needed (verified in Task 5) ✔
- create-on-miss → **explicitly out of P1** (cold path); missing ward dir errors — Task 2/3 Step 1 test ✔

**Type consistency:** `synthesize_ward_agent` and `load_or_create_specialist` both return `Result<(gateway_services::agents::Agent, Provider), String>`. `compose_ward_agent_instructions(identity, paths, ward_name, doctrine)` — same arg order at definition (Task 1 Step 3) and call site (Task 2 Step 3). `effective_ward_id(child_agent_id, parent_ward_id)` — same at definition and call site.

**Deferred to later phases:** L1–L5 generic prompt + first-turn protocol (P2); `list_agents` exposure + graduation gate (P3); `out_of_scope`/`capability_missing` returns (P4); removing the redundant ward-context injection (P2).
