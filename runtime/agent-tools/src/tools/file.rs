// ============================================================================
// FILE TOOLS
// Read tool
// ============================================================================

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use agent_primitives::{FileSystemContext, NoFileSystemContext};
use agent_primitives::{Result, Tool, ToolContext, ToolPermissions};

/// Tool for reading file contents.
pub struct ReadTool {
    /// File system context for ward-relative fallback reads.
    fs: Arc<dyn FileSystemContext>,
}

impl ReadTool {
    /// Create a new read tool with file system context.
    #[must_use]
    pub fn new(fs: Arc<dyn FileSystemContext>) -> Self {
        Self { fs }
    }
}

impl Default for ReadTool {
    fn default() -> Self {
        Self::new(Arc::new(NoFileSystemContext))
    }
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read UTF-8 or BOM-marked UTF-16 text file contents. Supports optional offset and limit for line-by-line reading. Relative paths fall back to the current ward when direct reads fail."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "Starting line number (0-indexed)",
                    "default": 0
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read"
                }
            },
            "required": ["path"]
        }))
    }

    fn permissions(&self) -> ToolPermissions {
        ToolPermissions::safe()
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let path = args.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            agent_primitives::AgentError::Tool("Missing 'path' parameter".to_string())
        })?;

        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = args.get("limit").and_then(|v| v.as_u64());

        tracing::debug!(
            file = %file!(),
            line = %line!(),
            "Reading file: {} (offset: {}, limit: {:?})",
            path, offset, limit
        );

        let content = read_with_ward_fallback(&self.fs, &ctx, path)?;

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let start = offset.min(total_lines);
        let end = if let Some(lim) = limit {
            (start + lim as usize).min(total_lines)
        } else {
            total_lines
        };

        let selected_lines = lines[start..end].join("\n");

        Ok(json!({
            "content": selected_lines,
            "total_lines": total_lines,
            "lines_read": end - start,
            "offset": start }))
    }
}

fn read_with_ward_fallback(
    fs: &Arc<dyn FileSystemContext>,
    ctx: &Arc<dyn ToolContext>,
    path: &str,
) -> Result<String> {
    match read_text_file(path) {
        Ok(content) => Ok(content),
        Err(direct_err) => {
            if !can_try_ward_relative(path) {
                return Err(agent_primitives::AgentError::Tool(format!(
                    "Failed to read file: {}",
                    direct_err
                )));
            }

            let ward_id = ctx
                .get_state("ward_id")
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "scratch".to_string());
            let Some(ward_dir) = fs.ward_dir(&ward_id) else {
                return Err(agent_primitives::AgentError::Tool(format!(
                    "Failed to read file: {}",
                    direct_err
                )));
            };

            let ward_relative = path.trim_start_matches("./");
            let ward_path = ward_dir.join(ward_relative);
            read_text_file(&ward_path).map_err(|ward_err| {
                agent_primitives::AgentError::Tool(format!(
                    "Failed to read file: {}; ward fallback {} failed: {}",
                    direct_err,
                    ward_path.display(),
                    ward_err
                ))
            })
        }
    }
}

/// Decode normal UTF-8 text and the BOM-marked UTF-16 files commonly
/// produced by voice-transcription exports. Other binary or unknown-encoding
/// files remain a clear read error rather than being lossy-decoded.
fn read_text_file(path: impl AsRef<Path>) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    if let Ok(text) = String::from_utf8(bytes.clone()) {
        return Ok(text);
    }

    let (encoding, body) = match bytes.as_slice() {
        [0xFE, 0xFF, rest @ ..] => ("UTF-16BE", rest),
        [0xFF, 0xFE, rest @ ..] => ("UTF-16LE", rest),
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "file is neither valid UTF-8 nor BOM-marked UTF-16",
            ));
        }
    };
    if body.len() % 2 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{encoding} file has an odd number of data bytes"),
        ));
    }

    let units: Vec<u16> = body
        .chunks_exact(2)
        .map(|pair| match encoding {
            "UTF-16BE" => u16::from_be_bytes([pair[0], pair[1]]),
            "UTF-16LE" => u16::from_le_bytes([pair[0], pair[1]]),
            _ => unreachable!("encoding is selected above"),
        })
        .collect();
    String::from_utf16(&units).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid {encoding} text: {error}"),
        )
    })
}

