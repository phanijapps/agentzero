//! First-boot default seeding: agents/skills/cron/policies. Extracted
//! from state/mod.rs in W5 of the gateway decompose.

use super::seeded_defaults;
use super::AppState;

impl AppState {
    /// Seed default agents and other initial data.
    ///
    /// This should be called after creating the state to set up default subagents
    /// that can be delegated to.
    pub async fn seed_defaults(&self) {
        // Get default provider ID
        let providers = self.provider_service().list().unwrap_or_default();
        let selected = gateway_services::select_provider(&providers, None);
        let default_provider_id = selected
            .and_then(|p| p.id.clone())
            .unwrap_or_else(|| "default".to_string());

        // Resolve default model from default provider (first model in list)
        let default_model = selected
            .map(|p| p.default_model().to_string())
            .unwrap_or_else(|| "gpt-4o".to_string());

        // Seed default agents from bundled templates (configs + AGENTS.md instructions)
        let agent_template =
            gateway_templates::Templates::get("default_agents.json").map(|f| f.data.to_vec());
        if let Err(e) = self
            .agents()
            .seed_default_agents(
                &default_provider_id,
                &default_model,
                agent_template.as_deref(),
                |name| {
                    let path = format!("agents/{}.md", name);
                    gateway_templates::Templates::get(&path)
                        .map(|f| String::from_utf8_lossy(&f.data).to_string())
                },
            )
            .await
        {
            tracing::warn!("Failed to seed default agents: {}", e);
        }

        // Seed default skills from bundled templates if skills dir is empty
        self.seed_default_skills();

        // Seed default cron jobs (idempotent on job id) so first-run
        // installs ship with the bundled cleanup schedule wired up.
        self.seed_default_cron().await;

        // Seed default policies from bundled template if no policies exist
        self.seed_default_policies().await;

        // Preload skills into cache
        if let Err(e) = self.skills().preload().await {
            tracing::warn!("Failed to preload skills: {}", e);
        }

        // Seed required workspace structure. Runtime environments are created
        // only by an explicit tool/runtime action, never during startup.
        self.ensure_runtime_environments().await;

        // Discover and start plugins
        self.discover_and_start_plugins().await;
    }

    /// Seed default skills from bundled templates if skills directory is empty.
    pub(crate) fn seed_default_skills(&self) {
        let skills_dir = self.paths().vault_dir().join("skills");

        // Only seed if skills dir is empty or doesn't exist
        let has_skills = skills_dir.exists()
            && std::fs::read_dir(&skills_dir)
                .map(|mut entries| entries.next().is_some())
                .unwrap_or(false);

        if has_skills {
            tracing::debug!("Skills directory not empty, skipping seed");
            return;
        }

        tracing::info!("Seeding default skills from bundled templates");
        std::fs::create_dir_all(&skills_dir).ok();

        // Iterate all embedded files under skills/
        for path in gateway_templates::Templates::iter() {
            let path_str = path.as_ref();
            if !path_str.starts_with("skills/") {
                continue;
            }

            // path_str is like "skills/coding/SKILL.md" or "skills/yfinance-market-analysis/scripts/run.py"
            let dest = self.paths().vault_dir().join(path_str);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).ok();
            }

