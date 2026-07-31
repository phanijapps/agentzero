//! # Ollama HTTP Client
//!
//! Minimal async client for the subset of the Ollama HTTP API that the
//! embedding backend selection feature needs:
//!
//! - `GET  /api/tags`  — enumerate locally-available models + reachability
//!   probe.
//! - `POST /api/pull`  — stream-pull a model, emitting per-layer download
//!   progress over NDJSON.
//!
//! Actual `/v1/embeddings` calls continue to go through
//! [`agent_runtime::llm::OpenAiEmbeddingClient`] which already speaks the
//! OpenAI-compatible surface Ollama exposes.

use futures::StreamExt;
use serde::Deserialize;

/// Thin client over the Ollama HTTP API.
pub struct OllamaClient {
    base_url: String,
    http: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<TagEntry>,
}

#[derive(Debug, Deserialize)]
struct TagEntry {
    name: String,
}

#[derive(Debug, Deserialize)]
struct PullLine {
    #[serde(default)]
    status: String,
    #[serde(default)]
    completed: Option<u64>,
    #[serde(default)]
    total: Option<u64>,
    #[serde(default)]
    error: Option<String>,
}

impl OllamaClient {
    /// Build a client pointing at `base_url` (e.g. `http://localhost:11434`).
    #[must_use]
    pub fn new(base_url: String) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
        }
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// List models currently pulled into the local Ollama registry.
    ///
    /// # Errors
    ///
    /// Returns an error string on network failure or non-2xx response.
    pub async fn list_models(&self) -> Result<Vec<String>, String> {
        let url = self.endpoint("/api/tags");
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("ollama /api/tags: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("ollama /api/tags: HTTP {}", resp.status()));
        }
        let body = resp
            .json::<TagsResponse>()
            .await
            .map_err(|e| format!("ollama /api/tags parse: {e}"))?;
        Ok(body.models.into_iter().map(|m| m.name).collect())
    }

    /// Reachability probe — `Ok(())` if the daemon answers `GET /api/tags`.
    ///
    /// # Errors
    ///
    /// Any transport or status error.
    pub async fn ping(&self) -> Result<(), String> {
        self.list_models().await.map(|_| ())
    }

    /// Stream `POST /api/pull` with `stream=true`, invoking `on_progress` once
    /// per NDJSON line that reports download progress.
    ///
    /// Ollama emits lines shaped like:
    /// ```json
    /// {"status":"pulling manifest"}
    /// {"status":"downloading","completed":42000000,"total":670000000}
    /// {"status":"success"}
    /// ```
    ///
    /// An `{"error":"..."}` line aborts the pull with that error.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure, non-2xx response, an explicit
    /// `error` line, or if the stream ends without `status:"success"`.
    pub async fn pull_model<F>(&self, model: &str, on_progress: F) -> Result<(), String>
    where
        F: Fn(u64, u64) + Send + Sync,
    {
        let url = self.endpoint("/api/pull");
        let body = serde_json::json!({ "name": model, "stream": true });
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("ollama /api/pull: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("ollama /api/pull: HTTP {}", resp.status()));
        }

        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        let mut saw_success = false;

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| format!("ollama stream: {e}"))?;
            buf.extend_from_slice(&bytes);
            // Drain complete NDJSON lines.
            while let Some(nl) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=nl).collect();
                let line_str = String::from_utf8_lossy(&line).trim().to_string();
                if line_str.is_empty() {
                    continue;
                }
                match handle_pull_line(&line_str, &on_progress) {
                    PullLineOutcome::Continue => {}
                    PullLineOutcome::Success => saw_success = true,
                    PullLineOutcome::Error(e) => return Err(e),
                }
            }
        }

        // Flush any trailing buffered line (no terminating newline).
        let tail = String::from_utf8_lossy(&buf).trim().to_string();
        if !tail.is_empty() {
            match handle_pull_line(&tail, &on_progress) {
                PullLineOutcome::Continue => {}
                PullLineOutcome::Success => saw_success = true,
                PullLineOutcome::Error(e) => return Err(e),
            }
        }

        if saw_success {
            Ok(())
        } else {
            Err("ollama /api/pull: stream ended without success".to_string())
        }
    }
}

enum PullLineOutcome {
    Continue,
    Success,
    Error(String),
}

