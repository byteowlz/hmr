//! Configuration management for hmr
//!
//! Supports:
//! - TOML config file at XDG locations
//! - Environment variable overrides
//! - Command-line argument overrides

use std::env;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use config::{Config, Environment, File, FileFormat};
use env_logger::fmt::WriteStyle;
use log::LevelFilter;
use serde::{Deserialize, Serialize};

use crate::cli::{GlobalOpts, OutputFormat};

const APP_NAME: &str = env!("CARGO_PKG_NAME");

/// Runtime context containing resolved configuration
#[derive(Debug, Clone)]
pub struct RuntimeContext {
    pub global: GlobalOpts,
    pub config: AppConfig,
    config_path: PathBuf,
}

impl RuntimeContext {
    pub fn new(global: &GlobalOpts) -> Result<Self> {
        let config_path = resolve_config_path(global.config.as_ref())?;
        let config = load_config(&config_path, global)?;

        Ok(Self {
            global: global.clone(),
            config,
            config_path,
        })
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub fn init_logging(&self) -> Result<()> {
        if self.global.quiet {
            log::set_max_level(LevelFilter::Off);
            return Ok(());
        }

        let mut builder = env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or(&self.config.logging.level),
        );

        builder.filter_level(self.effective_log_level());

        let force_color = env::var_os("FORCE_COLOR").is_some();
        let disable_color = self.global.no_color
            || env::var_os("NO_COLOR").is_some()
            || (!force_color && !std::io::stderr().is_terminal());

        if disable_color {
            builder.write_style(WriteStyle::Never);
        } else if force_color {
            builder.write_style(WriteStyle::Always);
        } else {
            builder.write_style(WriteStyle::Auto);
        }

        builder.try_init().or_else(|err| {
            if self.global.verbose > 0 {
                eprintln!("logger already initialized: {err}");
            }
            Ok(())
        })
    }

    fn effective_log_level(&self) -> LevelFilter {
        if self.global.trace {
            LevelFilter::Trace
        } else if self.global.debug {
            LevelFilter::Debug
        } else {
            match self.global.verbose {
                0 => LevelFilter::Warn,
                1 => LevelFilter::Info,
                2 => LevelFilter::Debug,
                _ => LevelFilter::Trace,
            }
        }
    }

    /// Get the effective server URL
    pub fn server_url(&self) -> Result<&str> {
        self.global
            .server
            .as_deref()
            .or(self.config.homeassistant.server.as_deref())
            .ok_or_else(|| {
                anyhow!(
                    "No Home Assistant server configured.\n\
                    Set via --server, HASS_SERVER env var, or in config file."
                )
            })
    }

    /// Get the effective auth token
    pub fn token(&self) -> Result<&str> {
        self.global
            .token
            .as_deref()
            .or(self.config.homeassistant.token.as_deref())
            .ok_or_else(|| {
                anyhow!(
                    "No authentication token configured.\n\
                    Set via --token, HASS_TOKEN env var, or in config file."
                )
            })
    }

    /// Get the effective timeout in seconds
    pub fn timeout(&self) -> u64 {
        self.global
            .timeout
            .unwrap_or(self.config.homeassistant.timeout)
    }

    /// Check if SSL verification should be skipped
    pub fn insecure(&self) -> bool {
        self.global.insecure || self.config.homeassistant.insecure
    }

    /// Get the effective output format
    pub fn output_format(&self) -> OutputFormat {
        self.global
            .output_format
            .unwrap_or(match self.config.output.format.as_str() {
                "json" => OutputFormat::Json,
                "yaml" => OutputFormat::Yaml,
                "table" => OutputFormat::Table,
                _ => OutputFormat::Auto,
            })
    }

    /// Check if output should be in table format
    #[allow(dead_code)]
    pub fn is_table_output(&self) -> bool {
        matches!(
            self.output_format(),
            OutputFormat::Table | OutputFormat::Auto
        ) && std::io::stdout().is_terminal()
    }
}

/// Application configuration structure
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub homeassistant: HomeAssistantConfig,
    pub websocket: WebSocketConfig,
    pub output: OutputConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HomeAssistantConfig {
    pub server: Option<String>,
    pub token: Option<String>,
    pub timeout: u64,
    pub insecure: bool,
    pub cert_path: Option<String>,
}

