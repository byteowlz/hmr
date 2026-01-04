//! Home Assistant WebSocket API client
//!
//! Handles real-time event streaming and entity watching.

use std::borrow::Cow;

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::config::RuntimeContext;

/// WebSocket message types from Home Assistant
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
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
    /// Handle to the sender task for error detection
    send_task: JoinHandle<()>,
    /// Handle to the receiver task for error detection
    recv_task: JoinHandle<()>,
}

impl WsClient {
    /// Connect to Home Assistant WebSocket API
    pub async fn connect(ctx: &RuntimeContext) -> Result<Self> {
        let server_url = ctx.server_url()?;
        let token = ctx.token()?.to_string();

        // Convert HTTP URL to WebSocket URL
        let ws_url = http_to_ws_url(server_url);
        let ws_url = format!("{}/api/websocket", ws_url.trim_end_matches('/'));

        log::debug!("Connecting to WebSocket: {ws_url}");

        // Use string directly - tokio-tungstenite accepts &str
        let (ws_stream, _) = connect_async(&ws_url)
            .await
            .context("connecting to WebSocket")?;

        let (mut write, mut read) = ws_stream.split();

        // Create channels for communication.
        // The bounded channels provide natural backpressure - if events arrive faster
        // than they can be processed, the sender will block until space is available.
        // This prevents unbounded memory growth at the cost of potentially dropping
        // the WebSocket connection if the receiver is too slow.
        let (tx_send, mut rx_send) = mpsc::channel::<String>(32);
        let (tx_recv, rx_recv) = mpsc::channel::<WsMessage>(32);

        // Spawn task to handle sending messages
        // Store the JoinHandle so we can detect task panics
        let tx_send_clone = tx_send.clone();
        let send_task = tokio::spawn(async move {
            while let Some(msg) = rx_send.recv().await {
                if write.send(Message::Text(msg)).await.is_err() {
                    log::debug!("WebSocket send task: connection closed");
                    break;
                }
            }
        });

        // Spawn task to handle receiving messages
        // Store the JoinHandle so we can detect task panics
        let recv_task = tokio::spawn(async move {
            while let Some(Ok(msg)) = read.next().await {
                if let Message::Text(text) = msg {
                    match serde_json::from_str::<WsMessage>(&text) {
                        Ok(ws_msg) => {
                            if tx_recv.send(ws_msg).await.is_err() {
                                log::debug!("WebSocket recv task: receiver dropped");
                                break;
                            }
                        }
                        Err(e) => {
                            log::debug!("Failed to parse WebSocket message: {e}");
                            log::trace!("Malformed message content: {text}");
                        }
                    }
                }
            }
            log::debug!("WebSocket recv task: stream ended");
        });

        let mut client = Self {
            sender: tx_send_clone,
            receiver: rx_recv,
            msg_id: 0,
            send_task,
            recv_task,
        };

        // Wait for auth_required
        let auth_required = client.receive().await?;
        match auth_required {
            WsMessage::AuthRequired { ha_version } => {
                log::debug!("Connected to Home Assistant {ha_version}");
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
                log::info!("Authenticated with Home Assistant {ha_version}");
            }
            WsMessage::AuthInvalid { message } => {
                return Err(anyhow!("Authentication failed: {message}"));
            }
            _ => return Err(anyhow!("unexpected auth response")),
        }

        Ok(client)
    }

