//! Entity and metadata caching for hmr
//!
//! Caches Home Assistant data locally for:
//! - Fast fuzzy matching without network calls
//! - Offline entity name suggestions
//! - Reduced API load
//!
//! Cache is stored at XDG_CACHE_HOME/hmr/ with configurable TTL.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::api::{EntityState, HassClient};
use crate::config::RuntimeContext;
use crate::websocket::{Area, Device, WsClient};

const APP_NAME: &str = env!("CARGO_PKG_NAME");

/// Default TTL values in seconds
pub mod ttl {
    /// Entity states - refresh frequently (5 minutes)
    pub const ENTITIES: u64 = 300;
    /// Areas - relatively static (1 hour)
    pub const AREAS: u64 = 3600;
    /// Services - rarely changes (1 hour)
    pub const SERVICES: u64 = 3600;
    /// Devices - relatively static (1 hour)
    pub const DEVICES: u64 = 3600;
}

/// Cached entity data with metadata for fuzzy matching
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedEntity {
    pub entity_id: String,
    pub domain: String,
    pub object_id: String,
    pub state: String,
    pub friendly_name: Option<String>,
    pub area_id: Option<String>,
    /// All searchable names for this entity
    pub search_names: Vec<String>,
}

impl From<&EntityState> for CachedEntity {
    fn from(state: &EntityState) -> Self {
        let parts: Vec<&str> = state.entity_id.split('.').collect();
        let (domain, object_id) = if parts.len() == 2 {
            (parts[0].to_string(), parts[1].to_string())
        } else {
            (String::new(), state.entity_id.clone())
        };

        let friendly_name = state
            .attributes
            .get("friendly_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let area_id = state
            .attributes
            .get("area_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Build search names for fuzzy matching
        let mut search_names = vec![state.entity_id.clone(), object_id.clone()];
        if let Some(ref name) = friendly_name {
            search_names.push(name.clone());
            // Also add lowercase and underscore versions
            search_names.push(name.to_lowercase());
            search_names.push(name.to_lowercase().replace(' ', "_"));
        }

        Self {
            entity_id: state.entity_id.clone(),
            domain,
            object_id,
            state: state.state.clone(),
            friendly_name,
            area_id,
            search_names,
        }
    }
}

/// Cached area with search names
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedArea {
    pub area_id: String,
    pub name: String,
    pub aliases: Vec<String>,
    /// All searchable names for this area
    pub search_names: Vec<String>,
}

impl From<&Area> for CachedArea {
    fn from(area: &Area) -> Self {
        let mut search_names = vec![
            area.area_id.clone(),
            area.name.clone(),
            area.name.to_lowercase(),
            area.name.to_lowercase().replace(' ', "_"),
        ];
        for alias in &area.aliases {
            search_names.push(alias.clone());
            search_names.push(alias.to_lowercase());
        }

        Self {
            area_id: area.area_id.clone(),
            name: area.name.clone(),
            aliases: area.aliases.clone(),
            search_names,
        }
    }
}

/// Cached service information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedService {
    pub domain: String,
    pub service: String,
    pub full_name: String,
    pub description: String,
}

/// Cached device with search names
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedDevice {
    pub id: String,
    pub name: Option<String>,
    pub name_by_user: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub area_id: Option<String>,
    /// All searchable names for this device
    pub search_names: Vec<String>,
}

impl From<&Device> for CachedDevice {
    fn from(device: &Device) -> Self {
        let mut search_names = vec![device.id.clone()];
        if let Some(ref name) = device.name {
            search_names.push(name.clone());
            search_names.push(name.to_lowercase());
        }
        if let Some(ref name) = device.name_by_user {
            search_names.push(name.clone());
            search_names.push(name.to_lowercase());
        }

        Self {
            id: device.id.clone(),
            name: device.name.clone(),
            name_by_user: device.name_by_user.clone(),
            manufacturer: device.manufacturer.clone(),
            model: device.model.clone(),
            area_id: device.area_id.clone(),
            search_names,
        }
    }
}

/// A cache file with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheFile<T> {
    /// When the cache was last updated
    pub updated_at: u64,
    /// TTL in seconds
    pub ttl: u64,
    /// Server URL this cache is for (to invalidate on server change)
    pub server_url: String,
    /// The cached data
    pub data: T,
}

