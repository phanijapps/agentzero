// ============================================================================
// WARD WORKING-DIRECTORY RESOLVER
// Shared helper used by `write_file` and `edit_file` to scope file operations
// to the active ward.
// ============================================================================

use std::path::PathBuf;
use std::sync::Arc;

use agent_primitives::{FileSystemContext, ToolContext};

/// Resolve the working directory for file operations.
///
/// Uses the active ward directory if available, otherwise falls back to
/// `~/Documents/zbot/wards/<ward_id>`. The `ward_id` is read from tool
/// context (`ctx.get_state("ward_id")`); absent → `"scratch"`. Final
/// fallback is the process CWD.
pub fn resolve_ward_cwd(fs: &Arc<dyn FileSystemContext>, ctx: &Arc<dyn ToolContext>) -> PathBuf {
    let ward_id = ctx
        .get_state("ward_id")
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "scratch".to_string());

    if let Some(dir) = fs.ward_dir(&ward_id) {
        return dir;
    }

    if let Some(doc_dir) = dirs::document_dir().or_else(dirs::home_dir) {
        return doc_dir.join("zbot").join("wards").join(&ward_id);
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
