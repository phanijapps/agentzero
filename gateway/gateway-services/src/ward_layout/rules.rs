use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::WardLayoutDocument;

const MAX_RULES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeFormat {
    #[serde(rename = "raw")]
    Raw,
    #[serde(rename = "markdown")]
    Markdown,
    #[serde(rename = "okf-v0.1")]
    OkfV01,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleNode {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub kind: Option<NodeKind>,
    #[serde(rename = "match", default)]
    pub match_pattern: Option<String>,
    #[serde(default = "required_default")]
    pub required: bool,
    #[serde(default)]
    pub repeat: bool,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub format: Option<NodeFormat>,
    #[serde(default)]
    pub children: Vec<RuleNode>,
    #[serde(rename = "$ref", default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub operations: BTreeMap<String, bool>,
}

fn required_default() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompiledWardLayout {
    pub definitions: BTreeMap<String, RuleNode>,
    pub root: RuleNode,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("ward layout rules are invalid: {0}")]
pub struct RuleError(pub String);

impl CompiledWardLayout {
    pub fn compile(document: &WardLayoutDocument) -> Result<Self, RuleError> {
        let definitions = document
            .body_value("definitions")
            .cloned()
            .unwrap_or_else(|| serde_yaml::Value::Mapping(Default::default()));
        let root = document
            .body_value("root")
            .cloned()
            .ok_or_else(|| RuleError("missing `root` rule".into()))?;
        let compiled = Self {
            definitions: serde_yaml::from_value(definitions)
                .map_err(|error| RuleError(format!("invalid `definitions`: {error}")))?,
            root: serde_yaml::from_value(root)
                .map_err(|error| RuleError(format!("invalid `root`: {error}")))?,
        };
        compiled.validate()?;
        Ok(compiled)
    }

    pub fn effective<'a>(&'a self, node: &'a RuleNode) -> Result<&'a RuleNode, RuleError> {
        node.reference.as_ref().map_or(Ok(node), |name| {
            self.definitions
                .get(name)
                .ok_or_else(|| RuleError(format!("unresolved definition `{name}`")))
        })
    }

    fn validate(&self) -> Result<(), RuleError> {
        let mut count = 0;
        for (name, definition) in &self.definitions {
            validate_identifier(name, "definition")?;
            validate_node(definition, true, None, &mut count)?;
        }
        validate_node(&self.root, true, Some(0), &mut count)?;
        if count > MAX_RULES {
            return Err(RuleError("rule count exceeds 4096".into()));
        }
        validate_references(&self.root, self)?;
        for definition in self.definitions.values() {
            validate_references(definition, self)?;
        }
        validate_siblings(&self.root, self)?;
        for definition in self.definitions.values() {
            validate_siblings(definition, self)?;
        }
        Ok(())
    }
}

fn validate_identifier(value: &str, label: &str) -> Result<(), RuleError> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    valid
        .then_some(())
        .ok_or_else(|| RuleError(format!("unsafe {label} identifier `{value}`")))
}

fn validate_node(
    node: &RuleNode,
    allow_missing_match: bool,
    root_depth: Option<usize>,
    count: &mut usize,
) -> Result<(), RuleError> {
    *count += 1;
    if let Some(id) = &node.id {
        validate_identifier(id, "rule")?;
    }
    if !allow_missing_match && node.match_pattern.is_none() {
        return Err(RuleError("child rule is missing `match`".into()));
    }
    if let Some(pattern) = &node.match_pattern {
        validate_pattern(pattern, root_depth == Some(1))?;
        if pattern.contains("{ward}")
            && (pattern != "{ward}.md"
                || !node.required
                || node.repeat
                || node.reference.is_some()
                || node.kind != Some(NodeKind::File)
                || node.format != Some(NodeFormat::Markdown))
        {
            return Err(RuleError(
                "`{ward}.md` must be one required, non-repeat root Markdown file".into(),
            ));
        }
    }
    for excluded in &node.exclude {
        validate_literal_component(excluded, "exclude")?;
    }
    match &node.reference {
        Some(name) => {
            validate_identifier(name, "reference")?;
            if node.kind.is_some() || node.format.is_some() || !node.children.is_empty() {
                return Err(RuleError(format!(
                    "reference rule `{}` must omit kind, format, and children",
                    node.id.as_deref().unwrap_or("<anonymous>")
                )));
            }
        }
        None => {
            let kind = node.kind.ok_or_else(|| {
                RuleError(format!(
                    "rule `{}` requires `kind`",
                    node.id.as_deref().unwrap_or("<anonymous>")
                ))
            })?;
            if kind == NodeKind::File && !node.children.is_empty() {
                return Err(RuleError("file rules cannot have children".into()));
            }
            if kind == NodeKind::Directory && node.format.is_some() {
                return Err(RuleError("directory rules cannot declare `format`".into()));
            }
        }
    }
    for child in &node.children {
        validate_node(child, false, root_depth.map(|depth| depth + 1), count)?;
    }
    Ok(())
}

