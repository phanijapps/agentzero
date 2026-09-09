//! Facade re-export: the `KnowledgeGraphStore` trait moved to the
//! `knowledge-graph` crate (`knowledge_graph::kg_trait`), next to the
//! types it is expressed in. This module keeps `zbot_stores::` imports
//! compiling during the facade retirement.

pub use knowledge_graph::kg_trait::*;
