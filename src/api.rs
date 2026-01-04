//! Home Assistant REST API client
//!
//! Handles all HTTP communication with the Home Assistant REST API.

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use reqwest::{Client, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

use crate::config::RuntimeContext;

/// Validate and encode an entity_id for use in URL paths.
///
/// Home Assistant entity IDs follow the pattern `domain.object_id` where:
/// - domain: lowercase letters, numbers, underscores
/// - object_id: lowercase letters, numbers, underscores
///
/// This function validates the format and returns an error for invalid IDs,
/// preventing URL injection attacks.
fn validate_entity_id(entity_id: &str) -> Result<&str> {
    // Check for basic pattern: must contain exactly one dot
    let parts: Vec<&str> = entity_id.split('.').collect();
    if parts.len() != 2 {
        bail!("Invalid entity_id format: '{entity_id}'. Expected format: domain.object_id (e.g., light.kitchen)");
    }

    let (domain, object_id) = (parts[0], parts[1]);

    // Validate domain and object_id contain only allowed characters
    let is_valid_part = |s: &str| -> bool {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };

    if !is_valid_part(domain) || !is_valid_part(object_id) {
        bail!(
            "Invalid entity_id: '{entity_id}'. Domain and object_id must contain only \
            lowercase letters, numbers, and underscores"
        );
    }

    Ok(entity_id)
}

/// Validate a domain name (e.g., "light", "switch")
fn validate_domain(domain: &str) -> Result<&str> {
    let is_valid = !domain.is_empty()
        && domain
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');

    if !is_valid {
        bail!(
            "Invalid domain: '{domain}'. Must contain only lowercase letters, numbers, and underscores"
        );
    }

    Ok(domain)
}

/// Validate a service name
fn validate_service_name(name: &str) -> Result<&str> {
    let is_valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');

    if !is_valid {
        bail!(
            "Invalid service name: '{name}'. Must contain only lowercase letters, numbers, and underscores"
        );
    }

    Ok(name)
}

/// Validate an event type name
fn validate_event_type(event_type: &str) -> Result<&str> {
    let is_valid = !event_type.is_empty()
        && event_type
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');

    if !is_valid {
        bail!(
            "Invalid event type: '{event_type}'. Must contain only lowercase letters, numbers, and underscores"
        );
    }

    Ok(event_type)
}

/// Home Assistant REST API client.
///
/// Each instance wraps a reqwest::Client which handles connection pooling internally.
/// While a new HassClient is created per command invocation, the underlying HTTP
/// connection pool is managed by reqwest and provides efficient connection reuse.
pub struct HassClient {
    client: Client,
    base_url: String,
    token: String,
}

impl HassClient {
    /// Create a new Home Assistant client from runtime context
    pub fn new(ctx: &RuntimeContext) -> Result<Self> {
        let base_url = ctx.server_url()?.trim_end_matches('/').to_string();
        let token = ctx.token()?.to_string();

        let mut builder = Client::builder()
            .timeout(Duration::from_secs(ctx.timeout()))
            .user_agent(format!("hmr/{}", env!("CARGO_PKG_VERSION")));

        if ctx.insecure() {
            builder = builder.danger_accept_invalid_certs(true);
        }

        let client = builder.build().context("building HTTP client")?;

        Ok(Self {
            client,
            base_url,
            token,
        })
    }

    /// Make a GET request to the API
    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}/api{}", self.base_url, path);
        log::debug!("GET {url}");

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await
            .with_context(|| format!("request to {url}"))?;

        self.handle_response(response).await
    }

    /// Make a POST request to the API
    async fn post<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T> {
        let url = format!("{}/api{}", self.base_url, path);
        log::debug!("POST {url}");
        log::trace!("POST body: {body:?}");

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await
            .with_context(|| format!("request to {url}"))?;

        self.handle_response(response).await
    }

    /// Make a DELETE request to the API
    #[allow(dead_code)]
    async fn delete<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}/api{}", self.base_url, path);
        log::debug!("DELETE {url}");

        let response = self
            .client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await
            .with_context(|| format!("request to {url}"))?;

        self.handle_response(response).await
    }

    async fn handle_response<T: DeserializeOwned>(&self, response: reqwest::Response) -> Result<T> {
        let status = response.status();
        let url = response.url().to_string();

        if !status.is_success() {
            // Try to read the error body; log if that fails (network error during error response)
            let error_text = match response.text().await {
                Ok(text) => text,
                Err(e) => {
                    log::debug!("Failed to read error response body: {e}");
                    String::new()
                }
            };
            return Err(self.status_to_error(status, &url, &error_text));
        }

        response
            .json()
            .await
            .with_context(|| format!("parsing response from {url}"))
    }

    fn status_to_error(&self, status: StatusCode, url: &str, body: &str) -> anyhow::Error {
        let hint = match status {
            StatusCode::UNAUTHORIZED => "Check your authentication token (HASS_TOKEN or --token)",
            StatusCode::FORBIDDEN => "Your token may not have sufficient permissions",
            StatusCode::NOT_FOUND => "The requested resource was not found",
            StatusCode::SERVICE_UNAVAILABLE => "Home Assistant may be starting up or restarting",
            StatusCode::BAD_REQUEST => "Invalid request parameters",
            _ => "",
        };

        let msg = if body.is_empty() {
            format!("HTTP {status} from {url}")
        } else {
            format!("HTTP {status} from {url}: {body}")
        };

        if hint.is_empty() {
            anyhow!(msg)
        } else {
            anyhow!("{msg}\nHint: {hint}")
        }
    }

    // --- API Methods ---

    /// Get Home Assistant instance information
    pub async fn get_info(&self) -> Result<HassInfo> {
        // The / endpoint returns basic API info
        let response: Value = self.get("/").await?;

        // Also get config for more details
        let config: HassConfig = self.get("/config").await?;

        Ok(HassInfo {
            message: response
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Home Assistant API")
                .to_string(),
            location_name: config.location_name,
            version: config.version,
            config_dir: config.config_dir,
            time_zone: config.time_zone,
            components: config.components,
            state: config.state,
            latitude: config.latitude,
            longitude: config.longitude,
            elevation: config.elevation,
            unit_system: config.unit_system,
        })
    }

    /// Get all entity states
    pub async fn get_states(&self) -> Result<Vec<EntityState>> {
        self.get("/states").await
    }

    /// Get a specific entity state
    pub async fn get_state(&self, entity_id: impl AsRef<str>) -> Result<EntityState> {
        let entity_id = validate_entity_id(entity_id.as_ref())?;
        self.get(&format!("/states/{entity_id}")).await
    }

    /// Set entity state
    pub async fn set_state(&self, entity_id: impl AsRef<str>, data: &Value) -> Result<EntityState> {
        let entity_id = validate_entity_id(entity_id.as_ref())?;
        self.post(&format!("/states/{entity_id}"), data).await
    }

    /// Get entity history
    pub async fn get_history(
        &self,
        entity_id: impl AsRef<str>,
        start_time: impl AsRef<str>,
    ) -> Result<Vec<Vec<EntityState>>> {
        let entity_id = validate_entity_id(entity_id.as_ref())?;
        let start_time = start_time.as_ref();
        // URL-encode the entity_id for query string
        let encoded_entity_id = urlencoding::encode(entity_id);
        self.get(&format!(
            "/history/period/{start_time}?filter_entity_id={encoded_entity_id}"
        ))
        .await
    }

    /// Get all services
    pub async fn get_services(&self) -> Result<Vec<ServiceDomain>> {
        self.get("/services").await
    }

    /// Call a service
    pub async fn call_service(
        &self,
        domain: impl AsRef<str>,
        service: impl AsRef<str>,
        data: &Value,
    ) -> Result<Value> {
        let domain = validate_domain(domain.as_ref())?;
        let service = validate_service_name(service.as_ref())?;
        self.post(&format!("/services/{domain}/{service}"), data)
            .await
    }

    /// Fire an event
    pub async fn fire_event(&self, event_type: impl AsRef<str>, data: &Value) -> Result<Value> {
        let event_type = validate_event_type(event_type.as_ref())?;
        self.post(&format!("/events/{event_type}"), data).await
    }

    /// Render a template
    pub async fn render_template(&self, template: impl AsRef<str>) -> Result<String> {
        let template = template.as_ref();
        let body = serde_json::json!({ "template": template });
        let url = format!("{}/api/template", self.base_url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .with_context(|| format!("request to {url}"))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(self.status_to_error(status, &url, &error_text));
        }

        response.text().await.context("reading template response")
    }

    /// Process a conversation through Home Assistant's conversation agent
    pub async fn process_conversation(
        &self,
        text: impl AsRef<str>,
        language: Option<&str>,
        agent_id: Option<&str>,
        conversation_id: Option<&str>,
    ) -> Result<ConversationResponse> {
        let mut body = serde_json::json!({
            "text": text.as_ref(),
        });

        if let Some(lang) = language {
            body["language"] = serde_json::json!(lang);
        }

        if let Some(agent) = agent_id {
            body["agent_id"] = serde_json::json!(agent);
        }

        if let Some(conv_id) = conversation_id {
            body["conversation_id"] = serde_json::json!(conv_id);
        }

        self.post("/conversation/process", &body).await
    }
}