fn can_try_ward_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with("~/")
        && !path.starts_with('\\')
        && !Path::new(path).is_absolute()
        && !Path::new(path)
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::LazyLock;

    use agent_primitives::types::Content;
    use agent_primitives::{CallbackContext, EventActions, ReadonlyContext};
    use serde_json::json;

    struct TestFs {
        wards_root: PathBuf,
    }

    impl FileSystemContext for TestFs {
        fn conversation_dir(&self, _conversation_id: &str) -> Option<PathBuf> {
            None
        }

        fn outputs_dir(&self) -> Option<PathBuf> {
            None
        }

        fn skills_dir(&self) -> Option<PathBuf> {
            None
        }

        fn agents_dir(&self) -> Option<PathBuf> {
            None
        }

        fn python_executable(&self) -> Option<PathBuf> {
            None
        }

        fn wards_root_dir(&self) -> Option<PathBuf> {
            Some(self.wards_root.clone())
        }
    }

    struct TestCtx {
        state: HashMap<String, Value>,
    }

    impl TestCtx {
        fn with_ward(ward_id: &str) -> Self {
            Self {
                state: HashMap::from([("ward_id".to_string(), json!(ward_id))]),
            }
        }

        fn empty() -> Self {
            Self {
                state: HashMap::new(),
            }
        }
    }

    impl ReadonlyContext for TestCtx {
        fn invocation_id(&self) -> &str {
            "test-invocation"
        }

        fn agent_name(&self) -> &str {
            "test-agent"
        }

        fn user_id(&self) -> &str {
            "test-user"
        }

        fn app_name(&self) -> &str {
            "test-app"
        }

        fn session_id(&self) -> &str {
            "test-session"
        }

        fn branch(&self) -> &str {
            "test"
        }

        fn user_content(&self) -> &Content {
            static CONTENT: LazyLock<Content> = LazyLock::new(|| Content {
                role: "user".to_string(),
                parts: vec![],
            });
            &CONTENT
        }
    }

    impl CallbackContext for TestCtx {
        fn get_state(&self, key: &str) -> Option<Value> {
            self.state.get(key).cloned()
        }

        fn set_state(&self, _key: String, _value: Value) {}
    }

    impl ToolContext for TestCtx {
        fn function_call_id(&self) -> String {
            "test-call".to_string()
        }

        fn actions(&self) -> EventActions {
            EventActions::default()
        }

        fn set_actions(&self, _actions: EventActions) {}
    }

    #[tokio::test]
    async fn read_falls_back_to_active_ward_for_relative_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let wards_root = temp.path().join("wards");
        let target = wards_root
            .join("financial-analysis")
            .join("xom-valuation")
            .join("code");
        std::fs::create_dir_all(&target).expect("create ward target");
        std::fs::write(
            target.join("fetch_catalysts_risk.py"),
            "line one\nline two\nline three\n",
        )
        .expect("write fixture");

        let tool = ReadTool::new(Arc::new(TestFs { wards_root }));
        let result = tool
            .execute(
                Arc::new(TestCtx::with_ward("financial-analysis")),
                json!({
                    "path": "xom-valuation/code/fetch_catalysts_risk.py",
                    "offset": 1,
                    "limit": 1
                }),
            )
            .await
            .expect("read should fall back to ward");

        assert_eq!(result["content"], "line two");
        assert_eq!(result["total_lines"], 3);
        assert_eq!(result["lines_read"], 1);
        assert_eq!(result["offset"], 1);
    }

    #[tokio::test]
    async fn read_preserves_absolute_path_behavior() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("absolute.txt");
        std::fs::write(&path, "absolute content").expect("write fixture");

        let tool = ReadTool::default();
        let result = tool
            .execute(
                Arc::new(TestCtx::empty()),
                json!({ "path": path.to_string_lossy() }),
            )
            .await
            .expect("absolute path should read directly");

        assert_eq!(result["content"], "absolute content");
    }

    #[test]
    fn reads_bom_marked_utf16be_text() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("transcript.txt");
        std::fs::write(
            &path,
            [0xFE, 0xFF, 0x00, b'H', 0x00, b'i', 0x00, b'!', 0x00, b'\n'],
        )
        .expect("write UTF-16BE fixture");

        assert_eq!(read_text_file(path).expect("decode UTF-16BE"), "Hi!\n");
    }
}
