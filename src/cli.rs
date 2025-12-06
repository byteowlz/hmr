//! CLI argument parsing and command definitions

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

/// A slim, fast CLI for Home Assistant
#[derive(Debug, Parser)]
#[command(
    name = "hmr",
    author,
    version,
    about = "A slim, fast CLI for Home Assistant",
    propagate_version = true,
    after_help = "Use 'hmr <command> --help' for more information about a command."
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalOpts,
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Global options available to all commands
#[derive(Debug, Clone, Args)]
pub struct GlobalOpts {
    /// Output format (json, yaml, table, auto)
    #[arg(
        short = 'o',
        long = "output",
        value_enum,
        global = true,
        conflicts_with = "json"
    )]
    pub output_format: Option<OutputFormat>,

    /// Output as JSON (shorthand for -o json)
    #[arg(long, global = true)]
    pub json: bool,

    /// Home Assistant server URL
    #[arg(short = 's', long, env = "HASS_SERVER", global = true)]
    pub server: Option<String>,

    /// Authentication token
    #[arg(long, env = "HASS_TOKEN", global = true, hide_env_values = true)]
    pub token: Option<String>,

    /// Request timeout in seconds
    #[arg(long, global = true)]
    pub timeout: Option<u64>,

    /// Skip SSL certificate verification
    #[arg(long, global = true)]
    pub insecure: bool,

    /// Override config file path
    #[arg(long, value_name = "PATH", env = "HMR_CONFIG", global = true)]
    pub config: Option<PathBuf>,

    /// Reduce output to only errors
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Increase logging verbosity (stackable: -v, -vv, -vvv)
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Enable debug logging (equivalent to -vv)
    #[arg(long, global = true)]
    pub debug: bool,

    /// Enable trace logging
    #[arg(long, global = true)]
    pub trace: bool,

    /// Disable colored output
    #[arg(long = "no-color", global = true)]
    pub no_color: bool,

    /// Custom table columns (comma-separated)
    #[arg(long, value_name = "COLUMNS", global = true)]
    pub columns: Option<String>,

    /// Hide table headers
    #[arg(long, global = true)]
    pub no_headers: bool,

    /// Sort table output by field
    #[arg(long, value_name = "FIELD", global = true)]
    pub sort_by: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[non_exhaustive]
pub enum OutputFormat {
    Json,
    Yaml,
    Table,
    Auto,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Display Home Assistant instance information
    Info,

    /// Manage entities
    Entity {
        #[command(subcommand)]
        command: EntityCommand,
    },

    /// Manage services
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },

    /// Manage events
    Event {
        #[command(subcommand)]
        command: EventCommand,
    },

    /// Render Jinja2 templates server-side
    Template(TemplateCommand),

    /// Manage areas
    Area {
        #[command(subcommand)]
        command: AreaCommand,
    },

    /// Manage devices
    Device {
        #[command(subcommand)]
        command: DeviceCommand,
    },

    /// Inspect and manage configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },

    /// Generate shell completions
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Debug, Subcommand)]
pub enum EntityCommand {
    /// List entities with optional filter
    List {
        /// Filter by entity_id or friendly_name (fuzzy match)
        filter: Option<String>,
    },

    /// Get detailed entity state
    Get {
        /// Entity ID (e.g., light.kitchen)
        entity_id: String,
    },

    /// Update entity state
    Set {
        /// Entity ID to update
        entity_id: String,

        /// JSON data for state and attributes
        #[arg(long = "data", value_name = "JSON", conflicts_with = "state")]
        data: Option<String>,

        /// Quick state update
        #[arg(long)]
        state: Option<String>,
    },

    /// Get entity history
    History {
        /// Entity ID
        entity_id: String,

        /// Time duration (e.g., "2h", "1d", "30m")
        #[arg(long, default_value = "1h")]
        since: String,
    },

    /// Watch entity state changes in real-time (WebSocket)
    Watch {
        /// Entity IDs to watch
        #[arg(required = true)]
        entity_ids: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ServiceCommand {
    /// List available services
    List {
        /// Filter by domain (e.g., "light", "switch")
        domain: Option<String>,
    },

    /// Call a service
    Call {
        /// Service to call (e.g., light.turn_on)
        service: String,

        /// JSON data for service call
        #[arg(long = "data", value_name = "JSON")]
        data: Option<String>,

        /// Key=value pairs for simple service calls
        #[arg(value_name = "KEY=VALUE")]
        args: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum EventCommand {
    /// Watch events in real-time (WebSocket)
    Watch {
        /// Event type to filter (e.g., state_changed)
        event_type: Option<String>,
    },

    /// Fire a custom event
    Fire {
        /// Event type to fire
        event_type: String,

        /// JSON data for event payload
        #[arg(long = "data", value_name = "JSON")]
        data: Option<String>,
    },
}

#[derive(Debug, Args)]
pub struct TemplateCommand {
    /// Template string to render
    #[arg(value_name = "TEMPLATE", conflicts_with = "file")]
    pub template: Option<String>,

    /// Read template from file
    #[arg(long, value_name = "FILE")]
    pub file: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum AreaCommand {
    /// List all areas
    List,

    /// Create a new area
    Create {
        /// Area name
        name: String,

        /// JSON metadata for the area
        #[arg(long = "data", value_name = "JSON")]
        data: Option<String>,
    },

    /// Delete an area
    Delete {
        /// Area name or ID
        name: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum DeviceCommand {
    /// List all devices
    List,

    /// Assign a device to an area
    Assign {
        /// Area name or ID
        area: String,

        /// Device ID
        device: String,
    },

    /// Update device metadata
    Update {
        /// Device ID
        device_id: String,

        /// JSON data for device update (can also be piped via stdin)
        #[arg(long = "data", value_name = "JSON")]
        data: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Show effective configuration
    Show,

    /// Print config file path
    Path,

    /// Get a specific configuration value
    Get {
        /// Configuration key (dot-separated path)
        key: Option<String>,
    },

    /// Reset configuration to defaults
    Reset,
}
