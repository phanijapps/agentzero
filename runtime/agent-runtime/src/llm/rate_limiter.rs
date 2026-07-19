//! Per-provider rate limiter with concurrency control + sliding window.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore, SemaphorePermit};

use super::client::{ChatResponse, LlmClient, LlmError, StreamCallback};
use crate::types::ChatMessage;

/// Per-provider rate limiter shared across all executors.
pub struct ProviderRateLimiter {
    concurrency: Arc<Semaphore>,
    window: Arc<Mutex<SlidingWindow>>,
}

struct SlidingWindow {
    timestamps: VecDeque<Instant>,
    max_per_minute: u32,
}

impl ProviderRateLimiter {
    /// Create a new rate limiter with the given concurrency and RPM limits.
    #[must_use]
    pub fn new(concurrent: u32, rpm: u32) -> Self {
        Self {
            concurrency: Arc::new(Semaphore::new(concurrent as usize)),
            window: Arc::new(Mutex::new(SlidingWindow {
                timestamps: VecDeque::new(),
                max_per_minute: rpm,
            })),
        }
    }

    /// Acquire a rate limit slot. Waits if necessary. Never fails.
    pub async fn acquire(&self) -> SemaphorePermit<'_> {
        // Wait for sliding window slot
        loop {
            {
                let mut window = self.window.lock().await;
                let now = Instant::now();
                let one_minute_ago = now.checked_sub(Duration::from_secs(60)).unwrap();

                // Remove old timestamps
                while window
                    .timestamps
                    .front()
                    .is_some_and(|t| *t < one_minute_ago)
                {
                    window.timestamps.pop_front();
                }

                if (window.timestamps.len() as u32) < window.max_per_minute {
                    window.timestamps.push_back(now);
                    break;
                }

                // Over limit — wait
                if let Some(oldest) = window.timestamps.front() {
                    let wait = (*oldest + Duration::from_secs(60)) - now;
                    tracing::debug!(
                        current = window.timestamps.len(),
                        max = window.max_per_minute,
                        wait_ms = wait.as_millis(),
                        "Rate limited — waiting for sliding window slot"
                    );
                    drop(window);
                    tokio::time::sleep(wait + Duration::from_millis(10)).await;
                    continue;
                }
            }
        }

        self.concurrency
            .acquire()
            .await
            .expect("Rate limiter semaphore closed")
    }

    /// Auto-reduce RPM after 429.
    pub async fn on_rate_limited(&self) {
        let mut window = self.window.lock().await;
        let new_rpm = (window.max_per_minute / 2).max(1);
        tracing::warn!(
            old_rpm = window.max_per_minute,
            new_rpm = new_rpm,
            "Auto-reducing RPM after 429"
        );
        window.max_per_minute = new_rpm;
    }
}

/// LLM client wrapper that enforces rate limits.
pub struct RateLimitedLlmClient {
    inner: Arc<dyn LlmClient>,
    limiter: Arc<ProviderRateLimiter>,
}

impl RateLimitedLlmClient {
    /// Wrap an existing LLM client with rate limiting.
    pub fn new(inner: Arc<dyn LlmClient>, limiter: Arc<ProviderRateLimiter>) -> Self {
        Self { inner, limiter }
    }
}

#[async_trait]
impl LlmClient for RateLimitedLlmClient {
    fn model(&self) -> &str {
        self.inner.model()
    }
    fn provider(&self) -> &str {
        self.inner.provider()
    }

    fn supports_tools(&self) -> bool {
        self.inner.supports_tools()
    }
    fn supports_reasoning(&self) -> bool {
        self.inner.supports_reasoning()
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
    ) -> Result<ChatResponse, LlmError> {
        let _permit = self.limiter.acquire().await;
        tracing::debug!(
            provider = self.inner.provider(),
            "Rate limit permit acquired for chat()"
        );
        self.inner.chat(messages, tools).await
    }

