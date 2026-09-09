//! HTTP client for the z-Bot daemon.
//!
//! Wraps the REST endpoints the CLI consumes:
//! - `GET  /api/health`         — smoke test on startup
//! - `POST /api/chat/init`      — reserve / reuse the persistent chat session
//! - `DELETE /api/chat/session` — clear the chat session (for `/new`)
//! - `GET  /api/wards`          — list wards (for `/wards` slash command)
//! - `GET  /api/conversations`  — list conversations (for `/sessions` picker)
//! - `GET  /api/memory/search`  — quick recall (for `/memory <q>`)
//!
//! The chat message flow goes over WebSocket — see `events.rs`.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::config::Config;

/// Thin wrapper over `reqwest::Client` bound to a daemon URL.
#[derive(Debug, Clone)]
pub struct DaemonClient {
    http: reqwest::Client,
    base: String,
}

impl DaemonClient {
    pub fn new(cfg: Config) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("zbot/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest::Client should always build");
        Self {
            http,
            base: cfg.daemon_url,
        }
    }

    /// `GET /api/health` — startup smoke test.
    pub async fn health(&self) -> Result<HealthResponse> {
        let url = format!("{}/api/health", self.base);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(anyhow!(
                "daemon health check returned HTTP {}",
                resp.status()
            ));
        }
        let body: HealthResponse = resp.json().await.context("parse /api/health body")?;
        Ok(body)
    }

    /// `POST /api/chat/init` — reserve (or reuse) the persistent chat session.
    ///
    /// Returns `{sessionId, conversationId, created}`. Idempotent — repeated
    /// calls return the same ids until `/api/chat/session` (DELETE) clears
    /// them or the cached session row disappears from the DB.
    pub async fn init_chat_session(&self) -> Result<ChatInit> {
        let url = format!("{}/api/chat/init", self.base);
        let resp = self
            .http
            .post(&url)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        if !resp.status().is_success() {
            return Err(anyhow!("/api/chat/init returned HTTP {}", resp.status()));
        }
        let body: ChatInit = resp.json().await.context("parse /api/chat/init body")?;
        Ok(body)
    }

    /// `DELETE /api/chat/session` — clear the cached chat session (for `/new`).
    pub async fn clear_chat_session(&self) -> Result<()> {
        let url = format!("{}/api/chat/session", self.base);
        let resp = self
            .http
            .delete(&url)
            .send()
            .await
            .with_context(|| format!("DELETE {url}"))?;
        if !resp.status().is_success() {
            return Err(anyhow!("/api/chat/session returned HTTP {}", resp.status()));
        }
        Ok(())
    }

    /// `GET /api/wards` — list wards for `/wards` slash command.
    pub async fn list_wards(&self) -> Result<Value> {
        let url = format!("{}/api/wards", self.base);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(anyhow!("/api/wards returned HTTP {}", resp.status()));
        }
        resp.json::<Value>().await.context("parse /api/wards body")
    }

    /// `GET /api/conversations` — list recent conversations for `/sessions`.
    pub async fn list_conversations(&self) -> Result<Value> {
        let url = format!("{}/api/conversations", self.base);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(anyhow!(
                "/api/conversations returned HTTP {}",
                resp.status()
            ));
        }
        resp.json::<Value>()
            .await
            .context("parse /api/conversations body")
    }

    /// `GET /api/memory/search?q=...` — quick memory recall for `/memory <q>`.
    pub async fn memory_search(&self, query: &str, limit: usize) -> Result<Value> {
        let url = format!("{}/api/memory/search", self.base);
        let resp = self
            .http
            .get(&url)
            .query(&[("q", query), ("limit", &limit.to_string())])
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(anyhow!(
                "/api/memory/search returned HTTP {}",
                resp.status()
            ));
        }
        resp.json::<Value>()
            .await
            .context("parse /api/memory/search body")
    }

    /// Invoke the gateway-owned surface action registry. This deliberately
    /// forwards only catalog identifiers; the daemon owns state checks,
    /// approval policy, and durable audit records.
    pub async fn invoke_surface_action(
        &self,
        action_id: &str,
        target: &str,
        expected_state: &str,
    ) -> Result<Value> {
        let url = format!("{}/api/surfaces/actions", self.base);
        let response = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "action_id": action_id,
                "surface_id": format!("cli:{target}"),
                "target": target,
                "expected_state": expected_state,
            }))
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        if !response.status().is_success() {
            return Err(anyhow!(
                "/api/surfaces/actions returned HTTP {}",
                response.status()
            ));
        }
        response
            .json::<Value>()
            .await
            .context("parse surface action response")
    }
}

/// Shape of the `/api/health` response.
#[derive(Debug, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

/// Shape of the `/api/chat/init` response (camelCase wire format).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatInit {
    pub session_id: String,
    pub conversation_id: String,
}
