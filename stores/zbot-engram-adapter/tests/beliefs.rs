use chrono::{DateTime, TimeZone, Utc};
use engram_domain::BeliefStatus;
use zbot_engram_adapter::{
    mapping::belief::{belief_record_to_belief, belief_to_belief_record},
    AdapterConfig, AdapterFeature, CapabilityReport, EngramBeliefStore,
};
use zbot_stores_traits::{
    Belief, BeliefContradiction, BeliefContradictionStore, BeliefStore, ContradictionType,
    EmbeddingQueryIdentity, Resolution,
};

fn engram_config(root: &tempfile::TempDir) -> AdapterConfig {
    let mut config = AdapterConfig::engram_for_data_root(root.path(), "engram.db");
    config.embedding_provider.dimensions = 2;
    config
}

fn query_identity() -> EmbeddingQueryIdentity {
    EmbeddingQueryIdentity {
        provider_type: "fastembed".to_string(),
        model: "BAAI/bge-small-en-v1.5".to_string(),
        dimensions: 2,
        prompt_profile: "query".to_string(),
        normalization: None,
    }
}

fn ts(year: i32, month: u32, day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
}

fn embedding_bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn belief(id: &str, subject: &str, content: &str, valid_from: DateTime<Utc>) -> Belief {
    Belief {
        id: id.to_string(),
        partition_id: "root".to_string(),
        subject: subject.to_string(),
        content: content.to_string(),
        confidence: 0.82,
        valid_from: Some(valid_from),
        valid_until: None,
        source_fact_ids: vec![format!("fact-{id}")],
        synthesizer_version: 3,
        reasoning: Some("synthesized from source facts".to_string()),
        created_at: valid_from,
        updated_at: valid_from,
        superseded_by: None,
        stale: false,
        embedding: Some(embedding_bytes(&[1.0, 0.0])),
    }
}

fn contradiction(id: &str, a: &str, b: &str) -> BeliefContradiction {
    BeliefContradiction {
        id: id.to_string(),
        belief_a_id: a.to_string(),
        belief_b_id: b.to_string(),
        contradiction_type: ContradictionType::Logical,
        severity: 0.9,
        judge_reasoning: Some("beliefs cannot both be current".to_string()),
        detected_at: ts(2026, 7, 6),
        resolved_at: None,
        resolution: None,
    }
}

#[test]
fn belief_mapping_preserves_valid_time_sources_and_metadata() {
    let config = AdapterConfig::engram_for_data_root(std::env::temp_dir(), "engram.db");
    let mapper = config.scope_mapper().expect("scope mapper");
    let belief = belief(
        "belief-current",
        "user.language",
        "User prefers Rust",
        ts(2026, 1, 1),
    );

    let record = belief_to_belief_record(&belief, &mapper).expect("record");
    let round_trip = belief_record_to_belief(&record).expect("round trip");

    assert_eq!(record.id.as_str(), "belief-current");
    assert_eq!(record.scope.workspace.as_deref(), Some("root"));
    assert_eq!(record.subject.key, "user.language");
    assert_eq!(record.content, "User prefers Rust");
    assert_eq!(record.status, BeliefStatus::Active);
    assert_eq!(record.confidence, 0.82);
    assert_eq!(record.sources.len(), 1);
    assert_eq!(record.sources[0].target_id, "fact-belief-current");
    assert_eq!(record.valid_from, Some(ts(2026, 1, 1)));
    assert_eq!(
        record
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("synthesizerVersion"))
            .and_then(|value| value.as_i64()),
        Some(3)
    );
    assert_eq!(round_trip.id, belief.id);
    assert_eq!(round_trip.partition_id, belief.partition_id);
    assert_eq!(round_trip.subject, belief.subject);
    assert_eq!(round_trip.source_fact_ids, belief.source_fact_ids);
    assert_eq!(round_trip.reasoning, belief.reasoning);
    assert_eq!(round_trip.embedding, None);

    let mut historical = belief.clone();
    historical.id = "belief-historical".to_string();
    historical.valid_until = Some(ts(2026, 6, 1));
    let historical_record = belief_to_belief_record(&historical, &mapper).expect("historical");
    assert_eq!(historical_record.status, BeliefStatus::Active);
}