impl<T> CacheFile<T> {
    pub fn new(data: T, ttl: u64, server_url: String) -> Self {
        let updated_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            updated_at,
            ttl,
            server_url,
            data,
        }
    }

    pub fn is_valid(&self, server_url: &str) -> bool {
        if self.server_url != server_url {
            return false;
        }

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        now < self.updated_at + self.ttl
    }

    pub fn age(&self) -> Duration {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Duration::from_secs(now.saturating_sub(self.updated_at))
    }

    pub fn expires_in(&self) -> Option<Duration> {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let expires_at = self.updated_at + self.ttl;
        if now < expires_at {
            Some(Duration::from_secs(expires_at - now))
        } else {
            None
        }
    }
}

/// The complete cache holding all cached data
#[derive(Debug, Default)]
pub struct Cache {
    pub entities: Option<CacheFile<Vec<CachedEntity>>>,
    pub areas: Option<CacheFile<Vec<CachedArea>>>,
    pub services: Option<CacheFile<Vec<CachedService>>>,
    pub devices: Option<CacheFile<Vec<CachedDevice>>>,
    /// Lookup maps for fast access
    entity_map: HashMap<String, CachedEntity>,
    area_map: HashMap<String, CachedArea>,
    domain_services: HashMap<String, Vec<String>>,
}

impl Cache {
    /// Create a new empty cache
    pub fn new() -> Self {
        Self::default()
    }

    /// Load cache from disk
    pub fn load(server_url: &str) -> Result<Self> {
        let cache_dir = cache_dir()?;
        let mut cache = Self::new();

        // Load entities
        let entities_path = cache_dir.join("entities.json");
        if entities_path.exists() {
            if let Ok(content) = fs::read_to_string(&entities_path) {
                if let Ok(file) = serde_json::from_str::<CacheFile<Vec<CachedEntity>>>(&content) {
                    if file.is_valid(server_url) {
                        cache.set_entities(file);
                    } else {
                        log::debug!("Entities cache expired or for different server");
                    }
                }
            }
        }

        // Load areas
        let areas_path = cache_dir.join("areas.json");
        if areas_path.exists() {
            if let Ok(content) = fs::read_to_string(&areas_path) {
                if let Ok(file) = serde_json::from_str::<CacheFile<Vec<CachedArea>>>(&content) {
                    if file.is_valid(server_url) {
                        cache.set_areas(file);
                    }
                }
            }
        }

        // Load services
        let services_path = cache_dir.join("services.json");
        if services_path.exists() {
            if let Ok(content) = fs::read_to_string(&services_path) {
                if let Ok(file) = serde_json::from_str::<CacheFile<Vec<CachedService>>>(&content) {
                    if file.is_valid(server_url) {
                        cache.set_services(file);
                    }
                }
            }
        }

        // Load devices
        let devices_path = cache_dir.join("devices.json");
        if devices_path.exists() {
            if let Ok(content) = fs::read_to_string(&devices_path) {
                if let Ok(file) = serde_json::from_str::<CacheFile<Vec<CachedDevice>>>(&content) {
                    if file.is_valid(server_url) {
                        cache.set_devices(file);
                    }
                }
            }
        }

        Ok(cache)
    }