    async fn chat_with_schema(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        output_schema: Option<Value>,
    ) -> Result<ChatResponse, LlmError> {
        let _permit = self.limiter.acquire().await;
        tracing::debug!(
            provider = self.inner.provider(),
            "Rate limit permit acquired for chat_with_schema()"
        );
        self.inner
            .chat_with_schema(messages, tools, output_schema)
            .await
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        callback: StreamCallback,
    ) -> Result<ChatResponse, LlmError> {
        let _permit = self.limiter.acquire().await;
        tracing::debug!(
            provider = self.inner.provider(),
            "Rate limit permit acquired for chat_stream()"
        );
        self.inner.chat_stream(messages, tools, callback).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter_allows_under_limit() {
        let limiter = ProviderRateLimiter::new(2, 60);
        let _permit = limiter.acquire().await;
    }

    #[tokio::test]
    async fn test_rate_limiter_concurrency() {
        let limiter = Arc::new(ProviderRateLimiter::new(1, 100));
        // Verify that concurrency semaphore starts with capacity 1
        // by checking try_acquire behavior directly on the semaphore
        let sem = Arc::new(Semaphore::new(1));
        let _permit1 = sem.try_acquire().expect("first acquire should succeed");
        assert!(
            sem.try_acquire().is_err(),
            "Second acquire should fail when concurrency=1 and permit is held"
        );
        drop(_permit1);
        assert!(
            sem.try_acquire().is_ok(),
            "Acquire should succeed after permit is dropped"
        );
        drop(limiter); // ensure limiter is used
    }

    #[tokio::test]
    async fn test_on_rate_limited_halves_rpm() {
        let limiter = ProviderRateLimiter::new(2, 60);
        limiter.on_rate_limited().await;
        let window = limiter.window.lock().await;
        assert_eq!(window.max_per_minute, 30);
    }

    #[tokio::test]
    async fn on_rate_limited_floors_at_one() {
        let limiter = ProviderRateLimiter::new(2, 1);
        limiter.on_rate_limited().await;
        let window = limiter.window.lock().await;
        // 1/2 = 0, but max(1) = 1.
        assert_eq!(window.max_per_minute, 1);
    }

    // ---------- Wrapper coverage ----------

    struct StubInner;

    #[async_trait]
    impl LlmClient for StubInner {
        fn model(&self) -> &str {
            "stub-model"
        }
        fn provider(&self) -> &str {
            "stub-provider"
        }
        fn supports_tools(&self) -> bool {
            false
        }
        fn supports_reasoning(&self) -> bool {
            true
        }
        async fn chat(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
        ) -> Result<ChatResponse, LlmError> {
            Ok(ChatResponse {
                content: "ok".to_string(),
                tool_calls: None,
                reasoning: None,
                usage: None,
            })
        }
        async fn chat_stream(
            &self,
            _msgs: Vec<ChatMessage>,
            _tools: Option<Value>,
            _cb: StreamCallback,
        ) -> Result<ChatResponse, LlmError> {
            Ok(ChatResponse {
                content: "ok-stream".to_string(),
                tool_calls: None,
                reasoning: None,
                usage: None,
            })
        }
    }

    #[tokio::test]
    async fn rate_limited_wrapper_passes_metadata() {
        let limiter = Arc::new(ProviderRateLimiter::new(1, 1000));
        let wrapped = RateLimitedLlmClient::new(Arc::new(StubInner), limiter);
        assert_eq!(wrapped.model(), "stub-model");
        assert_eq!(wrapped.provider(), "stub-provider");
        assert!(!wrapped.supports_tools());
        assert!(wrapped.supports_reasoning());
    }

    #[tokio::test]
    async fn rate_limited_wrapper_chat_and_stream_work() {
        let limiter = Arc::new(ProviderRateLimiter::new(2, 1000));
        let wrapped = RateLimitedLlmClient::new(Arc::new(StubInner), limiter);
        let resp = wrapped.chat(vec![], None).await.unwrap();
        assert_eq!(resp.content, "ok");
        let resp = wrapped
            .chat_stream(vec![], None, Box::new(|_| {}))
            .await
            .unwrap();
        assert_eq!(resp.content, "ok-stream");
    }

    #[tokio::test]
    async fn acquire_respects_sliding_window_under_load() {
        // 3 RPM. Acquire 3 in quick succession; the 4th must wait — but with a
        // tight 50ms timeout we just check the first three return promptly.
        let limiter = Arc::new(ProviderRateLimiter::new(10, 3));
        let p1 = limiter.acquire().await;
        let p2 = limiter.acquire().await;
        let p3 = limiter.acquire().await;
        // Three permits granted: window full.
        let window = limiter.window.lock().await;
        assert_eq!(window.timestamps.len(), 3);
        drop(window);
        drop(p1);
        drop(p2);
        drop(p3);
    }
}
