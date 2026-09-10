//! Integration tests for ward-agent synthesis via `load_or_create_specialist`.
//!
//! These tests verify that `delegate_to_agent(agent_id="ward:<name>")` resolves
//! to a synthesized ward-agent rather than falling through to the generic
//! auto-create path.

use std::sync::Arc;

use agent_primitives::vault_paths::VaultPaths;
use gateway_execution::invoke::setup::AgentLoader;
use gateway_services::{AgentService, ProviderService, SettingsService};

// ============================================================================
// HELPERS
// ============================================================================

/// Seed a minimal `config/providers.json` so the provider resolver can find
/// a default provider. Without this `get_default()` returns an error.
fn seed_default_provider(paths: &Arc<VaultPaths>) {
    let providers_json = serde_json::json!([
        {
            "id": "test-provider",
            "name": "Test Provider",
            "description": "Test provider for integration tests",
            "apiKey": "test-key",
            "baseUrl": "https://api.example.com",
            "models": ["test-model"],
            "defaultModel": "test-model",
            "isDefault": true
        }
    ]);
    let content = serde_json::to_string_pretty(&providers_json).unwrap();
    std::fs::write(paths.providers(), content).unwrap();
}

/// Build a test `AgentLoader` backed by real on-disk services.
///
/// The caller owns the `AgentService` and `ProviderService` — `AgentLoader`
/// borrows them for its lifetime.
fn make_services(paths: &Arc<VaultPaths>) -> (Arc<AgentService>, Arc<ProviderService>) {
    let agent_service = Arc::new(AgentService::new(paths.agents_dir()));
    let provider_service = Arc::new(ProviderService::new(paths.clone()));
    (agent_service, provider_service)
}

// ============================================================================
// TESTS
// ============================================================================

#[tokio::test]
async fn load_or_create_root_ignores_on_disk_root_agent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    paths.ensure_dirs_exist().unwrap();
    seed_default_provider(&paths);

    let root_dir = paths.agents_dir().join("root");
    std::fs::create_dir_all(&root_dir).unwrap();
    std::fs::write(
        root_dir.join("config.yaml"),
        "name: root\n\
         displayName: Evil Root\n\
         description: persisted root shadow\n\
         providerId: test-provider\n\
         model: test-model\n\
         temperature: 0.7\n\
         maxTokens: 8192\n\
         thinkingEnabled: false\n\
         voiceRecordingEnabled: false\n\
         skills: []\n\
         mcps: []\n",
    )
    .unwrap();
    std::fs::write(root_dir.join("AGENTS.md"), "malicious persisted root\n").unwrap();

    let (agent_service, provider_service) = make_services(&paths);
    let loader = AgentLoader::new(&agent_service, &provider_service, paths.clone());

    let (agent, provider) = loader
        .load_or_create_root("root")
        .await
        .expect("root should synthesize");

    assert_eq!(agent.id, "root");
    assert_eq!(agent.display_name, "Root Agent");
    assert_eq!(agent.agent_type.as_deref(), Some("orchestrator"));
    assert_eq!(agent.provider_id, "test-provider");
    assert_eq!(provider.name, "Test Provider");
    assert!(!agent.instructions.contains("malicious persisted root"));
}

#[tokio::test]
async fn load_or_create_root_disables_thinking_in_chat_mode() {
    let tmp = tempfile::TempDir::new().unwrap();
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    paths.ensure_dirs_exist().unwrap();
    seed_default_provider(&paths);

    let settings = SettingsService::new(paths.clone());
    let mut exec = settings.get_execution_settings().unwrap_or_default();
    exec.orchestrator.thinking_enabled = true;
    settings.update_execution_settings(exec).unwrap();

    let (agent_service, provider_service) = make_services(&paths);
    let research_loader =
        AgentLoader::new(&agent_service, &provider_service, paths.clone()).with_settings(&settings);
    let chat_loader = AgentLoader::new(&agent_service, &provider_service, paths.clone())
        .with_settings(&settings)
        .with_chat_mode(true);

    let (research_agent, _) = research_loader
        .load_or_create_root("root")
        .await
        .expect("research root should synthesize");
    let (chat_agent, _) = chat_loader
        .load_or_create_root("root")
        .await
        .expect("chat root should synthesize");

    assert!(
        research_agent.thinking_enabled,
        "research mode should honor orchestrator thinking"
    );
    assert!(
        !chat_agent.thinking_enabled,
        "Quick Chat must not request model reasoning"
    );
}

#[tokio::test]
async fn load_or_create_specialist_rejects_reserved_root_agent_id() {
    let tmp = tempfile::TempDir::new().unwrap();
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    paths.ensure_dirs_exist().unwrap();
    seed_default_provider(&paths);

    let (agent_service, provider_service) = make_services(&paths);
    let loader = AgentLoader::new(&agent_service, &provider_service, paths.clone());

    let err = loader
        .load_or_create_specialist("root")
        .await
        .expect_err("delegated root should be rejected");

    assert!(err.to_string().contains("Reserved system agent id"));
}

/// Happy path: a ward directory with an `AGENTS.md` doctrine file produces
/// a synthesized ward-agent whose `agent_type` is `"ward"`, whose `id` is
/// `"ward:<name>"`, and whose instructions contain both the ward-agent
/// identity text and the doctrine content.
#[tokio::test]
async fn load_or_create_specialist_synthesizes_ward_agent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    paths.ensure_dirs_exist().unwrap();

    // Seed a provider so the resolver doesn't return an error.
    seed_default_provider(&paths);

    // Seed a ward directory with doctrine.
    let ward_dir = paths.ward_dir("maritime");
    std::fs::create_dir_all(&ward_dir).unwrap();
    std::fs::write(
        ward_dir.join("AGENTS.md"),
        "# maritime\n## Purpose\nVessel tracking and AIS analysis.\n",
    )
    .unwrap();

    let (agent_service, provider_service) = make_services(&paths);
    let loader = AgentLoader::new(&agent_service, &provider_service, paths.clone());

    let (agent, _provider) = loader
        .load_or_create_specialist("ward:maritime")
        .await
        .expect("ward-agent synthesis should succeed");

    assert_eq!(agent.id, "ward:maritime");
    assert_eq!(agent.agent_type.as_deref(), Some("ward"));
    assert!(
        agent.instructions.contains("ward-agent"),
        "instructions should contain 'ward-agent'"
    );
    assert!(
        agent
            .instructions
            .contains("Vessel tracking and AIS analysis."),
        "instructions should contain doctrine content"
    );
    assert!(
        agent
            .instructions
            .contains("# --- WARD DOCTRINE: maritime ---"),
        "instructions should contain doctrine header"
    );
    assert!(agent.instructions.contains("already bound to the ward"));
    assert!(agent.instructions.contains("call update_plan"));
    assert!(!agent.instructions.contains("ward(action=\"use\")"));
}

/// Error path: delegating to a `ward:` id whose directory does not exist
/// must return an `Err` containing the ward name (P1: no auto-create).
#[tokio::test]
async fn load_or_create_specialist_errors_when_ward_dir_missing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
    paths.ensure_dirs_exist().unwrap();

    // Seed a provider so the resolver itself doesn't fail first.
    seed_default_provider(&paths);

    let (agent_service, provider_service) = make_services(&paths);
    let loader = AgentLoader::new(&agent_service, &provider_service, paths.clone());

    let err = loader
        .load_or_create_specialist("ward:nonexistent")
        .await
        .expect_err("missing ward dir must error in P1");

    assert!(
        err.to_string().contains("nonexistent"),
        "error message should contain the ward name; got: {err}"
    );
}
