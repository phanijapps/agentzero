use gateway_a2a::{
    agent_card, supported_optional_operations, supported_paths, AgentCardConfig, AgentSkillConfig,
};
use serde_json::Value;
use std::collections::BTreeSet;

#[test]
fn openapi_matches_supported_surface() {
    let openapi: Value = serde_yaml::from_str(include_str!(
        "../../../contracts/openapi/a2a-federation.yaml"
    ))
    .expect("openapi yaml parses");

    assert_eq!(openapi["x-spec"][0], "docs/specs/a2a-federation-discovery/");
    assert_all_refs_resolve(&openapi);

    let actual_paths = openapi["paths"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_paths, supported_paths());

    let unsupported = openapi["x-a2a-unsupported-operations"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(unsupported, supported_optional_operations());

    let card = agent_card(AgentCardConfig {
        name: "Research zBot".into(),
        description: "Bounded remote text work".into(),
        version: "2026.8.4".into(),
        base_url: "https://peer.example.test".into(),
        skills: vec![AgentSkillConfig {
            id: "research".into(),
            name: "Research".into(),
            description: "Answers bounded text prompts".into(),
            tags: vec!["research".into()],
            examples: vec![],
        }],
    })
    .unwrap();
    let card_value = serde_json::to_value(card).unwrap();
    let capabilities = &card_value["capabilities"];
    assert_eq!(
        capabilities["streaming"],
        openapi["components"]["schemas"]["AgentCapabilities"]["properties"]["streaming"]["const"]
    );
    assert_eq!(
        capabilities["pushNotifications"],
        openapi["components"]["schemas"]["AgentCapabilities"]["properties"]["pushNotifications"]
            ["const"]
    );
    assert_eq!(
        capabilities["extendedAgentCard"],
        openapi["components"]["schemas"]["AgentCapabilities"]["properties"]["extendedAgentCard"]
            ["const"]
    );
}

fn assert_all_refs_resolve(root: &Value) {
    fn walk(root: &Value, value: &Value) {
        match value {
            Value::Object(map) => {
                if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
                    assert!(
                        resolve_ref(root, reference).is_some(),
                        "unresolved {reference}"
                    );
                }
                for child in map.values() {
                    walk(root, child);
                }
            }
            Value::Array(values) => {
                for child in values {
                    walk(root, child);
                }
            }
            _ => {}
        }
    }

    walk(root, root);
}

fn resolve_ref<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
    let pointer = reference.strip_prefix('#')?;
    root.pointer(pointer)
}