            if let Some(file) = gateway_templates::Templates::get(path_str) {
                if let Err(e) = std::fs::write(&dest, &file.data) {
                    tracing::warn!("Failed to seed skill file {}: {}", path_str, e);
                }
            }
        }

        let count = std::fs::read_dir(&skills_dir)
            .map(|entries| entries.count())
            .unwrap_or(0);
        tracing::info!("Seeded {} default skills", count);
    }

    /// Seed default cron jobs from bundled `default_cron.json` template.
    ///
    /// Each ID is seeded **at most once per vault**: the first time we see
    /// it, we create the job (or migrate a pre-existing one) and record the
    /// ID in `<vault>/config/seeded-defaults.json`. Subsequent boots skip
    /// any ID already in the registry, so deletes the user makes through
    /// the UI stick across daemon restarts.
    pub(crate) async fn seed_default_cron(&self) {
        let template_bytes = match gateway_templates::Templates::get("default_cron.json") {
            Some(file) => file.data.to_vec(),
            None => {
                tracing::debug!(
                    "seed_default_cron: bundled `default_cron.json` not found, skipping"
                );
                return;
            }
        };

        let requests: Vec<gateway_cron::CreateCronJobRequest> =
            match serde_json::from_slice(&template_bytes) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("seed_default_cron: failed to parse default_cron.json: {e}");
                    return;
                }
            };

        if requests.is_empty() {
            tracing::debug!("seed_default_cron: no entries in default_cron.json");
            return;
        }

        let cron_service = gateway_cron::CronService::new(self.paths().clone());
        let seeded =
            seeded_defaults::seed_cron_with_registry(&self.paths(), &cron_service, requests).await;

        if seeded > 0 {
            tracing::info!(seeded, "seed_default_cron: completed");
        }
    }

    /// Seed default policies from bundled template if no policies/corrections exist.
    pub(crate) async fn seed_default_policies(&self) {
        // Route through the trait surface so Engram receives the same
        // default policy seed data as the rest of the runtime.
        let memory_store_slot = self.memory_store();
        let memory_store = match memory_store_slot.as_deref() {
            Some(s) => s,
            None => {
                tracing::warn!(
                    "seed_default_policies: memory_store is None — refusing to seed. \
                     Check persistence_factory output."
                );
                return;
            }
        };

        // Check if any correction facts already exist for the root agent.
        let existing = match memory_store
            .list_memory_facts(Some("root"), Some("correction"), None, 1, 0)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(
                    "seed_default_policies: existence check failed ({e}); \
                     proceeding as if empty (may produce duplicates if policies \
                     are already present)."
                );
                Vec::new()
            }
        };
        if !existing.is_empty() {
            tracing::debug!(
                existing_count = existing.len(),
                "seed_default_policies: policies already present for root/correction — skipping"
            );
            return;
        }

        let template = match gateway_templates::Templates::get("default_policies.json") {
            Some(f) => f.data.to_vec(),
            None => {
                tracing::warn!(
                    "seed_default_policies: bundled `default_policies.json` template \
                     missing from gateway-templates — nothing to seed."
                );
                return;
            }
        };

        let policies: Vec<serde_json::Value> = match serde_json::from_slice(&template) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("seed_default_policies: failed to parse default_policies.json: {e}");
                return;
            }
        };

        let total = policies.len();
        let now = chrono::Utc::now().to_rfc3339();
        let mut count = 0usize;
        let mut skipped_empty = 0usize;
        let mut errors: Vec<(String, String)> = Vec::new();

        for policy in &policies {
            let category = policy["category"].as_str().unwrap_or("correction");
            let key = policy["key"].as_str().unwrap_or_default();
            let content = policy["content"].as_str().unwrap_or_default();
            let confidence = policy["confidence"].as_f64().unwrap_or(1.0);
            let pinned = policy["pinned"].as_bool().unwrap_or(true);

            if key.is_empty() || content.is_empty() {
                skipped_empty += 1;
                continue;
            }

            let fact_value = zbot_stores_domain::MemoryFact {
                id: format!("policy-{}", uuid::Uuid::new_v4()),
                session_id: None,
                agent_id: "root".to_string(),
                scope: "agent".to_string(),
                category: category.to_string(),
                key: key.to_string(),
                content: content.to_string(),
                confidence,
                mention_count: 5,
                source_summary: Some("Default policy".to_string()),
                ward_id: "__global__".to_string(),
                contradicted_by: None,
                created_at: now.clone(),
                updated_at: now.clone(),
                expires_at: None,
                valid_from: None,
                valid_until: None,
                superseded_by: None,
                pinned,
                epistemic_class: Some("current".to_string()),
                source_episode_id: None,
                source_ref: None,
                embedding: None,
                last_accessed: None,
            };

            match memory_store.upsert_typed_fact(fact_value, None).await {
                Ok(()) => count += 1,
                Err(e) => errors.push((key.to_string(), e.to_string())),
            }
        }

        if !errors.is_empty() {
            for (key, e) in &errors {
                tracing::warn!(policy_key = %key, error = %e, "seed_default_policies: upsert failed");
            }
        }

        tracing::info!(
            total = total,
            seeded = count,
            skipped_empty = skipped_empty,
            failed = errors.len(),
            "seed_default_policies: completed"
        );
    }
}
