//! Field-name → relationship extraction rules for the Ward Artifact Indexer.
//!
//! Each rule inspects a JSON object and, when its field pattern matches,
//! emits candidate `(source_name, RelationshipType, target_name, target_type)`
//! tuples. The caller resolves names to entity IDs via `EntityResolver`.

use knowledge_graph::{EntityType, RelationshipType};
use serde_json::{Map, Value};

/// A pending relationship with resolved-by-name endpoints.
#[derive(Debug, Clone, PartialEq)]
pub struct RelationshipCandidate {
    pub source_name: String,
    pub source_type: EntityType,
    pub target_name: String,
    pub target_type: EntityType,
    pub relationship_type: RelationshipType,
}

/// Apply every rule to an object given its inferred source entity (name + type).
/// Returns zero or more relationship candidates. Unknown fields are ignored.
pub fn extract(
    source_name: &str,
    source_type: EntityType,
    obj: &Map<String, Value>,
) -> Vec<RelationshipCandidate> {
    let mut out = Vec::new();
    for rule in RULES {
        rule(source_name, source_type.clone(), obj, &mut out);
    }
    out
}

type Rule = fn(&str, EntityType, &Map<String, Value>, &mut Vec<RelationshipCandidate>);

const RULES: &[Rule] = &[
    rule_location,
    rule_organization,
    rule_role,
    rule_founder_reversed,
    rule_founded_in,
    rule_participants_reversed,
    rule_date_year_during,
    rule_author_reversed,
    rule_born_in,
    rule_died_in,
];

// --- Individual rules are added in later tasks. Stubs below keep the build green. ---

fn rule_location(
    source_name: &str,
    source_type: EntityType,
    obj: &Map<String, Value>,
    out: &mut Vec<RelationshipCandidate>,
) {
    let Some(target) = non_empty_string(obj.get("location")) else {
        return;
    };
    let rel = if matches!(source_type, EntityType::Event) {
        RelationshipType::HeldAt
    } else {
        RelationshipType::LocatedIn
    };
    out.push(RelationshipCandidate {
        source_name: source_name.to_string(),
        source_type,
        target_name: target,
        target_type: EntityType::Location,
        relationship_type: rel,
    });
}

/// Return `Some(s.trim().to_owned())` only when the value is a non-empty string.
fn non_empty_string(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}
fn rule_organization(
    source_name: &str,
    source_type: EntityType,
    obj: &Map<String, Value>,
    out: &mut Vec<RelationshipCandidate>,
) {
    let Some(target) = non_empty_string(obj.get("organization")) else {
        return;
    };
    out.push(RelationshipCandidate {
        source_name: source_name.to_string(),
        source_type,
        target_name: target,
        target_type: EntityType::Organization,
        relationship_type: RelationshipType::MemberOf,
    });
}

fn rule_role(
    source_name: &str,
    source_type: EntityType,
    obj: &Map<String, Value>,
    out: &mut Vec<RelationshipCandidate>,
) {
    let Some(target) = non_empty_string(obj.get("role")) else {
        return;
    };
    out.push(RelationshipCandidate {
        source_name: source_name.to_string(),
        source_type,
        target_name: target,
        target_type: EntityType::Role,
        relationship_type: RelationshipType::HeldRole,
    });
}

fn rule_founder_reversed(
    source_name: &str,
    source_type: EntityType,
    obj: &Map<String, Value>,
    out: &mut Vec<RelationshipCandidate>,
) {
    let Some(founder) = non_empty_string(obj.get("founder")) else {
        return;
    };
    // Direction inversion: person --founder_of--> org.
    out.push(RelationshipCandidate {
        source_name: founder,
        source_type: EntityType::Person,
        target_name: source_name.to_string(),
        target_type: source_type,
        relationship_type: RelationshipType::FounderOf,
    });
}
fn rule_founded_in(
    source_name: &str,
    source_type: EntityType,
    obj: &Map<String, Value>,
    out: &mut Vec<RelationshipCandidate>,
) {
    let Some(target) = non_empty_string(obj.get("founded_in")) else {
        return;
    };
    out.push(RelationshipCandidate {
        source_name: source_name.to_string(),
        source_type,
        target_name: target,
        target_type: EntityType::Location,
        relationship_type: RelationshipType::LocatedIn,
    });
}