fn validate_references(node: &RuleNode, layout: &CompiledWardLayout) -> Result<(), RuleError> {
    if let Some(name) = &node.reference {
        layout.effective(node)?;
        if layout
            .definitions
            .get(name)
            .is_some_and(|definition| definition.reference.is_some())
        {
            return Err(RuleError(format!(
                "definition `{name}` cannot be a reference"
            )));
        }
    }
    for child in &node.children {
        validate_references(child, layout)?;
    }
    Ok(())
}

fn validate_siblings(node: &RuleNode, layout: &CompiledWardLayout) -> Result<(), RuleError> {
    let effective = layout.effective(node)?;
    for (index, left) in effective.children.iter().enumerate() {
        for right in &effective.children[index + 1..] {
            let left_kind = layout.effective(left)?.kind.expect("validated rule kind");
            let right_kind = layout.effective(right)?.kind.expect("validated rule kind");
            if left_kind == right_kind && static_rules_overlap(left, right) {
                return Err(RuleError(format!(
                    "sibling rules `{}` and `{}` overlap",
                    left.id.as_deref().unwrap_or("<anonymous>"),
                    right.id.as_deref().unwrap_or("<anonymous>")
                )));
            }
        }
        // Definitions are validated independently. Following a recursive
        // reference here would not advance through a filesystem path and
        // would recurse forever during configuration validation.
        if left.reference.is_none() {
            validate_siblings(left, layout)?;
        }
    }
    Ok(())
}

fn static_rules_overlap(left: &RuleNode, right: &RuleNode) -> bool {
    let left_pattern = left
        .match_pattern
        .as_deref()
        .expect("validated child match");
    let right_pattern = right
        .match_pattern
        .as_deref()
        .expect("validated child match");
    if !left_pattern.contains('*') && right.exclude.iter().any(|item| item == left_pattern) {
        return false;
    }
    if !right_pattern.contains('*') && left.exclude.iter().any(|item| item == right_pattern) {
        return false;
    }
    static_patterns_overlap(left_pattern, right_pattern)
}

fn static_patterns_overlap(left: &str, right: &str) -> bool {
    if left.contains("{name}")
        || right.contains("{name}")
        || left.contains("{ward}")
        || right.contains("{ward}")
    {
        return false;
    }
    let left = left.to_ascii_lowercase();
    let right = right.to_ascii_lowercase();
    left == right || pattern_matches(&left, &right) || pattern_matches(&right, &left)
}

pub(crate) fn resolve_pattern(
    pattern: &str,
    directory_name: &str,
    ward_id: Option<&str>,
) -> String {
    let resolved = pattern.replace("{name}", directory_name);
    match ward_id {
        Some(ward_id) => resolved.replace("{ward}", ward_id),
        None => resolved,
    }
}

pub(crate) fn pattern_matches(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    match pattern.split_once('*') {
        Some((prefix, suffix)) => name.starts_with(prefix) && name.ends_with(suffix),
        None => pattern == name,
    }
}

fn validate_pattern(pattern: &str, allow_ward: bool) -> Result<(), RuleError> {
    let ward_count = pattern.matches("{ward}").count();
    let name_count = pattern.matches("{name}").count();
    let without_placeholders = pattern.replace("{name}", "").replace("{ward}", "");
    if name_count > 1
        || ward_count > 1
        || (ward_count != 0 && !allow_ward)
        || (pattern.contains('{') && name_count == 0 && ward_count == 0)
        || without_placeholders.contains(['{', '}'])
        || pattern.matches('*').count() > 1
        || pattern.contains(['/', '\\', '?', '[', ']'])
        || pattern == "."
        || pattern == ".."
        || pattern.is_empty()
        || pattern.len() > 255
    {
        return Err(RuleError(format!("unsafe match pattern `{pattern}`")));
    }
    Ok(())
}