    /// Save cache to disk
    pub fn save(&self) -> Result<()> {
        let cache_dir = cache_dir()?;
        fs::create_dir_all(&cache_dir)
            .with_context(|| format!("creating cache directory {}", cache_dir.display()))?;

        if let Some(ref entities) = self.entities {
            let path = cache_dir.join("entities.json");
            let content = serde_json::to_string_pretty(entities)?;
            fs::write(&path, content)
                .with_context(|| format!("writing entities cache to {}", path.display()))?;
        }

        if let Some(ref areas) = self.areas {
            let path = cache_dir.join("areas.json");
            let content = serde_json::to_string_pretty(areas)?;
            fs::write(&path, content)
                .with_context(|| format!("writing areas cache to {}", path.display()))?;
        }

        if let Some(ref services) = self.services {
            let path = cache_dir.join("services.json");
            let content = serde_json::to_string_pretty(services)?;
            fs::write(&path, content)
                .with_context(|| format!("writing services cache to {}", path.display()))?;
        }

        if let Some(ref devices) = self.devices {
            let path = cache_dir.join("devices.json");
            let content = serde_json::to_string_pretty(devices)?;
            fs::write(&path, content)
                .with_context(|| format!("writing devices cache to {}", path.display()))?;
        }

        Ok(())
    }

    /// Set entities and rebuild lookup maps
    pub fn set_entities(&mut self, file: CacheFile<Vec<CachedEntity>>) {
        self.entity_map.clear();
        for entity in &file.data {
            self.entity_map
                .insert(entity.entity_id.clone(), entity.clone());
        }
        self.entities = Some(file);
    }

    /// Set areas and rebuild lookup maps
    pub fn set_areas(&mut self, file: CacheFile<Vec<CachedArea>>) {
        self.area_map.clear();
        for area in &file.data {
            self.area_map.insert(area.area_id.clone(), area.clone());
        }
        self.areas = Some(file);
    }

    /// Set services and rebuild lookup maps
    pub fn set_services(&mut self, file: CacheFile<Vec<CachedService>>) {
        self.domain_services.clear();
        for service in &file.data {
            self.domain_services
                .entry(service.domain.clone())
                .or_default()
                .push(service.service.clone());
        }
        self.services = Some(file);
    }

    /// Set devices
    pub fn set_devices(&mut self, file: CacheFile<Vec<CachedDevice>>) {
        self.devices = Some(file);
    }

    /// Get an entity by ID
    pub fn get_entity(&self, entity_id: &str) -> Option<&CachedEntity> {
        self.entity_map.get(entity_id)
    }

    /// Get an area by ID
    pub fn get_area(&self, area_id: &str) -> Option<&CachedArea> {
        self.area_map.get(area_id)
    }

    /// Get all entities
    pub fn entities(&self) -> &[CachedEntity] {
        self.entities
            .as_ref()
            .map(|f| f.data.as_slice())
            .unwrap_or(&[])
    }

    /// Get all areas
    pub fn areas(&self) -> &[CachedArea] {
        self.areas
            .as_ref()
            .map(|f| f.data.as_slice())
            .unwrap_or(&[])
    }

    /// Get all services
    pub fn services(&self) -> &[CachedService] {
        self.services
            .as_ref()
            .map(|f| f.data.as_slice())
            .unwrap_or(&[])
    }

    /// Get all devices
    pub fn devices(&self) -> &[CachedDevice] {
        self.devices
            .as_ref()
            .map(|f| f.data.as_slice())
            .unwrap_or(&[])
    }

    /// Get services for a domain
    pub fn services_for_domain(&self, domain: &str) -> &[String] {
        self.domain_services
            .get(domain)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Get entities in a specific domain
    pub fn entities_in_domain(&self, domain: &str) -> Vec<&CachedEntity> {
        self.entities()
            .iter()
            .filter(|e| e.domain == domain)
            .collect()
    }

    /// Get entities in a specific area
    pub fn entities_in_area(&self, area_id: &str) -> Vec<&CachedEntity> {
        self.entities()
            .iter()
            .filter(|e| e.area_id.as_deref() == Some(area_id))
            .collect()
    }

    /// Get all known domains
    pub fn domains(&self) -> Vec<&str> {
        let mut domains: Vec<&str> = self.entities().iter().map(|e| e.domain.as_str()).collect();
        domains.sort();
        domains.dedup();
        domains
    }

    /// Check if cache has valid entities
    pub fn has_entities(&self) -> bool {
        self.entities.is_some()
    }

    /// Check if cache has valid areas
    pub fn has_areas(&self) -> bool {
        self.areas.is_some()
    }

    /// Check if cache has valid services
    pub fn has_services(&self) -> bool {
        self.services.is_some()
    }

    /// Check if cache has valid devices
    pub fn has_devices(&self) -> bool {
        self.devices.is_some()
    }
}

/// Cache manager for refreshing and managing cache
pub struct CacheManager<'a> {
    ctx: &'a RuntimeContext,
    cache: Cache,
}

