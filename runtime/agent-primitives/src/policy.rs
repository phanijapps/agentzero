//! # Tool Policy Framework
//!
//! Defines tool permissions and risk levels.
//!
//! ## Overview
//!
//! Every tool has associated permissions that describe:
//! - Risk level (safe, moderate, dangerous, critical)
//! - Required capabilities
//! - Resource limits (timeout, output size)
//!
//! ## Example
//!
//! ```rust
//! use agent_primitives::policy::{ToolPermissions, ToolRiskLevel};
//!
//! let permissions = ToolPermissions {
//!     risk_level: ToolRiskLevel::Moderate,
//!     requires: vec!["network:http".into()],
//!     auto_approve: true,
//!     max_duration_secs: Some(30),
//!     max_output_bytes: Some(1024 * 1024),
//! };
//! ```

use serde::{Deserialize, Serialize};

// ============================================================================
// RISK LEVELS
// ============================================================================

/// Tool risk level classification.
///
/// Used by the orchestrator to make routing decisions and
/// by the UI to show appropriate warnings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolRiskLevel {
    /// Safe operations with no side effects.
    /// Examples: read files, search, list entities
    #[default]
    Safe,

    /// Potentially risky operations with controlled side effects.
    /// Examples: write files (sandboxed), HTTP requests (filtered)
    Moderate,

    /// Dangerous operations that can affect the system.
    /// Examples: shell execution, browser automation
    Dangerous,

    /// Critical operations requiring explicit user approval.
    /// Examples: delete operations, system configuration
    Critical,
}

impl ToolRiskLevel {
    /// Returns true if this risk level requires user confirmation.
    pub fn requires_confirmation(&self) -> bool {
        matches!(self, ToolRiskLevel::Dangerous | ToolRiskLevel::Critical)
    }

    /// Returns a human-readable description.
    pub fn description(&self) -> &'static str {
        match self {
            ToolRiskLevel::Safe => "Safe operation with no side effects",
            ToolRiskLevel::Moderate => "Operation with controlled side effects",
            ToolRiskLevel::Dangerous => "Operation that can affect the system",
            ToolRiskLevel::Critical => "Critical operation requiring approval",
        }
    }
}

// ============================================================================
// TOOL PERMISSIONS
// ============================================================================

/// Permission requirements for a tool.
///
/// Tools declare their permissions, and the orchestrator/runtime
/// checks these against the current policy context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPermissions {
    /// Risk level of this tool.
    pub risk_level: ToolRiskLevel,

    /// Required capabilities (e.g., "filesystem:read", "network:http").
    /// Empty means no special capabilities required.
    #[serde(default)]
    pub requires: Vec<String>,

    /// Whether the tool can be auto-approved without user confirmation.
    /// Only applies to Moderate risk level; Dangerous/Critical always prompt.
    #[serde(default = "default_true")]
    pub auto_approve: bool,

    /// Maximum execution time in seconds.
    /// None means use system default.
    #[serde(default)]
    pub max_duration_secs: Option<u64>,

    /// Maximum output size in bytes.
    /// None means use system default.
    #[serde(default)]
    pub max_output_bytes: Option<usize>,
}

fn default_true() -> bool {
    true
}

impl Default for ToolPermissions {
    fn default() -> Self {
        Self {
            risk_level: ToolRiskLevel::Safe,
            requires: Vec::new(),
            auto_approve: true,
            max_duration_secs: None,
            max_output_bytes: None,
        }
    }
}

impl ToolPermissions {
    /// Create permissions for a safe, read-only tool.
    pub fn safe() -> Self {
        Self::default()
    }

    /// Create permissions for a moderate-risk tool.
    pub fn moderate(requires: Vec<String>) -> Self {
        Self {
            risk_level: ToolRiskLevel::Moderate,
            requires,
            auto_approve: true,
            max_duration_secs: Some(60),
            max_output_bytes: Some(1024 * 1024), // 1 MB
        }
    }

    /// Create permissions for a dangerous tool.
    pub fn dangerous(requires: Vec<String>) -> Self {
        Self {
            risk_level: ToolRiskLevel::Dangerous,
            requires,
            auto_approve: false,
            max_duration_secs: Some(300),
            max_output_bytes: Some(10 * 1024 * 1024), // 10 MB
        }
    }

    /// Create permissions for a critical tool.
    pub fn critical(requires: Vec<String>) -> Self {
        Self {
            risk_level: ToolRiskLevel::Critical,
            requires,
            auto_approve: false,
            max_duration_secs: Some(60),
            max_output_bytes: Some(1024 * 1024), // 1 MB
        }
    }

    /// Check if this tool requires the given capability.
    pub fn requires_capability(&self, capability: &str) -> bool {
        self.requires.iter().any(|c| c == capability)
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_risk_level_defaults() {
        let risk = ToolRiskLevel::default();
        assert_eq!(risk, ToolRiskLevel::Safe);
        assert!(!risk.requires_confirmation());
    }

    #[test]
    fn test_risk_level_confirmation() {
        assert!(!ToolRiskLevel::Safe.requires_confirmation());
        assert!(!ToolRiskLevel::Moderate.requires_confirmation());
        assert!(ToolRiskLevel::Dangerous.requires_confirmation());
        assert!(ToolRiskLevel::Critical.requires_confirmation());
    }

    #[test]
    fn test_permissions_constructors() {
        let safe = ToolPermissions::safe();
        assert_eq!(safe.risk_level, ToolRiskLevel::Safe);
        assert!(safe.auto_approve);

        let moderate = ToolPermissions::moderate(vec!["network:http".into()]);
        assert_eq!(moderate.risk_level, ToolRiskLevel::Moderate);
        assert!(moderate.requires_capability("network:http"));

        let dangerous = ToolPermissions::dangerous(vec!["shell:execute".into()]);
        assert_eq!(dangerous.risk_level, ToolRiskLevel::Dangerous);
        assert!(!dangerous.auto_approve);
    }
}
