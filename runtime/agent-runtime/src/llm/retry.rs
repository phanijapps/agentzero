// ============================================================================
// LLM RETRY LOGIC
// Exponential backoff with jitter for transient LLM API failures
// ============================================================================

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use super::client::{ChatResponse, LlmClient, LlmError, StreamCallback};
use crate::types::ChatMessage;

/// Configuration for retry behavior on LLM API calls.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts (total calls = `max_retries` + 1).
    pub max_retries: u32,

    /// Base delay before first retry. Subsequent delays grow exponentially.
    pub base_delay: Duration,

    /// Maximum delay between retries (caps exponential growth).
    pub max_delay: Duration,

    /// Whether to retry on 429 (rate limited) responses.
    pub retry_on_rate_limit: bool,

    /// Whether to retry on 5xx (server error) responses.
    pub retry_on_server_error: bool,

    /// Whether to retry on network/timeout errors.
    pub retry_on_transport: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            retry_on_rate_limit: true,
            retry_on_server_error: true,
            retry_on_transport: true,
        }
    }
}

impl RetryPolicy {
    /// Compute the delay for a given attempt (0-indexed).
    /// Uses exponential backoff with jitter: 2^attempt * base * random(0.9..1.1)
    fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let exp = 2u64.saturating_pow(attempt);
        let base_ms = self.base_delay.as_millis() as u64;
        let delay_ms = exp.saturating_mul(base_ms);

        // Apply jitter: ±10%
        let jitter_factor = 0.9 + (pseudo_random() * 0.2);
        let jittered_ms = (delay_ms as f64 * jitter_factor) as u64;

        // Cap at max_delay
        let max_ms = self.max_delay.as_millis() as u64;
        Duration::from_millis(jittered_ms.min(max_ms))
    }

    /// Check if a given error should be retried according to this policy.
    fn should_retry(&self, error: &LlmError) -> bool {
        match error {
            LlmError::RateLimited => self.retry_on_rate_limit,
            LlmError::HttpError(_) => self.retry_on_transport,
            LlmError::ApiError(msg) => {
                // Retry on 5xx server errors
                if self.retry_on_server_error && msg.starts_with("(5") {
                    return true;
                }
                // Retry on 429 rate limit errors from API
                if self.retry_on_rate_limit && msg.contains("429") {
                    return true;
                }
                // Z.AI/GLM returns 500 with code 1234 for rate limits (disguised as "network error")
                // Also catch code 1302 (explicit rate limit) and 1303 (frequency limit)
                if self.retry_on_rate_limit
                    && (msg.contains("1234") || msg.contains("1302") || msg.contains("1303"))
                {
                    return true;
                }
                false
            }
            // Don't retry on parse errors, auth failures, model not found, etc.
            _ => false,
        }
    }
}

/// Simple pseudo-random number generator for jitter (avoids pulling in `rand` crate).
/// Returns a value in [0.0, 1.0).
fn pseudo_random() -> f64 {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    f64::from(nanos % 1000) / 1000.0
}

/// An LLM client wrapper that adds retry logic with exponential backoff.
///
/// Wraps any `LlmClient` and transparently retries transient failures.
pub struct RetryingLlmClient {
    inner: Arc<dyn LlmClient>,
    policy: RetryPolicy,
}

impl RetryingLlmClient {
    /// Create a new retrying client wrapping the given inner client.
    pub fn new(inner: Arc<dyn LlmClient>, policy: RetryPolicy) -> Self {
        Self { inner, policy }
    }
}

#[async_trait]
impl LlmClient for RetryingLlmClient {
    fn model(&self) -> &str {
        self.inner.model()
    }

