use zbot_engram_adapter::{DependencyChecklist, DependencyEvidence, DependencyItemStatus};

#[test]
fn dependency_checklist_blocks_provider_selection_until_required_items_are_recorded() {
    let mut checklist = DependencyChecklist::current();

    let blockers = checklist.provider_selection_blockers();
    assert!(blockers.contains(&"evidence.cargo_metadata".to_string()));
    assert!(blockers.contains(&"evidence.lockfile".to_string()));
    assert!(blockers.contains(&"evidence.dependency_scanner".to_string()));
    assert!(blockers.contains(&"evidence.source_revision".to_string()));
    assert!(blockers.contains(&"evidence.dirty_state_policy".to_string()));
    assert!(!checklist.provider_selection_ready());

    checklist.evidence = DependencyEvidence {
        cargo_metadata: true,
        lockfile: true,
        dependency_scanner: true,
        source_revision: Some("mem-alpha git 3af132299de6747685218220a41551670dc74a83".to_string()),
        dirty_state_policy: Some("ignore untracked local tool cache only".to_string()),
    };

    assert!(checklist.provider_selection_ready());
}

#[test]
fn dependency_checklist_records_source_and_dirty_state_evidence_without_private_paths() {
    let mut checklist = DependencyChecklist::current();
    checklist.evidence = DependencyEvidence {
        cargo_metadata: true,
        lockfile: true,
        dependency_scanner: true,
        source_revision: Some("mem-alpha git 3af132299de6747685218220a41551670dc74a83".to_string()),
        dirty_state_policy: Some(
            "untracked tool-cache directories are not release inputs".to_string(),
        ),
    };

    let json = serde_json::to_string(&checklist).expect("json");

    assert!(!json.contains("/home/"));
    assert!(!json.contains("projects/mem-alpha"));
    assert!(!json.contains("Cargo metadata path"));
    assert!(json.contains("3af132299de6747685218220a41551670dc74a83"));
    assert!(checklist
        .items
        .iter()
        .any(|item| item.id == "retrieval_ranking_trace"
            && item.status == DependencyItemStatus::Missing
            && !item.required_for_provider_selection));
}
