# PRD: Home Assistant CLI (hmr)

## Overview

A slim, fast command-line interface for Home Assistant written in Rust. Provides essential functionality to interact with local or remote Home Assistant instances from the terminal.

## Goals

- Fast, single-binary CLI tool with minimal dependencies
- Core functionality for daily Home Assistant operations
- Clean, intuitive command structure
- Cross-platform support (Linux, macOS, Windows 11+)
- XDG-compliant configuration and data storage

## Non-Goals

- Feature parity with Python hass-cli (focus on essential operations)
- GUI or TUI interface
- Plugin system
- Docker image distribution
- Home Assistant OS/Supervisor management features

## Core Features

### Authentication & Connection

- Bearer token authentication via environment variable or config file
- Server URL configuration with auto-discovery support
- SSL/TLS with optional certificate validation bypass
- Client certificate support
- Configurable timeout for network operations

### Entity Management

**Commands:**

- `hmr entity list [FILTER]` - List entities with optional filter (REST)
- `hmr entity get <ENTITY_ID>` - Get detailed entity state (REST)
- `hmr entity watch <ENTITY_ID...>` - Watch entity state changes in real-time (WebSocket)
- `hmr entity history <ENTITY_ID> --since <DURATION>` - Get entity history (REST)
- `hmr entity set <ENTITY_ID> --json <JSON>` - Update entity state and attributes via JSON (REST)
- `hmr entity set <ENTITY_ID> --state <STATE>` - Quick state update (REST)

**Features:**

- Table, JSON, and YAML output formats
- Customizable table columns via `--columns` flag
- Sorting support via `--sort-by` flag
- Fuzzy filtering by entity_id or friendly_name
- Real-time state monitoring with WebSocket for efficient watching

### Service Management

**Commands:**

- `hmr service list [DOMAIN_FILTER]` - List available services
- `hmr service call <SERVICE> --json <JSON>` - Call service with JSON data
- `hmr service call <SERVICE> [KEY=VALUE...]` - Call service with key-value pairs

**Features:**

- JSON input via `--json` flag for complex service data
- Simple key=value syntax for basic calls
- Read JSON from stdin with `--json -`
- Service discovery with domain filtering
- Detailed service descriptions in YAML format

### Events

**Commands:**

- `hmr event watch [EVENT_TYPE]` - Subscribe and watch events in real-time (WebSocket)
- `hmr event fire <EVENT_TYPE> [--json <JSON>]` - Fire a custom event (REST)

**Features:**

- Filter by event type
- Live streaming output
- JSON data payload for custom events

### System Information

**Commands:**

- `hmr info` - Display Home Assistant instance information
- `hmr config get [KEY]` - Get configuration details

### Template Rendering

**Commands:**

- `hmr template <TEMPLATE_STRING>` - Render Jinja2 template server-side
- `hmr template --file <FILE>` - Render template from file

### Area & Device Registry

**Commands:**

- `hmr area list` - List all areas
- `hmr area create <NAME> [--json <JSON>]` - Create new area with optional metadata
- `hmr area delete <NAME>` - Delete area
- `hmr device list` - List all devices
- `hmr device assign <AREA> <DEVICE>` - Assign device to area
- `hmr device update <DEVICE_ID> --json <JSON>` - Update device metadata

## Configuration

### File Location

- Primary: `$XDG_CONFIG_HOME/hmr/config.toml` (Unix) or `%APPDATA%\hmr\config.toml` (Windows)
- Override with `--config <PATH>` or `HMR_CONFIG` environment variable

### Configuration Schema

```toml
[homeassistant]
server = "http://homeassistant.local:8123"
token = ""  # Prefer environment variable
timeout = 5
insecure = false
cert_path = ""

[websocket]
# Auto-reconnect settings for watch commands
reconnect = true
reconnect_delay = 5  # seconds
max_reconnect_attempts = 0  # 0 = infinite

[output]
format = "auto"  # auto, json, yaml, table
table_format = "simple"
no_headers = false

[logging]
level = "info"  # trace, debug, info, warn, error
```

### Environment Variables

- `HASS_SERVER` / `HMR_SERVER` - Home Assistant server URL
- `HASS_TOKEN` / `HMR_TOKEN` - Bearer token for authentication
- `HMR_CONFIG` - Custom config file path
- `HMR__*` - Override any config value (e.g., `HMR__LOGGING__LEVEL=debug`)

## Global Flags

All commands support:

- `-o, --output <FORMAT>` - Output format (json, yaml, table, auto)
- `--json <JSON>` - Provide JSON input (inline string, file path, or `-` for stdin)
- `-s, --server <URL>` - Server URL
- `--token <TOKEN>` - Authentication token
- `--timeout <SECONDS>` - Request timeout
- `--insecure` - Skip SSL certificate verification
- `--cert <PATH>` - Client certificate path
- `-v, --verbose` - Increase verbosity (stackable)
- `-q, --quiet` - Suppress non-essential output
- `--debug` - Enable debug logging
- `--no-color` - Disable colored output
- `--columns <SPEC>` - Custom table columns
- `--no-headers` - Hide table headers
- `--sort-by <FIELD>` - Sort table output

## Output Formats

### Table Format

Default for interactive terminals. Uses `tabled` or similar crate for clean ASCII tables.

```
ENTITY_ID              STATE    LAST_CHANGED
light.kitchen          on       2025-01-15T10:30:00Z
sensor.temperature     21.5     2025-01-15T10:25:00Z
```

