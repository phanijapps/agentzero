use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::rules::{pattern_matches, resolve_pattern};
use super::{CompiledWardLayout, LoadedWardLayout, NodeFormat, NodeKind, RuleError, RuleNode};

const MAX_DEPTH: usize = 32;
const MAX_ENTRIES: usize = 10_000;
const MAX_FILES: usize = 5_000;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MARKDOWN_BYTES: u64 = 1024 * 1024;
const MAX_FINDINGS: usize = 500;
const MAX_ELAPSED: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingCategory {
    Structure,
    Okf,
    Configuration,
    Budget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LintFinding {
    pub category: FindingCategory,
    pub code: String,
    pub path: String,
    pub rule_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WardLintReport {
    pub valid: bool,
    pub snapshot_digest: String,
    pub findings: Vec<LintFinding>,
    pub entries_visited: usize,
    pub files_read: usize,
    pub bytes_read: u64,
    pub terminal: bool,
}

pub fn lint_ward(root: &Path, loaded: &LoadedWardLayout) -> WardLintReport {
    let layout = match CompiledWardLayout::compile(&loaded.document) {
        Ok(layout) => layout,
        Err(error) => {
            return WardLintReport {
                valid: false,
                snapshot_digest: loaded.digest.clone(),
                findings: vec![configuration_finding(error)],
                entries_visited: 0,
                files_read: 0,
                bytes_read: 0,
                terminal: false,
            };
        }
    };

    let mut state = LintState {
        layout: &layout,
        ward_id: root
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned),
        started: Instant::now(),
        findings: Vec::new(),
        entries_visited: 0,
        files_read: 0,
        bytes_read: 0,
        terminal: false,
    };
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => state.finding(
            FindingCategory::Structure,
            "unsafe_ward_root",
            Path::new("."),
            None,
            "ward root must be a real directory",
        ),
        Ok(metadata) => {
            #[cfg(target_os = "linux")]
            match open_directory_no_follow(root) {
                Ok(directory) => {
                    use std::os::fd::AsRawFd;
                    let opened = directory.metadata();
                    if opened.as_ref().is_err()
                        || !same_directory(&metadata, opened.as_ref().expect("checked"))
                    {
                        state.finding(
                            FindingCategory::Structure,
                            "unsafe_ward_root",
                            Path::new("."),
                            None,
                            "ward root changed while it was opened",
                        );
                    } else {
                        let anchored =
                            PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
                        state.visit_directory(&anchored, Path::new("."), &layout.root, 0);
                    }
                }
                Err(_) => state.finding(
                    FindingCategory::Structure,
                    "unsafe_ward_root",
                    Path::new("."),
                    None,
                    "ward root cannot be opened without following links",
                ),
            }
            #[cfg(not(target_os = "linux"))]
            state.finding(
                FindingCategory::Configuration,
                "secure_lint_unsupported",
                Path::new("."),
                None,
                "secure ward lint is unavailable on this platform",
            );
        }
        Err(_) => state.finding(
            FindingCategory::Structure,
            "missing_ward_root",
            Path::new("."),
            None,
            "ward root is missing or unreadable",
        ),
    }
    state.finish(loaded.digest.clone())
}

struct LintState<'a> {
    layout: &'a CompiledWardLayout,
    ward_id: Option<String>,
    started: Instant,
    findings: Vec<LintFinding>,
    entries_visited: usize,
    files_read: usize,
    bytes_read: u64,
    terminal: bool,
}