impl<'a> CacheManager<'a> {
    /// Create a new cache manager
    pub fn new(ctx: &'a RuntimeContext) -> Result<Self> {
        let server_url = ctx.server_url().unwrap_or("");
        let cache = Cache::load(server_url).unwrap_or_default();

        Ok(Self { ctx, cache })
    }

    /// Get the cache
    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    /// Get mutable cache for advanced operations
    /// 
    /// This allows direct modification of cache state for operations that need to:
    /// - Manually update cache entries after operations
    /// - Perform bulk cache modifications
    /// - Implement custom cache management strategies
    /// 
    /// For standard cache updates, prefer using the refresh_* methods.
    /// This is part of the public API for extensibility and plugins.
    pub fn cache_mut(&mut self) -> &mut Cache {
        &mut self.cache
    }

    /// Refresh entities from Home Assistant
    pub async fn refresh_entities(&mut self) -> Result<()> {
        let client = HassClient::new(self.ctx)?;
        let states = client.get_states().await?;

        let cached: Vec<CachedEntity> = states.iter().map(CachedEntity::from).collect();
        let server_url = self.ctx.server_url()?.to_string();

        // Use cache_mut for direct manipulation
        let cache = self.cache_mut();
        cache.set_entities(CacheFile::new(cached, ttl::ENTITIES, server_url));
        cache.save()?;

        log::info!("Refreshed {} entities", self.cache.entities().len());
        Ok(())
    }

    /// Refresh areas from Home Assistant (requires WebSocket)
    pub async fn refresh_areas(&mut self) -> Result<()> {
        let mut ws = WsClient::connect(self.ctx).await?;
        let areas = ws.list_areas().await?;

        let cached: Vec<CachedArea> = areas.iter().map(CachedArea::from).collect();
        let server_url = self.ctx.server_url()?.to_string();

        // Use cache_mut for direct manipulation
        let cache = self.cache_mut();
        cache.set_areas(CacheFile::new(cached, ttl::AREAS, server_url));
        cache.save()?;

        log::info!("Refreshed {} areas", self.cache.areas().len());
        Ok(())
    }

    /// Refresh services from Home Assistant
    pub async fn refresh_services(&mut self) -> Result<()> {
        let client = HassClient::new(self.ctx)?;
        let domains = client.get_services().await?;

        let mut cached = Vec::new();
        for domain in &domains {
            for (service_name, info) in &domain.services {
                cached.push(CachedService {
                    domain: domain.domain.clone(),
                    service: service_name.clone(),
                    full_name: format!("{}.{}", domain.domain, service_name),
                    description: info.description.clone(),
                });
            }
        }

        let server_url = self.ctx.server_url()?.to_string();
        
        // Use cache_mut for direct manipulation
        let cache = self.cache_mut();
        cache.set_services(CacheFile::new(cached, ttl::SERVICES, server_url));
        cache.save()?;

        log::info!("Refreshed {} services", self.cache.services().len());
        Ok(())
    }

    /// Refresh devices from Home Assistant (requires WebSocket)
    pub async fn refresh_devices(&mut self) -> Result<()> {
        let mut ws = WsClient::connect(self.ctx).await?;
        let devices = ws.list_devices().await?;

        let cached: Vec<CachedDevice> = devices.iter().map(CachedDevice::from).collect();
        let server_url = self.ctx.server_url()?.to_string();

        // Use cache_mut for direct manipulation
        let cache = self.cache_mut();
        cache.set_devices(CacheFile::new(cached, ttl::DEVICES, server_url));
        cache.save()?;

        log::info!("Refreshed {} devices", self.cache.devices().len());
        Ok(())
    }