    /// Send a raw string message, accepting owned or borrowed strings efficiently.
    async fn send_raw<'a>(&self, msg: impl Into<Cow<'a, str>>) -> Result<()> {
        // Check if the background tasks are still alive
        if self.send_task.is_finished() {
            return Err(anyhow!("WebSocket send task has terminated unexpectedly"));
        }

        self.sender
            .send(msg.into().into_owned())
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
        self.send_raw(msg.to_string()).await?;
        Ok(id)
    }

    async fn receive(&mut self) -> Result<WsMessage> {
        // Check if the receive task has panicked or terminated
        if self.recv_task.is_finished() {
            return Err(anyhow!(
                "WebSocket receive task has terminated unexpectedly"
            ));
        }

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

    /// Wait for a subscription confirmation message
    pub async fn wait_for_subscription_confirmation(&mut self, sub_id: u64) -> Result<()> {
        loop {
            match self.next_event().await? {
                WsMessage::Result {
                    id, success, error, ..
                } if id == sub_id => {
                    if !success {
                        if let Some(err) = error {
                            return Err(anyhow!("Subscription failed: {}", err.message));
                        }
                        return Err(anyhow!("Subscription failed"));
                    }
                    return Ok(());
                }
                _ => continue,
            }
        }
    }

    /// Call an RPC method and wait for the result
    ///
    /// This sends a message to Home Assistant and waits for a response with matching ID.
    /// Use this for registry operations like listing/creating/deleting areas and devices.
    pub async fn call_rpc(&mut self, msg: &Value) -> Result<Value> {
        let id = self.send(msg).await?;

        // Wait for the response with matching ID
        loop {
            match self.receive().await? {
                WsMessage::Result {
                    id: result_id,
                    success,
                    result,
                    error,
                } if result_id == id => {
                    if success {
                        return Ok(result);
                    }
                    if let Some(err) = error {
                        return Err(anyhow!("RPC call failed: {} ({})", err.message, err.code));
                    }
                    return Err(anyhow!("RPC call failed without error details"));
                }
                // Ignore other messages while waiting for our response
                _ => continue,
            }
        }
    }

    /// List all areas from the area registry
    pub async fn list_areas(&mut self) -> Result<Vec<Area>> {
        let msg = json!({
            "type": "config/area_registry/list"
        });

        let result = self.call_rpc(&msg).await?;
        serde_json::from_value(result).context("parsing area list response")
    }

    /// Create a new area
    pub async fn create_area(&mut self, request: &CreateAreaRequest) -> Result<Area> {
        let msg = json!({
            "type": "config/area_registry/create",
            "name": request.name,
            "picture": request.picture,
            "aliases": request.aliases,
            "icon": request.icon,
            "floor_id": request.floor_id,
        });

        let result = self.call_rpc(&msg).await?;
        serde_json::from_value(result).context("parsing created area response")
    }

    /// Delete an area by ID
    pub async fn delete_area(&mut self, area_id: &str) -> Result<()> {
        let msg = json!({
            "type": "config/area_registry/delete",
            "area_id": area_id
        });

        self.call_rpc(&msg).await?;
        Ok(())
    }

    /// List all devices from the device registry
    pub async fn list_devices(&mut self) -> Result<Vec<Device>> {
        let msg = json!({
            "type": "config/device_registry/list"
        });

        let result = self.call_rpc(&msg).await?;
        serde_json::from_value(result).context("parsing device list response")
    }

    /// Update a device's metadata
    pub async fn update_device(&mut self, request: &UpdateDeviceRequest) -> Result<Device> {
        let mut msg = json!({
            "type": "config/device_registry/update",
            "device_id": request.device_id,
        });

        if let Some(area_id) = &request.area_id {
            msg["area_id"] = json!(area_id);
        }
        if let Some(name_by_user) = &request.name_by_user {
            msg["name_by_user"] = json!(name_by_user);
        }
        if let Some(disabled_by) = &request.disabled_by {
            msg["disabled_by"] = json!(disabled_by);
        }

        let result = self.call_rpc(&msg).await?;
        serde_json::from_value(result).context("parsing updated device response")
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

    log::debug!("Subscribed to events with id {sub_id}");

    client.wait_for_subscription_confirmation(sub_id).await?;

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

    log::debug!("Subscribed to state_changed events with id {sub_id}");

    client.wait_for_subscription_confirmation(sub_id).await?;

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

/// Convert an HTTP/HTTPS URL to a WebSocket URL.
///
/// This handles the protocol conversion properly:
/// - `http://` -> `ws://`
/// - `https://` -> `wss://`
///
/// Preserves ports, paths, and query strings.
pub fn http_to_ws_url(url: &str) -> Cow<'_, str> {
    if let Some(rest) = url.strip_prefix("https://") {
        Cow::Owned(format!("wss://{rest}"))
    } else if let Some(rest) = url.strip_prefix("http://") {
        Cow::Owned(format!("ws://{rest}"))
    } else {
        // Already a ws:// or wss:// URL, or some other scheme
        Cow::Borrowed(url)
    }
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

    #[test]
    fn test_http_to_ws_url() {
        assert_eq!(
            http_to_ws_url("http://localhost:8123"),
            "ws://localhost:8123"
        );
        assert_eq!(
            http_to_ws_url("https://home.example.com"),
            "wss://home.example.com"
        );
        assert_eq!(
            http_to_ws_url("https://home.example.com:8443/path?query=1"),
            "wss://home.example.com:8443/path?query=1"
        );
        // Already a WebSocket URL should be unchanged
        assert_eq!(http_to_ws_url("ws://localhost:8123"), "ws://localhost:8123");
        assert_eq!(
            http_to_ws_url("wss://home.example.com"),
            "wss://home.example.com"
        );
    }
}

// --- Area Registry Types ---

/// Area information from Home Assistant area registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Area {
    pub area_id: String,
    pub name: String,
    #[serde(default)]
    pub picture: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub floor_id: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

/// Request to create a new area
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAreaRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor_id: Option<String>,
}

impl CreateAreaRequest {
    pub fn new(name: String) -> Self {
        Self {
            name,
            picture: None,
            aliases: None,
            icon: None,
            floor_id: None,
        }
    }
}

// --- Device Registry Types ---

/// Device information from Home Assistant device registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    #[serde(default)]
    pub area_id: Option<String>,
    #[serde(default)]
    pub configuration_url: Option<String>,
    #[serde(default)]
    pub config_entries: Vec<String>,
    #[serde(default)]
    pub connections: Vec<(String, String)>,
    #[serde(default)]
    pub disabled_by: Option<String>,
    #[serde(default)]
    pub entry_type: Option<String>,
    #[serde(default)]
    pub hw_version: Option<String>,
    #[serde(default)]
    pub identifiers: Vec<(String, String)>,
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub name_by_user: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub sw_version: Option<String>,
    #[serde(default)]
    pub via_device_id: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

/// Request to update device metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateDeviceRequest {
    pub device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_by_user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_by: Option<String>,
}

impl UpdateDeviceRequest {
    pub fn new(device_id: String) -> Self {
        Self {
            device_id,
            area_id: None,
            name_by_user: None,
            disabled_by: None,
        }
    }

    pub fn with_area_id(mut self, area_id: String) -> Self {
        self.area_id = Some(area_id);
        self
    }

    #[allow(dead_code)]
    pub fn with_name(mut self, name: String) -> Self {
        self.name_by_user = Some(name);
        self
    }

    #[allow(dead_code)]
    pub fn with_disabled_by(mut self, disabled_by: Option<String>) -> Self {
        self.disabled_by = disabled_by;
        self
    }
}