impl LintState<'_> {
    fn finish(mut self, digest: String) -> WardLintReport {
        self.findings.sort_by(|left, right| {
            (&left.path, &left.code, &left.rule_id).cmp(&(&right.path, &right.code, &right.rule_id))
        });
        WardLintReport {
            valid: self.findings.is_empty(),
            snapshot_digest: digest,
            findings: self.findings,
            entries_visited: self.entries_visited,
            files_read: self.files_read,
            bytes_read: self.bytes_read,
            terminal: self.terminal,
        }
    }

    fn visit_directory(&mut self, directory: &Path, logical: &Path, node: &RuleNode, depth: usize) {
        if self.terminal || !self.check_budget(depth) {
            return;
        }
        let effective = match self.layout.effective(node) {
            Ok(rule) => rule,
            Err(error) => {
                self.finding(
                    FindingCategory::Configuration,
                    "invalid_reference",
                    logical,
                    node.id.as_deref(),
                    &error.to_string(),
                );
                return;
            }
        };
        let directory_entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(_) => {
                self.finding(
                    FindingCategory::Structure,
                    "unreadable_directory",
                    logical,
                    node.id.as_deref(),
                    "declared directory cannot be read",
                );
                return;
            }
        };
        let mut entries = Vec::new();
        for entry in directory_entries {
            if self.started.elapsed() > MAX_ELAPSED {
                self.budget("time_budget", "ward lint time budget was exhausted");
                return;
            }
            self.entries_visited = self.entries_visited.saturating_add(1);
            if self.entries_visited > MAX_ENTRIES {
                self.budget("entry_budget", "ward lint entry budget was exhausted");
                return;
            }
            if let Ok(entry) = entry {
                entries.push(entry);
            }
        }
        entries.sort_by_key(|entry| entry.file_name());

        let basename = logical
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let mut claims: BTreeMap<(String, NodeKind), String> = BTreeMap::new();
        let mut declared_names = BTreeSet::new();
        for entry in &entries {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() || (!file_type.is_file() && !file_type.is_dir()) {
                self.finding(
                    FindingCategory::Structure,
                    "unsafe_filesystem_entry",
                    &logical.join(entry.file_name()),
                    None,
                    "symlinks and special filesystem entries are not allowed in wards",
                );
            }
        }

        for child in &effective.children {
            if self.terminal {
                return;
            }
            let pattern = resolve_pattern(
                child
                    .match_pattern
                    .as_deref()
                    .expect("validated child match"),
                basename,
                (logical == Path::new("."))
                    .then_some(self.ward_id.as_deref())
                    .flatten(),
            );
            let expected = match self.layout.effective(child) {
                Ok(rule) => rule,
                Err(error) => {
                    self.finding(
                        FindingCategory::Configuration,
                        "invalid_reference",
                        logical,
                        child.id.as_deref(),
                        &error.to_string(),
                    );
                    continue;
                }
            };
            let kind = expected.kind.expect("validated rule kind");
            let mut matched = Vec::new();
            let mut wrong_kind = Vec::new();
            for entry in &entries {
                let name = entry.file_name().to_string_lossy().into_owned();
                if child.exclude.iter().any(|excluded| excluded == &name)
                    || !pattern_matches(&pattern, &name)
                {
                    continue;
                }
                let file_type = match entry.file_type() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if file_type.is_symlink() {
                    declared_names.insert(name);
                    continue;
                }
                let actual = if file_type.is_file() {
                    Some(NodeKind::File)
                } else if file_type.is_dir() {
                    Some(NodeKind::Directory)
                } else {
                    None
                };
                if actual == Some(kind) {
                    declared_names.insert(name.clone());
                    matched.push((entry.path(), name));
                } else if !pattern.contains('*') {
                    declared_names.insert(name);
                    wrong_kind.push(entry.file_name());
                }
            }
            for name in wrong_kind {
                self.finding(
                    FindingCategory::Structure,
                    "wrong_kind",
                    &logical.join(name),
                    child.id.as_deref(),
                    "declared path has the wrong filesystem kind",
                );
            }
            if matched.is_empty() && child.required {
                self.finding(
                    FindingCategory::Structure,
                    "missing_required",
                    &logical.join(&pattern),
                    child.id.as_deref(),
                    "required declared path is missing",
                );
            }
            if !child.repeat && matched.len() > 1 {
                self.finding(
                    FindingCategory::Structure,
                    "multiple_matches",
                    logical,
                    child.id.as_deref(),
                    "non-repeatable rule matched multiple paths",
                );
            }
            for (path, name) in matched {
                let logical_path = logical.join(&name);
                if let Some(first) = claims.insert(
                    (name, kind),
                    child.id.clone().unwrap_or_else(|| "<anonymous>".into()),
                ) {
                    self.finding(
                        FindingCategory::Configuration,
                        "dynamic_collision",
                        &logical_path,
                        child.id.as_deref(),
                        &format!("path is also claimed by rule `{first}`"),
                    );
                    continue;
                }
                match kind {
                    NodeKind::Directory => match open_directory_no_follow(&path) {
                        Ok(directory) => {
                            #[cfg(target_os = "linux")]
                            {
                                use std::os::fd::AsRawFd;
                                let anchored = PathBuf::from(format!(
                                    "/proc/self/fd/{}",
                                    directory.as_raw_fd()
                                ));
                                self.visit_directory(&anchored, &logical_path, expected, depth + 1);
                            }
                        }
                        Err(_) => self.finding(
                            FindingCategory::Structure,
                            "unreadable_directory",
                            &logical_path,
                            child.id.as_deref(),
                            "declared directory cannot be opened without following links",
                        ),
                    },
                    NodeKind::File => self.validate_file(&path, &logical_path, expected.format),
                }
            }
        }

        // A directory with declared children is a closed Markdown/directory
        // namespace. Leaf directories are resource areas and intentionally
        // allow arbitrary contents, including non-Markdown files.
        if !effective.children.is_empty() && !self.terminal {
            for entry in &entries {
                let name = entry.file_name().to_string_lossy().into_owned();
                if declared_names.contains(&name) {
                    continue;
                }
                let file_type = match entry.file_type() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let code = if file_type.is_dir() || file_type.is_symlink() {
                    Some("undeclared_directory")
                } else if file_type.is_file()
                    && Path::new(&name)
                        .extension()
                        .is_some_and(|value| value == "md")
                {
                    Some("undeclared_markdown")
                } else {
                    None
                };
                if let Some(code) = code {
                    self.finding(
                        FindingCategory::Structure,
                        code,
                        &logical.join(entry.file_name()),
                        None,
                        "path is not declared by the active ward template",
                    );
                }
            }
        }
    }

    fn validate_file(&mut self, path: &Path, logical: &Path, format: Option<NodeFormat>) {
        let Some(format) = format else { return };
        if format == NodeFormat::Raw {
            return;
        }
        let before = match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
            _ => return,
        };
        if before.len() > MAX_MARKDOWN_BYTES {
            self.budget(
                "markdown_file_bytes",
                "Markdown file exceeds the 1 MiB lint limit",
            );
            return;
        }
        if self.files_read >= MAX_FILES
            || self.bytes_read.saturating_add(before.len()) > MAX_TOTAL_BYTES
        {
            self.budget(
                "file_read_budget",
                "ward lint file or byte budget was exhausted",
            );
            return;
        }
        let mut bytes = Vec::with_capacity(before.len() as usize);
        let read = open_markdown_no_follow(path).and_then(|mut file| {
            let opened = file.metadata()?;
            validate_same_file(&before, &opened)?;
            Read::by_ref(&mut file)
                .take(MAX_MARKDOWN_BYTES + 1)
                .read_to_end(&mut bytes)
        });
        if read.is_err() {
            self.finding(
                FindingCategory::Structure,
                "unreadable_file",
                logical,
                None,
                "declared Markdown file cannot be read",
            );
            return;
        }
        if bytes.len() as u64 > MAX_MARKDOWN_BYTES {
            self.budget(
                "markdown_file_bytes",
                "Markdown file exceeds the 1 MiB lint limit",
            );
            return;
        }
        self.files_read += 1;
        self.bytes_read = self.bytes_read.saturating_add(bytes.len() as u64);
        let Ok(markdown) = std::str::from_utf8(&bytes) else {
            self.finding(
                FindingCategory::Structure,
                "invalid_markdown_utf8",
                logical,
                None,
                "Markdown must be UTF-8",
            );
            return;
        };
        if format == NodeFormat::OkfV01 {
            self.validate_okf(logical, markdown);
        }
    }

    fn validate_okf(&mut self, path: &Path, markdown: &str) {
        let Some(frontmatter) = frontmatter(markdown) else {
            self.finding(
                FindingCategory::Okf,
                "missing_frontmatter",
                path,
                None,
                "OKF Markdown requires YAML frontmatter",
            );
            return;
        };
        match serde_yaml::from_str::<serde_yaml::Value>(frontmatter) {
            Ok(value) => {
                let kind = value
                    .as_mapping()
                    .and_then(|map| map.get(serde_yaml::Value::String("type".into())))
                    .and_then(serde_yaml::Value::as_str)
                    .map(str::trim);
                if kind.is_none_or(str::is_empty) {
                    self.finding(
                        FindingCategory::Okf,
                        "missing_type",
                        path,
                        None,
                        "OKF frontmatter `type` must be non-empty",
                    );
                }
            }
            Err(_) => self.finding(
                FindingCategory::Okf,
                "invalid_frontmatter",
                path,
                None,
                "OKF frontmatter is invalid YAML",
            ),
        }
    }

    fn check_budget(&mut self, depth: usize) -> bool {
        if depth > MAX_DEPTH {
            self.budget("depth_budget", "ward lint depth budget was exhausted");
        } else if self.entries_visited > MAX_ENTRIES {
            self.budget("entry_budget", "ward lint entry budget was exhausted");
        } else if self.started.elapsed() > MAX_ELAPSED {
            self.budget("time_budget", "ward lint time budget was exhausted");
        }
        !self.terminal
    }

    fn budget(&mut self, code: &str, message: &str) {
        if self.terminal {
            return;
        }
        self.terminal = true;
        self.findings.clear();
        self.findings.push(LintFinding {
            category: FindingCategory::Budget,
            code: code.into(),
            path: ".".into(),
            rule_id: None,
            message: message.into(),
        });
    }

    fn finding(
        &mut self,
        category: FindingCategory,
        code: &str,
        path: &Path,
        rule_id: Option<&str>,
        message: &str,
    ) {
        if self.terminal || self.findings.len() >= MAX_FINDINGS {
            if self.findings.len() >= MAX_FINDINGS {
                self.budget("finding_budget", "ward lint finding budget was exhausted");
            }
            return;
        }
        self.findings.push(LintFinding {
            category,
            code: code.into(),
            path: relative_string(path),
            rule_id: rule_id.map(str::to_owned),
            message: message.into(),
        });
    }
}

