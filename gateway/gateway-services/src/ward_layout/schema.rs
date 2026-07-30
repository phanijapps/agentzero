use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The only typed portion of `ward-conf.yaml`.
///
/// Everything except the versioned envelope remains generic data. The rule
/// interpreter validates its own finite vocabulary separately; default artifact
/// names never become Rust fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WardLayoutDocument {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    #[serde(flatten)]
    pub body: BTreeMap<String, serde_yaml::Value>,
}