    /// Refresh all caches
    pub async fn refresh_all(&mut self) -> Result<()> {
        // Refresh REST API caches first (entities, services)
        self.refresh_entities().await?;
        self.refresh_services().await?;

        // Then WebSocket-based caches (areas, devices)
        self.refresh_areas().await?;
        self.refresh_devices().await?;

        Ok(())
    }

    /// Ensure entities are cached, refreshing if needed
    pub async fn ensure_entities(&mut self) -> Result<&[CachedEntity]> {
        if !self.cache.has_entities() {
            self.refresh_entities().await?;
        }
        Ok(self.cache.entities())
    }

    /// Ensure areas are cached, refreshing if needed
    pub async fn ensure_areas(&mut self) -> Result<&[CachedArea]> {
        if !self.cache.has_areas() {
            self.refresh_areas().await?;
        }
        Ok(self.cache.areas())
    }

    /// Ensure services are cached, refreshing if needed
    pub async fn ensure_services(&mut self) -> Result<&[CachedService]> {
        if !self.cache.has_services() {
            self.refresh_services().await?;
        }
        Ok(self.cache.services())
    }

    /// Ensure devices are cached, refreshing if needed
    pub async fn ensure_devices(&mut self) -> Result<&[CachedDevice]> {
        if !self.cache.has_devices() {
            self.refresh_devices().await?;
        }
        Ok(self.cache.devices())
    }
}

/// Get the cache directory path
pub fn cache_dir() -> Result<PathBuf> {
    // Check XDG_CACHE_HOME first
    if let Some(dir) = env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir).join(APP_NAME));
    }

    // Use platform-specific cache directory
    if let Some(mut dir) = dirs::cache_dir() {
        dir.push(APP_NAME);
        return Ok(dir);
    }

    // Fallback to ~/.cache
    dirs::home_dir()
        .map(|home| home.join(".cache").join(APP_NAME))
        .ok_or_else(|| anyhow::anyhow!("unable to determine cache directory"))
}

/// Clear all cache files
pub fn clear_cache() -> Result<()> {
    let dir = cache_dir()?;
    if dir.exists() {
        fs::remove_dir_all(&dir)
            .with_context(|| format!("removing cache directory {}", dir.display()))?;
    }
    Ok(())
}

