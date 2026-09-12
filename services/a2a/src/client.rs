use crate::peers::TrustedPeer;
use crate::{APPLICATION_A2A_JSON, MAX_RESPONSE_BODY_BYTES, MAX_TASK_ID_BYTES};
use a2a::{SendMessageRequest, SendMessageResponse, Task};
use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Method, StatusCode, Url};
use std::net::SocketAddr;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum A2aClientError {
    #[error("trusted peer is not configured for outbound A2A")]
    PeerUnavailable,
    #[error("peer authentication failed")]
    Unauthorized,
    #[error("peer request should be retried")]
    Retryable,
    #[error("peer returned an invalid A2A response")]
    Protocol,
    #[error("peer response exceeded the configured bound")]
    ResponseTooLarge,
}

impl A2aClientError {
    pub fn is_retryable(self) -> bool {
        matches!(self, Self::Retryable)
    }
}

#[async_trait]
pub trait A2aTransport: Send + Sync {
    async fn send_message(
        &self,
        peer: &TrustedPeer,
        request: &SendMessageRequest,
    ) -> Result<Task, A2aClientError>;

    async fn get_task(&self, peer: &TrustedPeer, task_id: &str) -> Result<Task, A2aClientError>;
}

#[derive(Debug, Clone)]
pub struct HttpA2aTransport {
    connect_timeout: Duration,
    request_timeout: Duration,
}

impl Default for HttpA2aTransport {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(30),
        }
    }
}

impl HttpA2aTransport {
    pub fn new(
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, A2aClientError> {
        if connect_timeout.is_zero()
            || connect_timeout > Duration::from_secs(30)
            || request_timeout.is_zero()
            || request_timeout > Duration::from_secs(120)
        {
            return Err(A2aClientError::PeerUnavailable);
        }
        Ok(Self {
            connect_timeout,
            request_timeout,
        })
    }

    async fn endpoint(
        &self,
        peer: &TrustedPeer,
        segments: &[&str],
    ) -> Result<(reqwest::Client, Url, String), A2aClientError> {
        let origin = peer
            .origin
            .as_ref()
            .ok_or(A2aClientError::PeerUnavailable)?;
        let token = peer
            .outbound_credential
            .as_ref()
            .ok_or(A2aClientError::PeerUnavailable)?
            .exposed()
            .to_owned();
        let mut url = Url::parse(origin.origin()).map_err(|_| A2aClientError::PeerUnavailable)?;
        let host = url
            .host_str()
            .ok_or(A2aClientError::PeerUnavailable)?
            .to_owned();
        let port = url
            .port_or_known_default()
            .ok_or(A2aClientError::PeerUnavailable)?;
        let resolution = tokio::time::timeout(
            self.connect_timeout,
            tokio::net::lookup_host((host.as_str(), port)),
        )
        .await
        .map_err(|_| A2aClientError::Retryable)?;
        let addrs: Vec<SocketAddr> = resolution.map_err(|_| A2aClientError::Retryable)?.collect();
        origin
            .validate_resolution(addrs.iter().map(SocketAddr::ip))
            .map_err(|_| A2aClientError::PeerUnavailable)?;
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| A2aClientError::PeerUnavailable)?;
            path.clear();
            for segment in segments {
                path.push(segment);
            }
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(self.connect_timeout)
            .timeout(self.request_timeout)
            .resolve_to_addrs(&host, &addrs)
            .build()
            .map_err(|_| A2aClientError::PeerUnavailable)?;
        Ok((client, url, token))
    }

    async fn execute<T, B>(
        &self,
        peer: &TrustedPeer,
        method: Method,
        segments: &[&str],
        body: Option<&B>,
        query: &[(&str, &str)],
    ) -> Result<T, A2aClientError>
    where
        T: serde::de::DeserializeOwned,
        B: serde::Serialize + ?Sized,
    {
        let (client, mut url, token) = self.endpoint(peer, segments).await?;
        url.query_pairs_mut().extend_pairs(query.iter().copied());
        let mut request = client
            .request(method, url)
            .header("A2A-Version", a2a::VERSION)
            .header(AUTHORIZATION, format!("Bearer {token}"));
        if let Some(body) = body {
            request = request
                .header(CONTENT_TYPE, APPLICATION_A2A_JSON)
                .json(body);
        }
        let mut response = request.send().await.map_err(classify_reqwest)?;
        match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(A2aClientError::Unauthorized)
            }
            status if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS => {
                return Err(A2aClientError::Retryable)
            }
            status if !status.is_success() => return Err(A2aClientError::Protocol),
            _ => {}
        }
        if response
            .headers()
            .get("A2A-Version")
            .and_then(|value| value.to_str().ok())
            != Some(a2a::VERSION)
            || response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                != Some(APPLICATION_A2A_JSON)
        {
            return Err(A2aClientError::Protocol);
        }
        if response.content_length().is_some_and(|size| {
            size > u64::try_from(MAX_RESPONSE_BODY_BYTES).expect("response bound fits u64")
        }) {
            return Err(A2aClientError::ResponseTooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(classify_reqwest)? {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BODY_BYTES {
                return Err(A2aClientError::ResponseTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| A2aClientError::Protocol)
    }
}

#[async_trait]
impl A2aTransport for HttpA2aTransport {
    async fn send_message(
        &self,
        peer: &TrustedPeer,
        request: &SendMessageRequest,
    ) -> Result<Task, A2aClientError> {
        let response: SendMessageResponse = self
            .execute(
                peer,
                Method::POST,
                &["a2a", "message:send"],
                Some(request),
                &[],
            )
            .await?;
        match response {
            SendMessageResponse::Task(task) => Ok(task),
            SendMessageResponse::Message(_) => Err(A2aClientError::Protocol),
        }
    }

    async fn get_task(&self, peer: &TrustedPeer, task_id: &str) -> Result<Task, A2aClientError> {
        if task_id.is_empty() || task_id.len() > MAX_TASK_ID_BYTES {
            return Err(A2aClientError::Protocol);
        }
        self.execute::<Task, serde_json::Value>(
            peer,
            Method::GET,
            &["a2a", "tasks", task_id],
            None,
            &[("includeArtifacts", "true")],
        )
        .await
    }
}

fn classify_reqwest(error: reqwest::Error) -> A2aClientError {
    if error.is_timeout() || error.is_connect() || error.is_request() || error.is_body() {
        A2aClientError::Retryable
    } else {
        A2aClientError::Protocol
    }
}