    fn provider(&self) -> &str {
        self.inner.provider()
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
    ) -> Result<ChatResponse, LlmError> {
        let mut last_error = None;

        for attempt in 0..=self.policy.max_retries {
            if attempt > 0 {
                let delay = self.policy.delay_for_attempt(attempt - 1);
                tracing::warn!(
                    "Retrying chat() (attempt {}/{}) after {:?}",
                    attempt + 1,
                    self.policy.max_retries + 1,
                    delay,
                );
                tokio::time::sleep(delay).await;
            }

            match self.inner.chat(messages.clone(), tools.clone()).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    if self.policy.should_retry(&e) && attempt < self.policy.max_retries {
                        tracing::warn!("chat() failed (attempt {}): {}", attempt + 1, e);
                        last_error = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(last_error.unwrap_or(LlmError::ApiError("Max retries exceeded".to_string())))
    }

    async fn chat_with_schema(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        output_schema: Option<Value>,
    ) -> Result<ChatResponse, LlmError> {
        let mut last_error = None;

        for attempt in 0..=self.policy.max_retries {
            if attempt > 0 {
                let delay = self.policy.delay_for_attempt(attempt - 1);
                tracing::warn!(
                    "Retrying chat_with_schema() (attempt {}/{}) after {:?}",
                    attempt + 1,
                    self.policy.max_retries + 1,
                    delay,
                );
                tokio::time::sleep(delay).await;
            }

            match self
                .inner
                .chat_with_schema(messages.clone(), tools.clone(), output_schema.clone())
                .await
            {
                Ok(response) => return Ok(response),
                Err(e) => {
                    if self.policy.should_retry(&e) && attempt < self.policy.max_retries {
                        tracing::warn!(
                            "chat_with_schema() failed (attempt {}): {}",
                            attempt + 1,
                            e
                        );
                        last_error = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(last_error.unwrap_or(LlmError::ApiError("Max retries exceeded".to_string())))
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        callback: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        // Streaming cannot be retried once the callback has been invoked with
        // partial data (tokens already emitted to the user). The callback is also
        // a non-Clone Box<dyn Fn>, so we can't re-use it across attempts.
        //
        // Retry happens at the executor level: if the stream task fails, the
        // executor's chat() retry path handles it. Here we just pass through.
        //
        // Note: The executor uses chat_stream via an mpsc channel. If the stream
        // fails, the channel closes and the executor sees the error. Future
        // improvement: accept a callback factory for pre-connection retries.
        self.inner.chat_stream(messages, tools, callback).await
    }

    fn supports_tools(&self) -> bool {
        self.inner.supports_tools()
    }

    fn supports_reasoning(&self) -> bool {
        self.inner.supports_reasoning()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_policy_defaults() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.max_retries, 3);
        assert!(policy.retry_on_rate_limit);
        assert!(policy.retry_on_server_error);
        assert!(policy.retry_on_transport);
    }

    #[test]
    fn test_delay_exponential_growth() {
        let policy = RetryPolicy {
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(60),
            ..Default::default()
        };

        let d0 = policy.delay_for_attempt(0);
        let d1 = policy.delay_for_attempt(1);
        let d2 = policy.delay_for_attempt(2);

        // d0 ≈ 100ms, d1 ≈ 200ms, d2 ≈ 400ms (with jitter ±10%)
        assert!(d0.as_millis() >= 90 && d0.as_millis() <= 110);
        assert!(d1.as_millis() >= 180 && d1.as_millis() <= 220);
        assert!(d2.as_millis() >= 360 && d2.as_millis() <= 440);
    }

    #[test]
    fn test_delay_capped_at_max() {
        let policy = RetryPolicy {
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(5),
            ..Default::default()
        };

        // Attempt 10 would be 2^10 * 1s = 1024s, should be capped at 5s
        let d = policy.delay_for_attempt(10);
        assert!(d <= Duration::from_millis(5500)); // 5s + 10% jitter
    }

    #[test]
    fn test_should_retry_logic() {
        let policy = RetryPolicy::default();

        assert!(policy.should_retry(&LlmError::RateLimited));
        assert!(policy.should_retry(&LlmError::ApiError(
            "(500): Internal Server Error".to_string()
        )));
        assert!(policy.should_retry(&LlmError::ApiError("(502): Bad Gateway".to_string())));
        assert!(policy.should_retry(&LlmError::ApiError("(429): Too Many Requests".to_string())));
        assert!(!policy.should_retry(&LlmError::AuthenticationFailed));
        assert!(!policy.should_retry(&LlmError::ParseError("bad json".to_string())));
        assert!(!policy.should_retry(&LlmError::ModelNotFound("gpt-5".to_string())));
    }

    #[test]
    fn should_retry_glm_disguised_codes() {
        let policy = RetryPolicy::default();
        assert!(policy.should_retry(&LlmError::ApiError("network error code 1234".to_string())));
        assert!(policy.should_retry(&LlmError::ApiError("rate limit 1302 reached".to_string())));
        assert!(policy.should_retry(&LlmError::ApiError("frequency 1303 hit".to_string())));
    }

    #[test]
    fn should_retry_disabled_flags() {
        let policy = RetryPolicy {
            retry_on_rate_limit: false,
            retry_on_server_error: false,
            retry_on_transport: false,
            ..RetryPolicy::default()
        };
        assert!(!policy.should_retry(&LlmError::RateLimited));
        assert!(!policy.should_retry(&LlmError::ApiError("(500)".to_string())));
        assert!(!policy.should_retry(&LlmError::ApiError("429 too many".to_string())));
    }

    #[test]
    fn pseudo_random_in_unit_interval() {
        for _ in 0..32 {
            let v = pseudo_random();
            assert!((0.0..1.0).contains(&v));
        }
    }

    // ----------------------------------------------------------------------
    // chat() retry behaviour with a programmable mock client
    // ----------------------------------------------------------------------

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    struct ProgrammableClient {
        // Outcomes (in order). Each call dequeues one. After exhausted, returns Ok.
        outcomes: Mutex<Vec<Result<ChatResponse, LlmError>>>,
        calls: Arc<AtomicUsize>,
    }

    impl ProgrammableClient {
        fn new(outcomes: Vec<Result<ChatResponse, LlmError>>) -> (Arc<Self>, Arc<AtomicUsize>) {
            let calls = Arc::new(AtomicUsize::new(0));
            (
                Arc::new(Self {
                    outcomes: Mutex::new(outcomes),
                    calls: Arc::clone(&calls),
                }),
                calls,
            )
        }
    }

    #[async_trait]
    impl LlmClient for ProgrammableClient {
        fn model(&self) -> &str {
            "test-model"
        }
        fn provider(&self) -> &str {
            "test-provider"
        }
        async fn chat(
            &self,
            _messages: Vec<ChatMessage>,
            _tools: Option<Value>,
        ) -> Result<ChatResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut q = self.outcomes.lock().unwrap();
            if q.is_empty() {
                Ok(ChatResponse {
                    content: "ok".to_string(),
                    tool_calls: None,
                    reasoning: None,
                    usage: None,
                })
            } else {
                q.remove(0)
            }
        }
        async fn chat_stream(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
            _cb: StreamCallback,
        ) -> Result<ChatResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ChatResponse {
                content: "stream-ok".to_string(),
                tool_calls: None,
                reasoning: None,
                usage: None,
            })
        }
        fn supports_tools(&self) -> bool {
            true
        }
        fn supports_reasoning(&self) -> bool {
            true
        }
    }

    fn fast_policy(max_retries: u32) -> RetryPolicy {
        RetryPolicy {
            max_retries,
            base_delay: Duration::from_millis(0),
            max_delay: Duration::from_millis(0),
            ..RetryPolicy::default()
        }
    }

    #[tokio::test]
    async fn chat_succeeds_first_try_no_retry() {
        let (client, calls) = ProgrammableClient::new(vec![Ok(ChatResponse {
            content: "first-call".to_string(),
            tool_calls: None,
            reasoning: None,
            usage: None,
        })]);
        let retrying = RetryingLlmClient::new(client, fast_policy(3));
        let resp = retrying.chat(vec![], None).await.unwrap();
        assert_eq!(resp.content, "first-call");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn chat_retries_on_transient_then_succeeds() {
        let (client, calls) = ProgrammableClient::new(vec![
            Err(LlmError::RateLimited),
            Err(LlmError::ApiError("(500): server error".to_string())),
            Ok(ChatResponse {
                content: "third".to_string(),
                tool_calls: None,
                reasoning: None,
                usage: None,
            }),
        ]);
        let retrying = RetryingLlmClient::new(client, fast_policy(5));
        let resp = retrying.chat(vec![], None).await.unwrap();
        assert_eq!(resp.content, "third");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn chat_does_not_retry_non_retryable_error() {
        let (client, calls) = ProgrammableClient::new(vec![Err(LlmError::AuthenticationFailed)]);
        let retrying = RetryingLlmClient::new(client, fast_policy(3));
        let err = retrying.chat(vec![], None).await.unwrap_err();
        assert!(matches!(err, LlmError::AuthenticationFailed));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn chat_exhausts_retries_then_returns_last_error() {
        // Always rate-limited
        let (client, calls) = ProgrammableClient::new(vec![
            Err(LlmError::RateLimited),
            Err(LlmError::RateLimited),
            Err(LlmError::RateLimited),
        ]);
        let retrying = RetryingLlmClient::new(client, fast_policy(2));
        let err = retrying.chat(vec![], None).await.unwrap_err();
        assert!(matches!(err, LlmError::RateLimited));
        // 1 initial + 2 retries
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn chat_stream_passes_through_no_retry() {
        let (client, calls) = ProgrammableClient::new(vec![]);
        let retrying = RetryingLlmClient::new(client, fast_policy(5));
        let resp = retrying
            .chat_stream(vec![], None, Box::new(|_| {}))
            .await
            .unwrap();
        assert_eq!(resp.content, "stream-ok");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn forwarding_methods_match_inner() {
        let (client, _) = ProgrammableClient::new(vec![]);
        let retrying = RetryingLlmClient::new(client, fast_policy(0));
        assert_eq!(retrying.model(), "test-model");
        assert_eq!(retrying.provider(), "test-provider");
        assert!(retrying.supports_tools());
        assert!(retrying.supports_reasoning());
    }
}
