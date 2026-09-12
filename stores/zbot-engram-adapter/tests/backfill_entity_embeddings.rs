
// One-time production backfill for entity name embeddings.
//
// Run manually (daemon stopped or idle — SQLite lock):
//   ZBOT_VAULT="$HOME/Documents/zbot/data/engram" \
//   cargo test -p zbot-engram-adapter --test backfill_entity_embeddings -- --ignored --nocapture
//
// Idempotent: only touches rows where embedding_json IS NULL.
#[tokio::test]
#[ignore = "production backfill — sets ZBOT_VAULT and run once"]
async fn backfill_entity_name_embeddings() {
    let vault = std::env::var("ZBOT_VAULT").expect("ZBOT_VAULT must point at the engram data dir");
    let config = zbot_engram_adapter::config::AdapterConfig::engram_for_data_root(
        std::path::PathBuf::from(vault),
        "engram.db",
    );
    // Embedding provider comes from the same config the daemon uses.
    let provider = zbot_engram_adapter::bootstrap::EngramProvider::open(config.clone())
        .expect("open engram provider");
    let sidecars = zbot_engram_adapter::EngramSidecarStores::from_provider(config.clone(), &provider)
        .expect("open sidecar stores");

    // The embedding client: same OpenAI-compatible surface the daemon uses.
    // Source: memory facts' stored identity (ground truth for this vault).
    let base_url =
        std::env::var("ZBOT_EMBED_BASE_URL").unwrap_or_else(|_| "http://localhost:11434/v1".into());
    let model = std::env::var("ZBOT_EMBED_MODEL").expect("ZBOT_EMBED_MODEL (e.g. nomic-embed-text)");
    use agent_runtime::llm::embedding::EmbeddingClient as _;
    let client = agent_runtime::llm::openai_embedding::OpenAiEmbeddingClient::new(
        base_url,
        String::new(),
        model.clone(),
        0,
    );

    let mut done = 0usize;
    loop {
        let batch = sidecars
            .entities_missing_name_embeddings(64)
            .expect("query missing embeddings");
        if batch.is_empty() {
            break;
        }
        let names: Vec<String> = batch.iter().map(|(_, name)| name.clone()).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let embeddings = match client.embed(&refs).await {
            Ok(embeddings) => embeddings,
            Err(error) => panic!("embedding call failed: {error}"),
        };
        for ((id, _), embedding) in batch.iter().zip(embeddings.iter()) {
            sidecars
                .set_entity_name_embedding(id, embedding)
                .expect("write embedding");
            done += 1;
        }
        println!("backfilled {done} entity name embeddings…");
    }
    println!("BACKFILL COMPLETE: {done} entities embedded");
}