// --- Request Types ---

/// Request body for setting an entity's state.
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct SetStateRequest {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<serde_json::Map<String, Value>>,
}

#[allow(dead_code)]
impl SetStateRequest {
    /// Create a new state request with just the state value.
    pub fn new(state: impl Into<String>) -> Self {
        Self {
            state: state.into(),
            attributes: None,
        }
    }

    /// Add attributes to the state request.
    pub fn with_attributes(mut self, attributes: serde_json::Map<String, Value>) -> Self {
        self.attributes = Some(attributes);
        self
    }
}

/// Request body for calling a service.
#[derive(Debug, Clone, Default, Serialize)]
#[allow(dead_code)]
pub struct CallServiceRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    #[serde(flatten)]
    pub data: serde_json::Map<String, Value>,
}

#[allow(dead_code)]
impl CallServiceRequest {
    /// Create an empty service call request.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a service call request targeting a specific entity.
    pub fn for_entity(entity_id: impl Into<String>) -> Self {
        Self {
            entity_id: Some(entity_id.into()),
            data: serde_json::Map::new(),
        }
    }

    /// Add a data field to the request.
    pub fn with_data(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.data.insert(key.into(), value.into());
        self
    }
}

/// Request body for firing an event.
#[derive(Debug, Clone, Default, Serialize)]
#[allow(dead_code)]
pub struct FireEventRequest {
    #[serde(flatten)]
    pub data: serde_json::Map<String, Value>,
}