### JSON Format

Machine-readable single JSON object or array. When used with `--json` flag for input, supports:

- Inline JSON string: `--json '{"state": "on"}'`
- File path: `--json @/path/to/data.json`
- Stdin: `--json -` (reads from pipe or stdin)

**Examples:**

```bash
# Inline JSON
hmr service call light.turn_on --json '{"entity_id": "light.kitchen", "brightness": 255}'

# From file
hmr entity set sensor.custom --json @sensor-data.json

# From stdin
echo '{"state": "on"}' | hmr entity set light.living_room --json -

# Output as JSON
hmr entity list --output json
```

### YAML Format

Human-readable YAML output for complex nested data.

## Shell Completion

Generate completions via:

```bash
hmr completions bash > hmr.bash
hmr completions zsh > _hmr
hmr completions fish > hmr.fish
```

## Error Handling

- Clear error messages with actionable suggestions
- HTTP error code translation to user-friendly messages
- Network timeout handling with retry hints
- WebSocket disconnection handling with auto-reconnect for watch commands
- Handle Home Assistant restarts gracefully (reconnect with backoff)
- Respect `NO_COLOR` environment variable
- Exit codes: 0 (success), 1 (general error), 2 (usage error), 130 (SIGINT/Ctrl+C)

## Usage Examples

### REST API Commands (Quick operations)

```bash
# Get system info
hmr info

# List all lights
hmr entity list light

# Turn on a light with brightness
hmr service call light.turn_on --json '{"entity_id": "light.kitchen", "brightness": 200}'

# Get entity state
hmr entity get sensor.temperature

# Update entity state
hmr entity set sensor.custom --json '{"state": "42", "attributes": {"unit": "°C"}}'

# View entity history
hmr entity history light.bedroom --since 2h
```

### WebSocket Commands (Streaming/watching)

```bash
# Watch all events in real-time
hmr event watch

# Watch specific event type
hmr event watch state_changed

# Monitor multiple entities for changes
hmr entity watch light.kitchen light.bedroom sensor.temperature

# Watch with JSON output for piping
hmr entity watch sensor.temperature -o json | jq '.new_state.state'
```

### Composability Examples

```bash
# Find all unavailable entities
hmr entity list -o json | jq '.[] | select(.state == "unavailable") | .entity_id'

# Call service with output from another command
hmr entity get climate.living_room -o json | jq '{entity_id: .entity_id, temperature: 22}' | hmr service call climate.set_temperature --json -

# Watch events and filter with jq
hmr event watch -o json | jq 'select(.event_type == "automation_triggered")'
```

## JSON Input/Output Philosophy

The `--json` flag serves dual purposes:

### As Input

- Primary method for passing structured data to commands
- Three input modes: inline string, file (`@path`), or stdin (`-`)
- Validates JSON before making API calls
- Falls back to key=value arguments for simple cases

### As Output

- Via `--output json` or `-o json` flag
- Pretty-printed by default in interactive terminals
- Compact single-line output when piped
- Enables composability with tools like `jq`

## API Communication Strategy

### REST API (via `homeassistant-rs`)

Used for one-off commands and simple queries where establishing a persistent connection is inefficient:

- Entity queries (list, get, history)
- State updates
- Service calls
- System information
- Configuration queries
- Area/device management

**Advantages:** Simple, stateless, lower overhead for single operations

### WebSocket API (via `hass-rs`)

Used for real-time streaming and persistent monitoring where continuous updates are needed:

- Event watching (`hmr event watch`)
- Entity state monitoring (`hmr entity watch`)
- Live subscriptions

**Advantages:** Efficient for continuous updates, lower latency, push-based updates

The CLI automatically selects the appropriate protocol based on the command.

## Dependencies

Core crates:

- `homeassistant-rs` (0.1.3) - Home Assistant REST API client
- `hass-rs` (0.4.1) - Home Assistant WebSocket API client
- `clap` (4.5+) - CLI framework with derive macros
- `config` (0.15+) - Configuration management with TOML support
- `serde` + `serde_json` + `serde_yaml` - Serialization formats
- `tokio` (1.36+) - Async runtime
- `anyhow` - Error handling and context
- `tabled` - Table formatting for output
- `tokio-tungstenite` - WebSocket support (via hass-rs)

## Implementation Priority

### Phase 1 (MVP - REST API only)

- Authentication and connection management
- Basic entity operations (list, get)
- Service call functionality
- System info command
- Configuration file support
- JSON input/output handling
- Table formatting with custom columns

### Phase 2 (Add WebSocket support)

- WebSocket client integration
- Event watching (`hmr event watch`)
- Entity state monitoring (`hmr entity watch`)
- Entity history
- Template rendering
- Event firing

### Phase 3 (Polish & extend)

- Area and device management
- Advanced filtering and sorting
- Shell completion generation
- Performance optimizations
- Connection pooling and reuse strategies
- Extended output formats

## Success Criteria

- Single binary under 10MB
- Sub-100ms startup time for simple commands
- REST commands execute within network latency + 50ms overhead
- WebSocket commands connect within 500ms and stream indefinitely
- Zero runtime dependencies
- Config file created on first run with sensible defaults
- Works with Home Assistant Core and Home Assistant OS
- Clean shutdown of WebSocket connections (no zombie connections)
- Graceful handling of HA restarts during watch operations