impl Default for HomeAssistantConfig {
    fn default() -> Self {
        Self {
            server: None,
            token: None,
            timeout: 30,
            insecure: false,
            cert_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebSocketConfig {
    pub reconnect: bool,
    pub reconnect_delay: u64,
    pub max_reconnect_attempts: u32,
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            reconnect: true,
            reconnect_delay: 5,
            max_reconnect_attempts: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputConfig {
    pub format: String,
    pub table_format: String,
    pub no_headers: bool,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            format: "auto".to_string(),
            table_format: "simple".to_string(),
            no_headers: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "warn".to_string(),
        }
    }
}

fn resolve_config_path(override_path: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        let expanded = expand_path(path)?;
        if expanded.is_dir() {
            return Ok(expanded.join("config.toml"));
        }
        return Ok(expanded);
    }

    Ok(default_config_dir()?.join("config.toml"))
}

fn load_config(config_path: &Path, global: &GlobalOpts) -> Result<AppConfig> {
    // Create default config if it doesn't exist
    if !config_path.exists() {
        write_default_config(config_path)?;
    }

    let config = Config::builder()
        // Set defaults
        .set_default("homeassistant.timeout", 30_i64)?
        .set_default("homeassistant.insecure", false)?
        .set_default("websocket.reconnect", true)?
        .set_default("websocket.reconnect_delay", 5_i64)?
        .set_default("websocket.max_reconnect_attempts", 0_i64)?
        .set_default("output.format", "auto")?
        .set_default("output.table_format", "simple")?
        .set_default("output.no_headers", false)?
        .set_default("logging.level", "warn")?
        // Load from file
        .add_source(
            File::from(config_path)
                .format(FileFormat::Toml)
                .required(false),
        )
        // Environment variable overrides (HASS_* and HMR__*)
        .add_source(
            Environment::with_prefix("HASS")
                .try_parsing(true)
                .separator("_"),
        )
        .add_source(
            Environment::with_prefix("HMR")
                .try_parsing(true)
                .separator("__"),
        )
        .build()?;

    let mut app_config: AppConfig = config.try_deserialize()?;

    // Apply CLI overrides
    if global.no_headers {
        app_config.output.no_headers = true;
    }

    Ok(app_config)
}

pub fn write_default_config(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating config directory {}", parent.display()))?;
    }

    let config = AppConfig::default();
    let toml = toml::to_string_pretty(&config).context("serializing default config")?;

    let content = format!(
        "# hmr configuration\n\
        # File: {}\n\
        #\n\
        # Environment variables:\n\
        #   HASS_SERVER - Home Assistant server URL\n\
        #   HASS_TOKEN  - Authentication token\n\
        #   HMR__*      - Override any config value (e.g., HMR__LOGGING__LEVEL=debug)\n\
        \n\
        {toml}",
        path.display()
    );

    fs::write(path, content).with_context(|| format!("writing config to {}", path.display()))
}

fn expand_path(path: &Path) -> Result<PathBuf> {
    if let Some(text) = path.to_str() {
        let expanded = shellexpand::full(text).context("expanding path")?;
        Ok(PathBuf::from(expanded.to_string()))
    } else {
        Ok(path.to_path_buf())
    }
}

fn default_config_dir() -> Result<PathBuf> {
    // Check XDG_CONFIG_HOME first
    if let Some(dir) = env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir).join(APP_NAME));
    }

    // Use platform-specific config directory
    if let Some(mut dir) = dirs::config_dir() {
        dir.push(APP_NAME);
        return Ok(dir);
    }

    // Fallback to ~/.config
    dirs::home_dir()
        .map(|home| home.join(".config").join(APP_NAME))
        .ok_or_else(|| anyhow!("unable to determine configuration directory"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.homeassistant.timeout, 30);
        assert!(!config.homeassistant.insecure);
        assert!(config.websocket.reconnect);
        assert_eq!(config.output.format, "auto");
    }

    #[test]
    fn test_config_serialization() {
        let config = AppConfig::default();
        let toml = toml::to_string_pretty(&config).unwrap();
        assert!(toml.contains("[homeassistant]"));
        assert!(toml.contains("[websocket]"));
        assert!(toml.contains("[output]"));
        assert!(toml.contains("[logging]"));
    }
}