/// Get cache status information
#[derive(Debug, Clone, Serialize)]
pub struct CacheStatus {
    pub cache_dir: PathBuf,
    pub entities: Option<CacheFileStatus>,
    pub areas: Option<CacheFileStatus>,
    pub services: Option<CacheFileStatus>,
    pub devices: Option<CacheFileStatus>,
    pub total_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheFileStatus {
    pub path: PathBuf,
    pub count: usize,
    pub size_bytes: u64,
    pub age_secs: u64,
    pub expires_in_secs: Option<u64>,
    pub server_url: String,
}

/// Get status of all cache files
pub fn cache_status(server_url: &str) -> Result<CacheStatus> {
    let dir = cache_dir()?;
    let mut total_size = 0u64;

    let entities = get_file_status::<Vec<CachedEntity>>(&dir.join("entities.json"), server_url)?;
    let areas = get_file_status::<Vec<CachedArea>>(&dir.join("areas.json"), server_url)?;
    let services = get_file_status::<Vec<CachedService>>(&dir.join("services.json"), server_url)?;
    let devices = get_file_status::<Vec<CachedDevice>>(&dir.join("devices.json"), server_url)?;

    if let Some(ref s) = entities {
        total_size += s.size_bytes;
    }
    if let Some(ref s) = areas {
        total_size += s.size_bytes;
    }
    if let Some(ref s) = services {
        total_size += s.size_bytes;
    }
    if let Some(ref s) = devices {
        total_size += s.size_bytes;
    }

    Ok(CacheStatus {
        cache_dir: dir,
        entities,
        areas,
        services,
        devices,
        total_size_bytes: total_size,
    })
}

fn get_file_status<T: for<'de> Deserialize<'de>>(
    path: &Path,
    _server_url: &str,
) -> Result<Option<CacheFileStatus>> {
    if !path.exists() {
        return Ok(None);
    }

    let metadata = fs::metadata(path)?;
    let content = fs::read_to_string(path)?;
    let file: CacheFile<T> = serde_json::from_str(&content)?;

    Ok(Some(CacheFileStatus {
        path: path.to_path_buf(),
        count: 0, // Will be set by caller based on type
        size_bytes: metadata.len(),
        age_secs: file.age().as_secs(),
        expires_in_secs: file.expires_in().map(|d| d.as_secs()),
        server_url: file.server_url,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_file_validity() {
        let data = vec!["test".to_string()];
        let file = CacheFile::new(data, 60, "http://localhost:8123".to_string());

        assert!(file.is_valid("http://localhost:8123"));
        assert!(!file.is_valid("http://other:8123"));
    }

    #[test]
    fn test_cache_file_expiry() {
        let data = vec!["test".to_string()];
        // Create a file with 0 TTL (already expired)
        let mut file = CacheFile::new(data, 0, "http://localhost:8123".to_string());
        file.updated_at = 0; // Set to epoch (definitely expired)

        assert!(!file.is_valid("http://localhost:8123"));
        assert!(file.expires_in().is_none());
    }

    #[test]
    fn test_cache_file_age() {
        let data = vec!["test".to_string()];
        let file = CacheFile::new(data, 60, "http://localhost:8123".to_string());

        // Age should be very small (just created)
        assert!(file.age().as_secs() < 2);
    }

    #[test]
    fn test_cached_entity_from_state() {
        let state = EntityState {
            entity_id: "light.kitchen".to_string(),
            state: "on".to_string(),
            attributes: serde_json::json!({
                "friendly_name": "Kitchen Light",
                "brightness": 255
            }),
            last_changed: "2025-01-01T00:00:00Z".to_string(),
            last_updated: "2025-01-01T00:00:00Z".to_string(),
            context: serde_json::Value::Null,
        };

        let cached = CachedEntity::from(&state);
        assert_eq!(cached.entity_id, "light.kitchen");
        assert_eq!(cached.domain, "light");
        assert_eq!(cached.object_id, "kitchen");
        assert_eq!(cached.friendly_name, Some("Kitchen Light".to_string()));
        assert!(cached.search_names.contains(&"Kitchen Light".to_string()));
        assert!(cached.search_names.contains(&"kitchen_light".to_string()));
    }

    #[test]
    fn test_cached_entity_without_friendly_name() {
        let state = EntityState {
            entity_id: "sensor.temperature".to_string(),
            state: "22.5".to_string(),
            attributes: serde_json::json!({}),
            last_changed: "2025-01-01T00:00:00Z".to_string(),
            last_updated: "2025-01-01T00:00:00Z".to_string(),
            context: serde_json::Value::Null,
        };

        let cached = CachedEntity::from(&state);
        assert_eq!(cached.entity_id, "sensor.temperature");
        assert_eq!(cached.domain, "sensor");
        assert_eq!(cached.object_id, "temperature");
        assert!(cached.friendly_name.is_none());
        assert!(cached
            .search_names
            .contains(&"sensor.temperature".to_string()));
        assert!(cached.search_names.contains(&"temperature".to_string()));
    }

    #[test]
    fn test_cached_area_from_area() {
        let area = crate::websocket::Area {
            area_id: "kitchen".to_string(),
            name: "Kitchen".to_string(),
            picture: None,
            aliases: vec!["Cooking Area".to_string()],
            icon: None,
            floor_id: None,
            labels: vec![],
        };

        let cached = CachedArea::from(&area);
        assert_eq!(cached.area_id, "kitchen");
        assert_eq!(cached.name, "Kitchen");
        assert!(cached.search_names.contains(&"Kitchen".to_string()));
        assert!(cached.search_names.contains(&"kitchen".to_string()));
        assert!(cached.search_names.contains(&"Cooking Area".to_string()));
    }

    #[test]
    fn test_cached_device_from_device() {
        let device = crate::websocket::Device {
            id: "device123".to_string(),
            area_id: Some("living_room".to_string()),
            configuration_url: None,
            config_entries: vec![],
            connections: vec![],
            disabled_by: None,
            entry_type: None,
            hw_version: None,
            identifiers: vec![],
            manufacturer: Some("Philips".to_string()),
            model: Some("Hue Bulb".to_string()),
            name_by_user: Some("Living Room Lamp".to_string()),
            name: Some("Hue Light".to_string()),
            sw_version: None,
            via_device_id: None,
            labels: vec![],
        };

        let cached = CachedDevice::from(&device);
        assert_eq!(cached.id, "device123");
        assert_eq!(cached.manufacturer, Some("Philips".to_string()));
        assert_eq!(cached.area_id, Some("living_room".to_string()));
        assert!(cached
            .search_names
            .contains(&"Living Room Lamp".to_string()));
        assert!(cached.search_names.contains(&"Hue Light".to_string()));
    }

    #[test]
    fn test_cache_new() {
        let cache = Cache::new();
        assert!(!cache.has_entities());
        assert!(!cache.has_areas());
        assert!(!cache.has_services());
        assert!(!cache.has_devices());
        assert!(cache.entities().is_empty());
        assert!(cache.areas().is_empty());
    }

    #[test]
    fn test_cache_domains() {
        let mut cache = Cache::new();

        // Add some test entities
        let entities = vec![
            CachedEntity {
                entity_id: "light.kitchen".to_string(),
                domain: "light".to_string(),
                object_id: "kitchen".to_string(),
                state: "on".to_string(),
                friendly_name: None,
                area_id: None,
                search_names: vec![],
            },
            CachedEntity {
                entity_id: "light.bedroom".to_string(),
                domain: "light".to_string(),
                object_id: "bedroom".to_string(),
                state: "off".to_string(),
                friendly_name: None,
                area_id: None,
                search_names: vec![],
            },
            CachedEntity {
                entity_id: "switch.outlet".to_string(),
                domain: "switch".to_string(),
                object_id: "outlet".to_string(),
                state: "on".to_string(),
                friendly_name: None,
                area_id: None,
                search_names: vec![],
            },
        ];

        let file = CacheFile::new(entities, 60, "http://localhost:8123".to_string());
        cache.set_entities(file);

        let domains = cache.domains();
        assert!(domains.contains(&"light"));
        assert!(domains.contains(&"switch"));
        assert_eq!(domains.len(), 2);
    }

    #[test]
    fn test_cache_entities_in_domain() {
        let mut cache = Cache::new();

        let entities = vec![
            CachedEntity {
                entity_id: "light.kitchen".to_string(),
                domain: "light".to_string(),
                object_id: "kitchen".to_string(),
                state: "on".to_string(),
                friendly_name: None,
                area_id: None,
                search_names: vec![],
            },
            CachedEntity {
                entity_id: "switch.outlet".to_string(),
                domain: "switch".to_string(),
                object_id: "outlet".to_string(),
                state: "on".to_string(),
                friendly_name: None,
                area_id: None,
                search_names: vec![],
            },
        ];

        let file = CacheFile::new(entities, 60, "http://localhost:8123".to_string());
        cache.set_entities(file);

        let lights = cache.entities_in_domain("light");
        assert_eq!(lights.len(), 1);
        assert_eq!(lights[0].entity_id, "light.kitchen");
    }

    #[test]
    fn test_cache_dir() {
        let dir = cache_dir().unwrap();
        assert!(dir.to_string_lossy().contains("hmr"));
    }

}
