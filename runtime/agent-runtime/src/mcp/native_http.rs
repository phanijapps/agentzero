//! Cancellation/redaction boundary around the SDK's HTTP implementation.
//! The SDK owns protocol decoding; this wrapper owns request lifetime only.
use futures::stream::BoxStream;
use reqwest::header::{HeaderName, HeaderValue};
use rmcp::{
    model::ClientJsonRpcMessage,
    transport::streamable_http_client::{
        SseError, StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
    },
};
use std::{collections::HashMap, sync::Arc, time::Duration};

#[derive(Clone)]
pub(super) struct SessionHttpClient {
    pub client: reqwest_mcp::Client,
    pub canceled: tokio::sync::watch::Sender<bool>,
}

impl SessionHttpClient {
    async fn request<T>(
        &self,
        future: impl std::future::Future<Output = Result<T, StreamableHttpError<reqwest_mcp::Error>>>,
    ) -> Result<T, StreamableHttpError<std::io::Error>> {
        let mut canceled = self.canceled.subscribe();
        tokio::select! {
        biased;
        _ = canceled.wait_for(|value| *value) => Err(safe_error()),
        result = tokio::time::timeout(Duration::from_secs(30),future) => result.map_err(|_|safe_error())?.map_err(sanitize_error) }
    }
}

fn safe_error() -> StreamableHttpError<std::io::Error> {
    StreamableHttpError::Client(std::io::Error::other("MCP HTTP request failed or canceled"))
}
fn sanitize_error(
    error: StreamableHttpError<reqwest_mcp::Error>,
) -> StreamableHttpError<std::io::Error> {
    // Preserve protocol control signals, but never let server error bodies,
    // URLs or authentication challenge strings reach SDK diagnostic logging.
    match error {
        StreamableHttpError::ServerDoesNotSupportSse => {
            StreamableHttpError::ServerDoesNotSupportSse
        }
        StreamableHttpError::ServerDoesNotSupportDeleteSession => {
            StreamableHttpError::ServerDoesNotSupportDeleteSession
        }
        StreamableHttpError::SessionExpired => StreamableHttpError::SessionExpired,
        _ => safe_error(),
    }
}

impl StreamableHttpClient for SessionHttpClient {
    type Error = std::io::Error;
    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        self.request(
            self.client
                .post_message(uri, message, session_id, auth_header, headers),
        )
        .await
    }
    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<
        BoxStream<'static, Result<sse_stream::Sse, SseError>>,
        StreamableHttpError<Self::Error>,
    > {
        self.request(
            self.client
                .get_stream(uri, session_id, last_event_id, auth_header, headers),
        )
        .await
    }
    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        // DELETE is cleanup, so cancellation must not preempt its opportunity.
        tokio::time::timeout(
            Duration::from_secs(4),
            self.client
                .delete_session(uri, session_id, auth_header, headers),
        )
        .await
        .map_err(|_| safe_error())?
        .map_err(sanitize_error)
    }
}

pub(super) struct StartupGuard(pub Option<tokio::sync::watch::Sender<bool>>);
impl Drop for StartupGuard {
    fn drop(&mut self) {
        if let Some(sender) = &self.0 {
            sender.send_replace(true);
        }
    }
}
