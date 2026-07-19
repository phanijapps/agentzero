//! Belief-contradiction persistence trait.
//!
//! The trait itself lives in the dependency-light `zbot-stores-traits`
//! crate so that consumers deep in the dep graph (notably `agent-tools`)
//! can pull in the trait without inheriting `zbot-stores`' transitive
//! dependency on `knowledge-graph`.
//!
//! This module re-exports the public surface unchanged so callers can
//! import `zbot_stores::BeliefContradictionStore` if they already depend
//! on `zbot-stores`.

pub use zbot_stores_traits::{
    BeliefContradiction, BeliefContradictionStore, ContradictionType, Resolution,
};
