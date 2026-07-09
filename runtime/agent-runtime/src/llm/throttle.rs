// ============================================================================
// LLM THROTTLE
// Limits concurrent LLM API calls per provider to prevent 429 rate limits
// ============================================================================

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Semaphore;

use super::client::{ChatResponse, LlmClient, LlmError, StreamCallback};
use crate::types::ChatMessage;

/// An LLM client wrapper that limits concurrent API calls.
///
/// Wraps any `LlmClient` with a shared semaphore. All clients sharing
/// the same semaphore (e.g., all executors for the same provider) are
/// throttled together, preventing burst 429s.
pub struct ThrottledLlmClient {
    inner: Arc<dyn LlmClient>,
    semaphore: Arc<Semaphore>,
}

impl ThrottledLlmClient {
    /// Create a new throttled client.
    ///
    /// The semaphore should be shared across all clients for the same provider.
    /// `max_concurrent` controls how many simultaneous LLM calls are allowed.
    pub fn new(inner: Arc<dyn LlmClient>, semaphore: Arc<Semaphore>) -> Self {
        Self { inner, semaphore }
    }

    /// Create a new throttled client with a dedicated semaphore.
    pub fn with_limit(inner: Arc<dyn LlmClient>, max_concurrent: u32) -> Self {
        Self {
            inner,
            semaphore: Arc::new(Semaphore::new(max_concurrent as usize)),
        }
    }
}

#[async_trait]
impl LlmClient for ThrottledLlmClient {
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
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| LlmError::ApiError("Throttle semaphore closed".to_string()))?;

        tracing::debug!(
            provider = self.inner.provider(),
            available = self.semaphore.available_permits(),
            "Acquired throttle permit for chat()"
        );

        self.inner.chat(messages, tools).await
    }

    async fn chat_with_schema(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        output_schema: Option<Value>,
    ) -> Result<ChatResponse, LlmError> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| LlmError::ApiError("Throttle semaphore closed".to_string()))?;

        tracing::debug!(
            provider = self.inner.provider(),
            available = self.semaphore.available_permits(),
            "Acquired throttle permit for chat_with_schema()"
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
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| LlmError::ApiError("Throttle semaphore closed".to_string()))?;

        tracing::debug!(
            provider = self.inner.provider(),
            available = self.semaphore.available_permits(),
            "Acquired throttle permit for chat_stream()"
        );

        self.inner.chat_stream(messages, tools, callback).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct CountingClient {
        concurrent: Arc<AtomicU32>,
        max_seen: Arc<AtomicU32>,
    }

    #[async_trait]
    impl LlmClient for CountingClient {
        fn model(&self) -> &'static str {
            "test"
        }
        fn provider(&self) -> &'static str {
            "test"
        }

        async fn chat(
            &self,
            _messages: Vec<ChatMessage>,
            _tools: Option<Value>,
        ) -> Result<ChatResponse, LlmError> {
            let current = self.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
            // Update max seen
            self.max_seen.fetch_max(current, Ordering::SeqCst);
            // Simulate work
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            self.concurrent.fetch_sub(1, Ordering::SeqCst);
            Ok(ChatResponse {
                content: "ok".to_string(),
                tool_calls: None,
                reasoning: None,
                usage: None,
            })
        }

        async fn chat_stream(
            &self,
            _messages: Vec<ChatMessage>,
            _tools: Option<Value>,
            _callback: StreamCallback,
        ) -> Result<ChatResponse, LlmError> {
            self.chat(_messages, _tools).await
        }
    }

    #[tokio::test]
    async fn test_throttle_limits_concurrency() {
        let concurrent = Arc::new(AtomicU32::new(0));
        let max_seen = Arc::new(AtomicU32::new(0));

        let inner = Arc::new(CountingClient {
            concurrent: concurrent.clone(),
            max_seen: max_seen.clone(),
        });

        let throttled = Arc::new(ThrottledLlmClient::with_limit(inner, 2));

        // Fire 5 concurrent calls — should be limited to 2 at a time
        let mut handles = vec![];
        for _ in 0..5 {
            let client = throttled.clone();
            handles.push(tokio::spawn(async move {
                client.chat(vec![], None).await.unwrap();
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        assert!(
            max_seen.load(Ordering::SeqCst) <= 2,
            "Max concurrent was {} but limit is 2",
            max_seen.load(Ordering::SeqCst)
        );
    }

    #[tokio::test]
    async fn model_and_provider_passthrough() {
        let inner = Arc::new(CountingClient {
            concurrent: Arc::new(AtomicU32::new(0)),
            max_seen: Arc::new(AtomicU32::new(0)),
        });
        let throttled = ThrottledLlmClient::with_limit(inner, 1);
        assert_eq!(throttled.model(), "test");
        assert_eq!(throttled.provider(), "test");
    }

    #[tokio::test]
    async fn chat_stream_acquires_permit_and_returns_response() {
        let inner = Arc::new(CountingClient {
            concurrent: Arc::new(AtomicU32::new(0)),
            max_seen: Arc::new(AtomicU32::new(0)),
        });
        let throttled = ThrottledLlmClient::with_limit(inner, 1);
        let resp = throttled
            .chat_stream(vec![], None, Box::new(|_| {}))
            .await
            .unwrap();
        assert_eq!(resp.content, "ok");
    }

    #[tokio::test]
    async fn shared_semaphore_constructor() {
        let inner = Arc::new(CountingClient {
            concurrent: Arc::new(AtomicU32::new(0)),
            max_seen: Arc::new(AtomicU32::new(0)),
        });
        let sem = Arc::new(Semaphore::new(3));
        let throttled = ThrottledLlmClient::new(inner, Arc::clone(&sem));
        let resp = throttled.chat(vec![], None).await.unwrap();
        assert_eq!(resp.content, "ok");
        // Permit was released
        assert_eq!(sem.available_permits(), 3);
    }

    #[tokio::test]
    async fn closed_semaphore_returns_api_error() {
        let inner = Arc::new(CountingClient {
            concurrent: Arc::new(AtomicU32::new(0)),
            max_seen: Arc::new(AtomicU32::new(0)),
        });
        let sem = Arc::new(Semaphore::new(1));
        sem.close();
        let throttled = ThrottledLlmClient::new(inner, sem);
        let err = throttled.chat(vec![], None).await.unwrap_err();
        assert!(matches!(err, LlmError::ApiError(ref m) if m.contains("Throttle")));

        let inner2 = Arc::new(CountingClient {
            concurrent: Arc::new(AtomicU32::new(0)),
            max_seen: Arc::new(AtomicU32::new(0)),
        });
        let sem2 = Arc::new(Semaphore::new(1));
        sem2.close();
        let throttled2 = ThrottledLlmClient::new(inner2, sem2);
        let err = throttled2
            .chat_stream(vec![], None, Box::new(|_| {}))
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::ApiError(ref m) if m.contains("Throttle")));
    }
}