fn handle_pull_line<F>(line: &str, on_progress: &F) -> PullLineOutcome
where
    F: Fn(u64, u64) + Send + Sync,
{
    let parsed: PullLine = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return PullLineOutcome::Continue, // tolerate garbage
    };
    if let Some(err) = parsed.error {
        return PullLineOutcome::Error(format!("ollama pull error: {err}"));
    }
    if parsed.status == "success" {
        return PullLineOutcome::Success;
    }
    if let (Some(done), Some(total)) = (parsed.completed, parsed.total) {
        on_progress(done, total);
    }
    PullLineOutcome::Continue
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn list_models_parses_tags_response() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/api/tags");
                then.status(200)
                    .header("content-type", "application/json")
                    .json_body(serde_json::json!({
                        "models": [
                            {"name": "mxbai-embed-large"},
                            {"name": "bge-m3"}
                        ]
                    }));
            })
            .await;
        let c = OllamaClient::new(server.base_url());
        let got = c.list_models().await.unwrap();
        assert_eq!(got, vec!["mxbai-embed-large", "bge-m3"]);
    }

    #[tokio::test]
    async fn ping_unreachable_returns_error() {
        // No MockServer spun — the port is free, so connect fails.
        let c = OllamaClient::new("http://127.0.0.1:1".into());
        let err = c.ping().await.unwrap_err();
        assert!(err.contains("/api/tags"));
    }

    #[tokio::test]
    async fn pull_model_parses_ndjson_progress() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/api/pull");
                then.status(200)
                    .header("content-type", "application/x-ndjson")
                    .body(
                        "{\"status\":\"pulling manifest\"}\n\
                         {\"status\":\"downloading\",\"completed\":100,\"total\":670}\n\
                         {\"status\":\"downloading\",\"completed\":400,\"total\":670}\n\
                         {\"status\":\"success\"}\n",
                    );
            })
            .await;
        let c = OllamaClient::new(server.base_url());
        let calls = Arc::new(AtomicU64::new(0));
        let last_done = Arc::new(AtomicU64::new(0));
        let calls_c = calls.clone();
        let last_c = last_done.clone();
        let cb = move |done: u64, _total: u64| {
            calls_c.fetch_add(1, Ordering::SeqCst);
            last_c.store(done, Ordering::SeqCst);
        };
        c.pull_model("mxbai-embed-large", cb).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(last_done.load(Ordering::SeqCst), 400);
    }

    #[tokio::test]
    async fn pull_model_success_line_terminates() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/api/pull");
                then.status(200).body("{\"status\":\"success\"}\n");
            })
            .await;
        let c = OllamaClient::new(server.base_url());
        c.pull_model("m", |_, _| {}).await.unwrap();
    }

    #[tokio::test]
    async fn pull_model_error_line_returns_err() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/api/pull");
                then.status(200).body("{\"error\":\"model not found\"}\n");
            })
            .await;
        let c = OllamaClient::new(server.base_url());
        let err = c.pull_model("missing", |_, _| {}).await.unwrap_err();
        assert!(err.contains("model not found"), "got: {err}");
    }

    #[tokio::test]
    async fn pull_model_stream_without_success_errors() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/api/pull");
                then.status(200)
                    .body("{\"status\":\"downloading\",\"completed\":5,\"total\":10}\n");
            })
            .await;
        let c = OllamaClient::new(server.base_url());
        let err = c.pull_model("m", |_, _| {}).await.unwrap_err();
        assert!(err.contains("without success"));
    }

    #[tokio::test]
    async fn pull_model_http_error_status() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/api/pull");
                then.status(500).body("boom");
            })
            .await;
        let c = OllamaClient::new(server.base_url());
        let err = c.pull_model("m", |_, _| {}).await.unwrap_err();
        assert!(err.contains("HTTP 500"), "got: {err}");
    }

    #[tokio::test]
    async fn pull_model_handles_unterminated_final_line() {
        // No trailing newline on the last line — common when servers flush
        // without a closing delimiter.
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/api/pull");
                then.status(200).body(
                    "{\"status\":\"downloading\",\"completed\":1,\"total\":2}\n\
                     {\"status\":\"success\"}",
                );
            })
            .await;
        let c = OllamaClient::new(server.base_url());
        c.pull_model("m", |_, _| {}).await.unwrap();
    }
}
