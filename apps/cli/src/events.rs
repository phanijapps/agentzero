// `invoke()` is scaffolded ahead of the Phase 3 chat REPL that calls it.
#![allow(dead_code)]

//! WebSocket event stream client.
//!
//! Connects to the daemon's `/ws` endpoint and bridges it to two channels:
//! - **Outbound** (`send_to_ws`): app → server `ClientMessage`s
//! - **Inbound** (`recv_from_ws`): server → app `ServerMessage`s
//!
//! The background pump task owns the socket. Components can call `send()`
//! at any time; they receive events by `await`ing `recv()`. When the
//! `EventStream` is dropped, the pump task is cancelled.

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use gateway_ws_protocol::{ClientMessage, ServerMessage, SubscriptionScope};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Connected event stream. Holds two channels into a background socket
/// pump and the join handle for that pump.
pub struct EventStream {
    outbound: mpsc::UnboundedSender<ClientMessage>,
    inbound: mpsc::UnboundedReceiver<ServerMessage>,
    /// Aborted on drop — see `Drop` impl.
    pump: JoinHandle<()>,
}

impl EventStream {
    /// Open a WebSocket connection to the daemon and start the pump.
    pub async fn connect(ws_url: &str) -> Result<Self> {
        let (socket, _resp) = connect_async(ws_url)
            .await
            .with_context(|| format!("connect to {ws_url}"))?;
        let (mut writer, mut reader) = socket.split();

        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<ClientMessage>();
        let (in_tx, in_rx) = mpsc::unbounded_channel::<ServerMessage>();

        let pump = tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Outbound: app → server
                    msg = out_rx.recv() => {
                        let Some(msg) = msg else { break; };
                        let json = match serde_json::to_string(&msg) {
                            Ok(j) => j,
                            Err(e) => {
                                tracing::error!(error = %e, "serialize ClientMessage failed");
                                continue;
                            }
                        };
                        if let Err(e) = writer.send(Message::Text(json)).await {
                            tracing::error!(error = %e, "ws send failed");
                            break;
                        }
                    }

                    // Inbound: server → app
                    frame = reader.next() => {
                        match frame {
                            Some(Ok(Message::Text(txt))) => {
                                match serde_json::from_str::<ServerMessage>(&txt) {
                                    Ok(server_msg) => {
                                        if in_tx.send(server_msg).is_err() {
                                            // App dropped the receiver — pump exits.
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(error = %e, raw = %txt, "unknown ServerMessage");
                                    }
                                }
                            }
                            Some(Ok(Message::Ping(payload))) => {
                                if let Err(e) = writer.send(Message::Pong(payload)).await {
                                    tracing::error!(error = %e, "ws pong failed");
                                    break;
                                }
                            }
                            Some(Ok(Message::Close(_))) | None => {
                                tracing::info!("ws closed by server");
                                break;
                            }
                            Some(Ok(_)) => { /* ignore Binary / Pong / etc. */ }
                            Some(Err(e)) => {
                                tracing::error!(error = %e, "ws read failed");
                                break;
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            outbound: out_tx,
            inbound: in_rx,
            pump,
        })
    }

    /// Queue a ClientMessage for the server. Non-blocking.
    pub fn send(&self, msg: ClientMessage) -> Result<()> {
        self.outbound
            .send(msg)
            .map_err(|_| anyhow!("ws pump has stopped — cannot send"))
    }

    /// Subscribe to a conversation with the default `Session` scope.
    /// Convenience wrapper around `send(ClientMessage::Subscribe)`.
    pub fn subscribe(&self, conversation_id: impl Into<String>) -> Result<()> {
        self.send(ClientMessage::Subscribe {
            conversation_id: conversation_id.into(),
            scope: SubscriptionScope::Session,
        })
    }

    /// Send an `Invoke` to the root agent.
    pub fn invoke(
        &self,
        agent_id: impl Into<String>,
        conversation_id: impl Into<String>,
        session_id: Option<String>,
        message: impl Into<String>,
    ) -> Result<()> {
        self.send(ClientMessage::Invoke {
            agent_id: agent_id.into(),
            conversation_id: conversation_id.into(),
            message: message.into(),
            session_id,
            metadata: None,
            mode: "deep".to_string(),
        })
    }

    /// Await the next ServerMessage. Returns `None` when the pump exits.
    pub async fn recv(&mut self) -> Option<ServerMessage> {
        self.inbound.recv().await
    }
}

impl Drop for EventStream {
    fn drop(&mut self) {
        self.pump.abort();
    }
}