fn configuration_finding(error: RuleError) -> LintFinding {
    LintFinding {
        category: FindingCategory::Configuration,
        code: "invalid_rules".into(),
        path: "ward-conf.yaml".into(),
        rule_id: None,
        message: error.to_string(),
    }
}

fn relative_string(path: &Path) -> String {
    let value = path
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    if value.is_empty() {
        ".".into()
    } else {
        value
    }
}

fn frontmatter(markdown: &str) -> Option<&str> {
    let rest = markdown
        .strip_prefix("---\n")
        .or_else(|| markdown.strip_prefix("---\r\n"))?;
    let end = rest.find("\n---\n").or_else(|| rest.find("\r\n---\r\n"))?;
    Some(&rest[..end])
}

fn open_markdown_no_follow(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Linux O_NOFOLLOW | O_NONBLOCK. The latter prevents a raced FIFO
        // from blocking the synchronous post-write lint hook.
        options.custom_flags(0x20_000 | 0x800);
    }
    options.open(path)
}

#[cfg(target_os = "linux")]
fn open_directory_no_follow(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(0o200000 | 0o400000 | 0o2000000)
        .open(path)
}

#[cfg(not(target_os = "linux"))]
fn open_directory_no_follow(_path: &Path) -> std::io::Result<fs::File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "secure directory traversal is unavailable",
    ))
}