#[allow(dead_code)]
impl FireEventRequest {
    /// Create an empty event request.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a data field to the event.
    pub fn with_data(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.data.insert(key.into(), value.into());
        self
    }
}

// --- Response Types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HassInfo {
    pub message: String,
    pub location_name: String,
    pub version: String,
    pub config_dir: String,
    pub time_zone: String,
    pub components: Vec<String>,
    pub state: String,
    pub latitude: f64,
    pub longitude: f64,
    pub elevation: i32,
    pub unit_system: UnitSystem,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HassConfig {
    pub location_name: String,
    pub version: String,
    pub config_dir: String,
    pub time_zone: String,
    pub components: Vec<String>,
    pub state: String,
    #[serde(default)]
    pub latitude: f64,
    #[serde(default)]
    pub longitude: f64,
    #[serde(default)]
    pub elevation: i32,
    pub unit_system: UnitSystem,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitSystem {
    pub length: String,
    pub mass: String,
    pub temperature: String,
    pub volume: String,
    #[serde(default)]
    pub pressure: String,
    #[serde(default)]
    pub wind_speed: String,
    #[serde(default)]
    pub accumulated_precipitation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityState {
    pub entity_id: String,
    pub state: String,
    #[serde(default)]
    pub attributes: Value,
    pub last_changed: String,
    pub last_updated: String,
    #[serde(default)]
    pub context: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDomain {
    pub domain: String,
    pub services: std::collections::HashMap<String, ServiceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub fields: Value,
    #[serde(default)]
    pub target: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationResponse {
    pub response: ConversationResponseData,
    #[serde(default)]
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationResponseData {
    pub response_type: String,
    #[serde(default)]
    pub speech: Option<ConversationSpeech>,
    #[serde(default)]
    pub card: Option<Value>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSpeech {
    pub plain: ConversationPlainSpeech,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationPlainSpeech {
    pub speech: String,
    #[serde(default)]
    pub extra_data: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entity_state_deserialize() {
        let json = r#"{
            "entity_id": "light.kitchen",
            "state": "on",
            "attributes": {"brightness": 255},
            "last_changed": "2025-01-15T10:30:00Z",
            "last_updated": "2025-01-15T10:30:00Z"
        }"#;

        let state: EntityState = serde_json::from_str(json).unwrap();
        assert_eq!(state.entity_id, "light.kitchen");
        assert_eq!(state.state, "on");
    }

    #[test]
    fn test_service_domain_deserialize() {
        let json = r#"{
            "domain": "light",
            "services": {
                "turn_on": {
                    "name": "Turn on",
                    "description": "Turn on a light",
                    "fields": {}
                }
            }
        }"#;

        let domain: ServiceDomain = serde_json::from_str(json).unwrap();
        assert_eq!(domain.domain, "light");
        assert!(domain.services.contains_key("turn_on"));
    }

    #[test]
    fn test_validate_entity_id_valid() {
        assert!(validate_entity_id("light.kitchen").is_ok());
        assert!(validate_entity_id("sensor.temperature_1").is_ok());
        assert!(validate_entity_id("binary_sensor.motion_2").is_ok());
    }

    #[test]
    fn test_validate_entity_id_invalid() {
        // No dot
        assert!(validate_entity_id("light_kitchen").is_err());
        // Multiple dots
        assert!(validate_entity_id("light.kitchen.main").is_err());
        // Path traversal attempt
        assert!(validate_entity_id("../etc/passwd").is_err());
        // Query injection attempt
        assert!(validate_entity_id("light.foo?admin=true").is_err());
        // Uppercase letters
        assert!(validate_entity_id("Light.Kitchen").is_err());
        // Empty parts
        assert!(validate_entity_id(".kitchen").is_err());
        assert!(validate_entity_id("light.").is_err());
    }

    #[test]
    fn test_validate_domain_valid() {
        assert!(validate_domain("light").is_ok());
        assert!(validate_domain("binary_sensor").is_ok());
        assert!(validate_domain("sensor2").is_ok());
    }

    #[test]
    fn test_validate_domain_invalid() {
        assert!(validate_domain("").is_err());
        assert!(validate_domain("Light").is_err());
        assert!(validate_domain("light/turn_on").is_err());
        assert!(validate_domain("../etc").is_err());
    }
}