#[tokio::test]
async fn belief_store_round_trip_valid_time_stale_source_and_search() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramBeliefStore::open(engram_config(&root)).expect("store");
    let mut old = belief(
        "belief-old",
        "user.language",
        "User preferred Python",
        ts(2026, 1, 1),
    );
    old.valid_until = Some(ts(2026, 3, 1));
    old.embedding = Some(embedding_bytes(&[0.0, 1.0]));
    let current = belief(
        "belief-current",
        "user.language",
        "User prefers Rust",
        ts(2026, 4, 1),
    );

    store.upsert_belief(&old).await.expect("old belief");
    store.upsert_belief(&current).await.expect("current belief");

    let historical = store
        .get_belief("root", "user.language", Some(ts(2026, 2, 1)))
        .await
        .expect("historical")
        .expect("old belief");
    assert_eq!(historical.id, "belief-old");

    let active = store
        .get_belief("root", "user.language", Some(ts(2026, 5, 1)))
        .await
        .expect("active")
        .expect("current belief");
    assert_eq!(active.id, "belief-current");

    let listed = store.list_beliefs("root", 10).await.expect("list");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, "belief-current");
    assert!(store
        .list_beliefs("root", 0)
        .await
        .expect("zero-list")
        .is_empty());

    assert!(store
        .search_beliefs("root", &[1.0, 0.0], 5)
        .await
        .expect("legacy search")
        .is_empty());
    let hits = store
        .search_beliefs_with_identity("root", &[1.0, 0.0], Some(&query_identity()), 5)
        .await
        .expect("search");
    assert_eq!(hits[0].belief.id, "belief-current");
    assert!(hits[0].score > 0.99);
    let mut same_basename = query_identity();
    same_basename.model = "other/bge-small-en-v1.5".to_string();
    assert!(store
        .search_beliefs_with_identity("root", &[1.0, 0.0], Some(&same_basename), 5)
        .await
        .expect("same-basename mismatch")
        .is_empty());
    assert!(store
        .search_beliefs_with_identity("root", &[1.0, 0.0], Some(&query_identity()), 0)
        .await
        .expect("zero-search")
        .is_empty());

    store
        .mark_stale("belief-current")
        .await
        .expect("mark stale");
    let stale = store.list_stale("root", 10).await.expect("stale");
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].id, "belief-current");
    assert!(store
        .list_stale("root", 0)
        .await
        .expect("zero-stale")
        .is_empty());
    store
        .clear_stale("belief-current")
        .await
        .expect("clear stale");
    assert!(store
        .list_stale("root", 10)
        .await
        .expect("stale after clear")
        .is_empty());

    assert_eq!(
        store
            .beliefs_referencing_fact("fact-belief-current")
            .await
            .expect("source refs"),
        vec!["belief-current".to_string()]
    );

    store
        .supersede_belief("belief-current", "belief-next", ts(2026, 6, 1))
        .await
        .expect("supersede");
    let superseded = store
        .get_belief_by_id("belief-current")
        .await
        .expect("get superseded")
        .expect("belief");
    assert_eq!(superseded.superseded_by.as_deref(), Some("belief-next"));
    assert_eq!(superseded.valid_until, Some(ts(2026, 6, 1)));

    store
        .retract_belief("belief-old", ts(2026, 2, 15))
        .await
        .expect("retract");
    assert_eq!(
        store
            .get_belief_by_id("belief-old")
            .await
            .expect("old")
            .expect("belief")
            .valid_until,
        Some(ts(2026, 2, 15))
    );
}

#[tokio::test]
async fn contradictions_round_trip_canonicalize_and_resolve() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramBeliefStore::open(engram_config(&root)).expect("store");
    let a = belief("belief-a", "user.language.a", "A", ts(2026, 1, 1));
    let z = belief("belief-z", "user.language.z", "Z", ts(2026, 1, 1));
    store.upsert_belief(&a).await.expect("a");
    store.upsert_belief(&z).await.expect("z");

    store
        .insert_contradiction(&contradiction("contradiction-1", "belief-z", "belief-a"))
        .await
        .expect("insert");
    store
        .insert_contradiction(&contradiction("contradiction-2", "belief-a", "belief-z"))
        .await
        .expect("duplicate no-op");

    assert!(store
        .pair_exists("belief-z", "belief-a")
        .await
        .expect("pair exists"));

    let for_a = store.for_belief("belief-a").await.expect("for belief");
    assert_eq!(for_a.len(), 1);
    assert_eq!(for_a[0].id, "contradiction-1");
    assert_eq!(for_a[0].belief_a_id, "belief-a");
    assert_eq!(for_a[0].belief_b_id, "belief-z");

    let recent = store.list_recent("root", 10).await.expect("recent");
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].contradiction_type, ContradictionType::Logical);
    assert!(store
        .list_recent("root", 0)
        .await
        .expect("zero-recent")
        .is_empty());

    store
        .resolve("contradiction-1", Resolution::Compatible)
        .await
        .expect("resolve");
    let resolved = store
        .for_belief("belief-z")
        .await
        .expect("resolved")
        .pop()
        .expect("row");
    assert_eq!(resolved.resolution, Some(Resolution::Compatible));
    assert!(resolved.resolved_at.is_some());
}

#[tokio::test]
async fn record_time_history_is_explicitly_unsupported() {
    let root = tempfile::tempdir().expect("root");
    let store = EngramBeliefStore::open(engram_config(&root)).expect("store");

    let err = store
        .get_belief_recorded_at("root", "user.language", Utc::now(), Utc::now())
        .await
        .expect_err("record-time history unsupported");

    assert!(err.contains("unsupported: record_time_history"));
}

#[test]
fn belief_capabilities_can_be_enabled_independently() {
    let root = tempfile::tempdir().expect("root");
    let config = engram_config(&root);

    let report = CapabilityReport::from_verified_features(
        &config,
        [AdapterFeature::Beliefs, AdapterFeature::Contradictions],
    );

    assert!(report.supports(AdapterFeature::Beliefs));
    assert!(report.supports(AdapterFeature::Contradictions));
    assert!(!report.supports(AdapterFeature::Recall));
    assert!(!report.supports(AdapterFeature::Migration));
    assert!(!report.supports(AdapterFeature::Auxiliary));
}
