use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Closed identifier for a locally bundled Ward Layout archetype.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum WardArchetypeId {
    #[default]
    Generic,
    Coding,
    Documentation,
    Journal,
    Ebook,
    Research,
    News,
}

impl WardArchetypeId {
    pub const ALL: [Self; 7] = [
        Self::Generic,
        Self::Coding,
        Self::Documentation,
        Self::Journal,
        Self::Ebook,
        Self::Research,
        Self::News,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::Coding => "coding",
            Self::Documentation => "documentation",
            Self::Journal => "journal",
            Self::Ebook => "ebook",
            Self::Research => "research",
            Self::News => "news",
        }
    }
}

impl fmt::Display for WardArchetypeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("unknown ward archetype")]
pub struct UnknownWardArchetype;

impl FromStr for WardArchetypeId {
    type Err = UnknownWardArchetype;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "generic" => Ok(Self::Generic),
            "coding" => Ok(Self::Coding),
            "documentation" => Ok(Self::Documentation),
            "journal" => Ok(Self::Journal),
            "ebook" => Ok(Self::Ebook),
            "research" => Ok(Self::Research),
            "news" => Ok(Self::News),
            _ => Err(UnknownWardArchetype),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // STUB: AC1
    #[test]
    fn ward_archetype_id_is_closed() {
        for expected in WardArchetypeId::ALL {
            let encoded = serde_json::to_string(&expected).unwrap();
            assert_eq!(encoded, format!("\"{}\"", expected.as_str()));
            assert_eq!(expected.as_str().parse(), Ok(expected));
        }

        for invalid in [
            "",
            "Generic",
            "unknown",
            "../coding",
            "coding/news",
            "/coding",
            "coding\nlayout: {}",
        ] {
            assert_eq!(
                invalid.parse::<WardArchetypeId>(),
                Err(UnknownWardArchetype)
            );
        }
    }
}
