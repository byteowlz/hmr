//! Home Assistant WebSocket API client
//!
//! Handles real-time event streaming and entity watching.

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::config::RuntimeContext;

/// WebSocket message types from Home Assistant
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum WsMessage {
    AuthRequired {
        ha_version: String,
    },
    AuthOk {
        ha_version: String,
    },
    AuthInvalid {
        message: String,
    },
    Result {
        id: u64,
        success: bool,
        #[serde(default)]
        result: Value,
        #[serde(default)]
        error: Option<WsError>,
    },
    Event {
        id: u64,
        event: WsEvent,
    },
    Pong {
        id: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsEvent {
    pub event_type: String,
    #[serde(default)]
    pub data: Value,
    pub origin: String,
    pub time_fired: String,
    #[serde(default)]
    pub context: Value,
}

/// Home Assistant WebSocket client
pub struct WsClient {
    sender: mpsc::Sender<String>,
    receiver: mpsc::Receiver<WsMessage>,
    msg_id: u64,
}

impl WsClient {
    /// Connect to Home Assistant WebSocket API
    pub async fn connect(ctx: &RuntimeContext) -> Result<Self> {
        let server_url = ctx.server_url()?;
        let token = ctx.token()?.to_string();

        // Convert HTTP URL to WebSocket URL
        let ws_url = server_url
            .replace("http://", "ws://")
            .replace("https://", "wss://");
        let ws_url = format!("{}/api/websocket", ws_url.trim_end_matches('/'));

        log::debug!("Connecting to WebSocket: {}", ws_url);

        // Use string directly - tokio-tungstenite accepts &str
        let (ws_stream, _) = connect_async(&ws_url)
            .await
            .context("connecting to WebSocket")?;

        let (mut write, mut read) = ws_stream.split();

        // Create channels for communication
        let (tx_send, mut rx_send) = mpsc::channel::<String>(32);
        let (tx_recv, rx_recv) = mpsc::channel::<WsMessage>(32);

        // Spawn task to handle sending messages
        let tx_send_clone = tx_send.clone();
        tokio::spawn(async move {
            while let Some(msg) = rx_send.recv().await {
                if write.send(Message::Text(msg)).await.is_err() {
                    break;
                }
            }
        });

        // Spawn task to handle receiving messages
        tokio::spawn(async move {
            while let Some(Ok(msg)) = read.next().await {
                if let Message::Text(text) = msg {
                    if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                        if tx_recv.send(ws_msg).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        let mut client = Self {
            sender: tx_send_clone,
            receiver: rx_recv,
            msg_id: 0,
        };

        // Wait for auth_required
        let auth_required = client.receive().await?;
        match auth_required {
            WsMessage::AuthRequired { ha_version } => {
                log::debug!("Connected to Home Assistant {}", ha_version);
            }
            _ => return Err(anyhow!("unexpected message, expected auth_required")),
        }

        // Send auth message
        let auth_msg = json!({
            "type": "auth",
            "access_token": token
        });
        client.send_raw(&auth_msg.to_string()).await?;

        // Wait for auth response
        let auth_response = client.receive().await?;
        match auth_response {
            WsMessage::AuthOk { ha_version } => {
                log::info!("Authenticated with Home Assistant {}", ha_version);
            }
            WsMessage::AuthInvalid { message } => {
                return Err(anyhow!("Authentication failed: {}", message));
            }
            _ => return Err(anyhow!("unexpected auth response")),
        }

        Ok(client)
    }

    async fn send_raw(&self, msg: &str) -> Result<()> {
        self.sender
            .send(msg.to_string())
            .await
            .context("sending WebSocket message")
    }

    fn next_id(&mut self) -> u64 {
        self.msg_id += 1;
        self.msg_id
    }

    async fn send(&mut self, msg: &Value) -> Result<u64> {
        let id = self.next_id();
        let mut msg = msg.clone();
        msg["id"] = json!(id);
        self.send_raw(&msg.to_string()).await?;
        Ok(id)
    }

    async fn receive(&mut self) -> Result<WsMessage> {
        self.receiver
            .recv()
            .await
            .ok_or_else(|| anyhow!("WebSocket connection closed"))
    }

    /// Subscribe to all events
    pub async fn subscribe_events(&mut self, event_type: Option<&str>) -> Result<u64> {
        let mut msg = json!({
            "type": "subscribe_events"
        });

        if let Some(et) = event_type {
            msg["event_type"] = json!(et);
        }

        self.send(&msg).await
    }

    /// Subscribe to state changes for specific entities
    #[allow(dead_code)]
    pub async fn subscribe_entities(&mut self, entity_ids: &[String]) -> Result<u64> {
        let msg = json!({
            "type": "subscribe_entities",
            "entity_ids": entity_ids
        });

        self.send(&msg).await
    }

    /// Receive the next event
    pub async fn next_event(&mut self) -> Result<WsMessage> {
        self.receive().await
    }
}

/// Run an event watch loop
pub async fn watch_events(
    ctx: &RuntimeContext,
    event_type: Option<&str>,
    mut handler: impl FnMut(&WsEvent) -> Result<bool>,
) -> Result<()> {
    let mut client = WsClient::connect(ctx).await?;
    let sub_id = client.subscribe_events(event_type).await?;

    log::debug!("Subscribed to events with id {}", sub_id);

    // Wait for subscription confirmation
    loop {
        match client.next_event().await? {
            WsMessage::Result {
                id, success, error, ..
            } if id == sub_id => {
                if !success {
                    if let Some(err) = error {
                        return Err(anyhow!("Subscription failed: {}", err.message));
                    }
                    return Err(anyhow!("Subscription failed"));
                }
                break;
            }
            _ => continue,
        }
    }

    // Process events
    loop {
        tokio::select! {
            msg = client.next_event() => {
                if let WsMessage::Event { event, .. } = msg? {
                    if !handler(&event)? {
                        break;
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                log::debug!("Received Ctrl+C, stopping watch");
                break;
            }
        }
    }

    Ok(())
}

/// Run an entity watch loop
pub async fn watch_entities(
    ctx: &RuntimeContext,
    entity_ids: &[String],
    mut handler: impl FnMut(&Value) -> Result<bool>,
) -> Result<()> {
    let mut client = WsClient::connect(ctx).await?;

    // Subscribe to state_changed events
    let sub_id = client.subscribe_events(Some("state_changed")).await?;

    log::debug!("Subscribed to state_changed events with id {}", sub_id);

    // Wait for subscription confirmation
    loop {
        match client.next_event().await? {
            WsMessage::Result {
                id, success, error, ..
            } if id == sub_id => {
                if !success {
                    if let Some(err) = error {
                        return Err(anyhow!("Subscription failed: {}", err.message));
                    }
                    return Err(anyhow!("Subscription failed"));
                }
                break;
            }
            _ => continue,
        }
    }

    // Filter and process events
    let entity_set: std::collections::HashSet<&str> =
        entity_ids.iter().map(|s| s.as_str()).collect();

    loop {
        tokio::select! {
            msg = client.next_event() => {
                if let WsMessage::Event { event, .. } = msg? {
                    if event.event_type == "state_changed" {
                        if let Some(entity_id) = event.data.get("entity_id").and_then(|v| v.as_str()) {
                            if entity_set.contains(entity_id) && !handler(&event.data)? {
                                break;
                            }
                        }
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                log::debug!("Received Ctrl+C, stopping watch");
                break;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_message_deserialize() {
        let json = r#"{"type": "auth_required", "ha_version": "2024.1.0"}"#;
        let msg: WsMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsMessage::AuthRequired { ha_version } => {
                assert_eq!(ha_version, "2024.1.0");
            }
            _ => panic!("unexpected message type"),
        }
    }

    #[test]
    fn test_ws_event_deserialize() {
        let json = r#"{
            "event_type": "state_changed",
            "data": {"entity_id": "light.kitchen"},
            "origin": "LOCAL",
            "time_fired": "2025-01-15T10:30:00Z"
        }"#;

        let event: WsEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_type, "state_changed");
        assert_eq!(event.data["entity_id"], "light.kitchen");
    }
}