fn rule_participants_reversed(
    source_name: &str,
    source_type: EntityType,
    obj: &Map<String, Value>,
    out: &mut Vec<RelationshipCandidate>,
) {
    let Some(arr) = obj.get("participants").and_then(|v| v.as_array()) else {
        return;
    };
    for item in arr {
        let Some(name) = item.as_str() else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        out.push(RelationshipCandidate {
            source_name: name.to_string(),
            source_type: EntityType::Person,
            target_name: source_name.to_string(),
            target_type: source_type.clone(),
            // The base ontology has no dedicated participation predicate.
            // Preserve the governed link using its generic built-in edge.
            relationship_type: RelationshipType::RelatedTo,
        });
    }
}

fn rule_date_year_during(
    source_name: &str,
    source_type: EntityType,
    obj: &Map<String, Value>,
    out: &mut Vec<RelationshipCandidate>,
) {
    let raw = obj.get("date").or_else(|| obj.get("year"));
    let Some(raw) = raw else {
        return;
    };
    let label = match raw {
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                return;
            } else {
                t.to_string()
            }
        }
        Value::Number(n) => n.to_string(),
        _ => return,
    };
    out.push(RelationshipCandidate {
        source_name: source_name.to_string(),
        source_type,
        target_name: label,
        target_type: EntityType::TimePeriod,
        relationship_type: RelationshipType::During,
    });
}

fn rule_author_reversed(
    source_name: &str,
    source_type: EntityType,
    obj: &Map<String, Value>,
    out: &mut Vec<RelationshipCandidate>,
) {
    let Some(author) = non_empty_string(obj.get("author")) else {
        return;
    };
    out.push(RelationshipCandidate {
        source_name: author,
        source_type: EntityType::Person,
        target_name: source_name.to_string(),
        target_type: source_type,
        relationship_type: RelationshipType::AuthorOf,
    });
}

fn rule_born_in(
    source_name: &str,
    source_type: EntityType,
    obj: &Map<String, Value>,
    out: &mut Vec<RelationshipCandidate>,
) {
    let Some(target) = non_empty_string(obj.get("born_in")) else {
        return;
    };
    out.push(RelationshipCandidate {
        source_name: source_name.to_string(),
        source_type,
        target_name: target,
        target_type: EntityType::Location,
        relationship_type: RelationshipType::BornIn,
    });
}

