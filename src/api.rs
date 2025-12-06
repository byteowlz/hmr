//! Home Assistant REST API client
//!
//! Handles all HTTP communication with the Home Assistant REST API.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::{Client, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

use crate::config::RuntimeContext;

/// Home Assistant REST API client
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
        log::debug!("GET {}", url);

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
        log::debug!("POST {} {:?}", url, body);

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
        log::debug!("DELETE {}", url);

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
            let error_text = response.text().await.unwrap_or_default();
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
    pub async fn get_state(&self, entity_id: &str) -> Result<EntityState> {
        self.get(&format!("/states/{entity_id}")).await
    }

    /// Set entity state
    pub async fn set_state(&self, entity_id: &str, data: &Value) -> Result<EntityState> {
        self.post(&format!("/states/{entity_id}"), data).await
    }

    /// Get entity history
    pub async fn get_history(
        &self,
        entity_id: &str,
        start_time: &str,
    ) -> Result<Vec<Vec<EntityState>>> {
        self.get(&format!(
            "/history/period/{start_time}?filter_entity_id={entity_id}"
        ))
        .await
    }

    /// Get all services
    pub async fn get_services(&self) -> Result<Vec<ServiceDomain>> {
        self.get("/services").await
    }

    /// Call a service
    pub async fn call_service(&self, domain: &str, service: &str, data: &Value) -> Result<Value> {
        self.post(&format!("/services/{domain}/{service}"), data)
            .await
    }

    /// Fire an event
    pub async fn fire_event(&self, event_type: &str, data: &Value) -> Result<Value> {
        self.post(&format!("/events/{event_type}"), data).await
    }

    /// Render a template
    pub async fn render_template(&self, template: &str) -> Result<String> {
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
}

// --- API Types ---

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
}
