//! Runtime session-title derivation.
//!
//! This keeps session naming out of the default model-visible tool path while
//! preserving the same persisted `sessions.title` and websocket event contract.

#[derive(Debug, Clone, Copy, Default)]
pub struct SessionTitleService;

#[derive(Debug, Clone, Copy, Default)]
pub struct SessionTitleInputs<'a> {
    pub explicit_title: Option<&'a str>,
    pub intent_title_hint: Option<&'a str>,
    pub first_user_message: Option<&'a str>,
    pub first_meaningful_activity: Option<&'a str>,
}

impl SessionTitleService {
    pub fn derive_title(inputs: SessionTitleInputs<'_>) -> Option<String> {
        [
            inputs.explicit_title,
            inputs.intent_title_hint,
            inputs.first_user_message,
            inputs.first_meaningful_activity,
        ]
        .into_iter()
        .flatten()
        .find_map(sanitize_title)
    }
}

fn sanitize_title(candidate: &str) -> Option<String> {
    let stripped = candidate
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("```"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut words = Vec::new();
    for word in stripped
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|ch: char| {
                ch == '#'
                    || ch == '*'
                    || ch == '`'
                    || ch == '"'
                    || ch == '\''
                    || ch == ':'
                    || ch == ';'
                    || ch == ','
                    || ch == '.'
                    || ch == '!'
                    || ch == '?'
            })
        })
        .filter(|word| !word.is_empty())
    {
        words.push(word);
        if words.len() == 8 {
            break;
        }
    }

    let title = words.join(" ");
    if title.is_empty() {
        return None;
    }

    let mut truncated = title.chars().take(80).collect::<String>();
    while truncated.ends_with(|ch: char| ch.is_ascii_punctuation()) {
        truncated.pop();
    }
    let title = truncated.trim().to_string();
    (!title.is_empty()).then_some(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_title_by_priority() {
        let title = SessionTitleService::derive_title(SessionTitleInputs {
            explicit_title: Some("Explicit title"),
            intent_title_hint: Some("Intent title"),
            first_user_message: Some("First message"),
            first_meaningful_activity: Some("Plan result"),
        });

        assert_eq!(title.as_deref(), Some("Explicit title"));
    }

    #[test]
    fn falls_back_to_intent_then_message_then_activity() {
        let title = SessionTitleService::derive_title(SessionTitleInputs {
            explicit_title: None,
            intent_title_hint: Some("peer valuation of WMT"),
            first_user_message: Some("Analyze WMT against peers"),
            first_meaningful_activity: Some("Plan result"),
        });
        assert_eq!(title.as_deref(), Some("peer valuation of WMT"));

        let title = SessionTitleService::derive_title(SessionTitleInputs {
            explicit_title: None,
            intent_title_hint: None,
            first_user_message: Some("Analyze WMT against peers using AAPL comps"),
            first_meaningful_activity: Some("Plan result"),
        });
        assert_eq!(
            title.as_deref(),
            Some("Analyze WMT against peers using AAPL comps")
        );
    }

    #[test]
    fn sanitizes_and_bounds_titles() {
        let title = SessionTitleService::derive_title(SessionTitleInputs {
            explicit_title: Some("```md\n# Analyze AAPL, WMT, and peers with a very long instruction tail that should not all fit\n```"),
            intent_title_hint: None,
            first_user_message: None,
            first_meaningful_activity: None });

        assert_eq!(
            title.as_deref(),
            Some("Analyze AAPL WMT and peers with a very")
        );
    }
}