fn validate_same_file(before: &fs::Metadata, opened: &fs::Metadata) -> std::io::Result<()> {
    if !opened.is_file() || opened.file_type().is_symlink() {
        return Err(std::io::Error::other(
            "Markdown target is not a regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != opened.dev() || before.ino() != opened.ino() || opened.nlink() != 1 {
            return Err(std::io::Error::other(
                "Markdown target changed while it was opened",
            ));
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn same_directory(before: &fs::Metadata, opened: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    !before.file_type().is_symlink()
        && before.is_dir()
        && opened.is_dir()
        && before.dev() == opened.dev()
        && before.ino() == opened.ino()
}

#[cfg(test)]
mod tests {
    use super::super::loader::DEFAULT_WARD_LAYOUT;
    use super::*;
    use crate::ward_layout::load_ward_layout_bytes;
    use tempfile::tempdir;

    fn loaded(yaml: &str) -> LoadedWardLayout {
        load_ward_layout_bytes(yaml.as_bytes()).unwrap()
    }

    #[test]
    fn default_template_compiles_without_product_role_code() {
        CompiledWardLayout::compile(&loaded(DEFAULT_WARD_LAYOUT).document).unwrap();
    }

    #[test]
    fn lints_an_incompatible_minimal_template_and_allows_undeclared_binary_files() {
        let yaml = "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n    - { id: landing, match: home.md, kind: file, format: markdown }\n    - { id: assets, match: assets, kind: directory }\n";
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("home.md"), "# Home\n").unwrap();
        fs::create_dir(dir.path().join("assets")).unwrap();
        fs::write(dir.path().join("assets/blob.bin"), [0, 159, 146, 150]).unwrap();
        let report = lint_ward(dir.path(), &loaded(yaml));
        assert!(report.valid, "{:?}", report.findings);
        assert_eq!(report.files_read, 1);
    }

    #[test]
    fn declared_namespaces_reject_undeclared_markdown_and_directories() {
        let yaml = "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n    - { id: landing, match: home.md, kind: file, format: markdown }\n";
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("home.md"), "# Home\n").unwrap();
        fs::write(dir.path().join("rogue.md"), "# Rogue\n").unwrap();
        fs::create_dir(dir.path().join("rogue-dir")).unwrap();
        let report = lint_ward(dir.path(), &loaded(yaml));
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "undeclared_markdown"));
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "undeclared_directory"));
    }

    #[test]
    fn reports_missing_wrong_kind_okf_and_dynamic_collisions() {
        let yaml = "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\ndefinitions:\n  item:\n    kind: directory\n    children:\n      - { id: companion, match: '{name}.md', kind: file, format: okf-v0.1 }\n      - { id: fixed, match: spec.md, kind: file, required: false, format: markdown }\nroot:\n  kind: directory\n  children:\n    - { id: required, match: index.md, kind: file, format: markdown }\n    - { id: items, match: '*', required: false, repeat: true, $ref: item }\n";
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("index.md")).unwrap();
        fs::create_dir(dir.path().join("spec")).unwrap();
        fs::write(dir.path().join("spec/spec.md"), "not okf").unwrap();
        let report = lint_ward(dir.path(), &loaded(yaml));
        let codes = report
            .findings
            .iter()
            .map(|finding| finding.code.as_str())
            .collect::<Vec<_>>();
        assert!(codes.contains(&"wrong_kind"));
        assert!(codes.contains(&"missing_frontmatter"));
        assert!(codes.contains(&"dynamic_collision"));
    }

    #[cfg(unix)]
    #[test]
    fn declared_symlinks_are_never_followed() {
        use std::os::unix::fs::symlink;
        let yaml = "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n    - { id: home, match: home.md, kind: file, format: markdown }\n";
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("secret"), "secret").unwrap();
        symlink(outside.path().join("secret"), dir.path().join("home.md")).unwrap();
        let report = lint_ward(dir.path(), &loaded(yaml));
        assert_eq!(report.files_read, 0);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "unsafe_filesystem_entry"));
    }
}