fn rule_died_in(
    source_name: &str,
    source_type: EntityType,
    obj: &Map<String, Value>,
    out: &mut Vec<RelationshipCandidate>,
) {
    let Some(target) = non_empty_string(obj.get("died_in")) else {
        return;
    };
    out.push(RelationshipCandidate {
        source_name: source_name.to_string(),
        source_type,
        target_name: target,
        target_type: EntityType::Location,
        relationship_type: RelationshipType::DiedIn,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn no_rules_fire_on_empty_object() {
        let o = obj(json!({}));
        assert!(extract("X", EntityType::Concept, &o).is_empty());
    }

    #[test]
    fn location_on_event_emits_held_at() {
        let o = obj(json!({"location": "Ahmedabad"}));
        let out = extract("Session 1937", EntityType::Event, &o);
        assert_eq!(out.len(), 1);
        let r = &out[0];
        assert_eq!(r.source_name, "Session 1937");
        assert_eq!(r.target_name, "Ahmedabad");
        assert_eq!(r.target_type, EntityType::Location);
        assert_eq!(r.relationship_type, RelationshipType::HeldAt);
    }

    #[test]
    fn location_on_non_event_emits_located_in() {
        let o = obj(json!({"location": "Mumbai"}));
        let out = extract("Acme Corp", EntityType::Organization, &o);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].relationship_type, RelationshipType::LocatedIn);
    }

    #[test]
    fn empty_or_missing_location_emits_nothing() {
        assert!(extract("X", EntityType::Event, &obj(json!({}))).is_empty());
        assert!(extract("X", EntityType::Event, &obj(json!({"location": ""}))).is_empty());
        assert!(extract("X", EntityType::Event, &obj(json!({"location": null}))).is_empty());
    }

    #[test]
    fn organization_emits_member_of() {
        let o = obj(json!({"organization": "Acme Research"}));
        let out = extract("Ada Lovelace", EntityType::Person, &o);
        assert!(out
            .iter()
            .any(|r| r.relationship_type == RelationshipType::MemberOf
                && r.target_name == "Acme Research"
                && r.target_type == EntityType::Organization));
    }

    #[test]
    fn role_emits_held_role() {
        let o = obj(json!({"role": "President"}));
        let out = extract("Ada Lovelace", EntityType::Person, &o);
        assert!(out
            .iter()
            .any(|r| r.relationship_type == RelationshipType::HeldRole
                && r.target_name == "President"
                && r.target_type == EntityType::Role));
    }

    #[test]
    fn founder_is_reversed_person_founder_of_org() {
        let o = obj(json!({"founder": "Charles Babbage"}));
        let out = extract("Acme Research", EntityType::Organization, &o);
        let r = out
            .iter()
            .find(|r| r.relationship_type == RelationshipType::FounderOf)
            .expect("founder_of relationship");
        assert_eq!(r.source_name, "Charles Babbage");
        assert_eq!(r.source_type, EntityType::Person);
        assert_eq!(r.target_name, "Acme Research");
        assert_eq!(r.target_type, EntityType::Organization);
    }

    #[test]
    fn founded_in_emits_located_in() {
        let o = obj(json!({"founded_in": "Pune"}));
        let out = extract("Acme Research", EntityType::Organization, &o);
        assert!(
            out.iter()
                .any(|r| r.relationship_type == RelationshipType::LocatedIn
                    && r.target_name == "Pune")
        );
    }

    #[test]
    fn participants_inverted_emits_one_edge_per_participant() {
        let o = obj(json!({"participants": ["Alice", "Bob"]}));
        let out = extract("Cambridge Symposium 1843", EntityType::Event, &o);
        let count = out
            .iter()
            .filter(|r| r.relationship_type == RelationshipType::RelatedTo)
            .count();
        assert_eq!(count, 2);
        let alice = out
            .iter()
            .find(|r| {
                r.source_name == "Alice" && r.relationship_type == RelationshipType::RelatedTo
            })
            .expect("alice edge");
        assert_eq!(alice.target_name, "Cambridge Symposium 1843");
        assert_eq!(alice.source_type, EntityType::Person);
    }

    #[test]
    fn date_string_emits_during_time_period() {
        let o = obj(json!({"date": "1843"}));
        let out = extract("Session", EntityType::Event, &o);
        let r = out
            .iter()
            .find(|r| r.relationship_type == RelationshipType::During)
            .expect("during edge");
        assert_eq!(r.target_name, "1843");
        assert_eq!(r.target_type, EntityType::TimePeriod);
    }

    #[test]
    fn year_integer_emits_during() {
        let o = obj(json!({"year": 1843}));
        let out = extract("Session", EntityType::Event, &o);
        let r = out
            .iter()
            .find(|r| r.relationship_type == RelationshipType::During)
            .expect("during edge");
        assert_eq!(r.target_name, "1843");
    }

    #[test]
    fn author_inverted_emits_author_of() {
        let o = obj(json!({"author": "Ada Lovelace"}));
        let out = extract("Analytical Engine Notes", EntityType::Document, &o);
        let r = out
            .iter()
            .find(|r| r.relationship_type == RelationshipType::AuthorOf)
            .expect("author_of edge");
        assert_eq!(r.source_name, "Ada Lovelace");
        assert_eq!(r.source_type, EntityType::Person);
        assert_eq!(r.target_name, "Analytical Engine Notes");
    }

    #[test]
    fn born_died_emit_location_edges() {
        let o1 = obj(json!({"born_in": "London"}));
        let out1 = extract("Ada Lovelace", EntityType::Person, &o1);
        assert!(out1
            .iter()
            .any(|r| r.relationship_type == RelationshipType::BornIn && r.target_name == "London"));

        let o2 = obj(json!({"died_in": "Oxford"}));
        let out2 = extract("Ada Lovelace", EntityType::Person, &o2);
        assert!(out2
            .iter()
            .any(|r| r.relationship_type == RelationshipType::DiedIn && r.target_name == "Oxford"));
    }
}