fn validate_literal_component(value: &str, label: &str) -> Result<(), RuleError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains(['/', '\\', '*', '?', '[', ']', '{', '}'])
        || value.len() > 255
    {
        return Err(RuleError(format!("unsafe {label} component `{value}`")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ward_layout::load_ward_layout_bytes;

    fn compile(yaml: &str) -> Result<CompiledWardLayout, RuleError> {
        let loaded = load_ward_layout_bytes(yaml.as_bytes()).unwrap();
        CompiledWardLayout::compile(&loaded.document)
    }

    // STUB: AC5
    #[test]
    fn root_ward_placeholder_compiles_as_one_required_file() {
        let yaml = "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n    - { id: canonical, match: '{ward}.md', kind: file, format: markdown }\n";
        let layout = compile(yaml).expect("root canonical placeholder must compile");
        assert_eq!(
            layout.root.children[0].match_pattern.as_deref(),
            Some("{ward}.md")
        );
    }

    #[test]
    fn ward_placeholder_rejects_every_unsupported_shape() {
        let rejected = |yaml: &str| {
            load_ward_layout_bytes(yaml.as_bytes())
                .map(|loaded| CompiledWardLayout::compile(&loaded.document).is_err())
                .unwrap_or(true)
        };
        let root = |rule: &str| {
            format!(
                "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n    {rule}\n"
            )
        };
        for rule in [
            "- { id: canonical, match: '{ward}.md', kind: directory }",
            "- { id: canonical, match: '{ward}.md', kind: file, format: raw }",
            "- { id: canonical, match: '{ward}.md', kind: file, format: markdown, required: false }",
            "- { id: canonical, match: '{ward}.md', kind: file, format: markdown, repeat: true }",
            "- { id: canonical, match: '*{ward}.md', kind: file, format: markdown }",
            "- { id: canonical, match: '{ward}{ward}.md', kind: file, format: markdown }",
        ] {
            let yaml = root(rule);
            assert!(rejected(&yaml), "{rule}");
        }

        let nested = "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n    - id: pages\n      match: pages\n      kind: directory\n      children:\n        - { id: canonical, match: '{ward}.md', kind: file, format: markdown }\n";
        assert!(rejected(nested));

        let definition = "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\ndefinitions:\n  canonical:\n    kind: file\n    match: '{ward}.md'\n    format: markdown\nroot:\n  kind: directory\n  children: []\n";
        assert!(rejected(definition));
    }

    #[test]
    fn compiles_a_template_without_any_product_roles() {
        let layout = compile(
            "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n    - { id: home, match: home.md, kind: file, format: markdown }\n",
        )
        .unwrap();
        assert_eq!(layout.root.children[0].id.as_deref(), Some("home"));
    }

    #[test]
    fn arbitrary_ids_do_not_change_rule_mechanics() {
        let yaml = "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\ndefinitions:\n  box:\n    kind: directory\n    children:\n      - { id: note, match: index.md, kind: file, format: markdown }\nroot:\n  kind: directory\n  children:\n    - { id: anything, match: '*', required: false, repeat: true, $ref: box }\n";
        let layout = compile(yaml).unwrap();
        assert_eq!(
            layout.effective(&layout.root.children[0]).unwrap().kind,
            Some(NodeKind::Directory)
        );
    }

    #[test]
    fn rejects_unknown_keys_paths_refs_and_static_collisions() {
        let base = "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n";
        assert!(compile(&format!(
            "{base}    - {{ id: x, match: x, kind: file, surprise: true }}\n"
        ))
        .is_err());
        assert!(compile(&format!(
            "{base}    - {{ id: x, match: ../x, kind: file }}\n"
        ))
        .is_err());
        assert!(compile(&format!(
            "{base}    - {{ id: x, match: x, $ref: absent }}\n"
        ))
        .is_err());
        assert!(compile(&format!("{base}    - {{ id: x, match: '*.md', kind: file }}\n    - {{ id: y, match: index.md, kind: file }}\n")).is_err());
        assert!(compile(&format!("{base}    - {{ id: x, match: log.md, kind: file }}\n    - {{ id: y, match: Log.md, kind: file }}\n")).is_err());
        assert!(compile(&format!("{base}    - {{ id: x, match: '*.MD', kind: file }}\n    - {{ id: y, match: log.md, kind: file }}\n")).is_err());
    }
}
