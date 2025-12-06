![banner](banner.png)

# homer (hmr), a home assistant CLI

A slim, fast CLI for Home Assistant.

## Quick Start

1. Install the latest stable Rust toolchain (`rustup default stable`).

2. Build and install:

   ```bash
   cargo install --path .
   ```

3. Configure your Home Assistant connection:

   ```bash
   export HASS_SERVER="http://homeassistant.local:8123"
   export HASS_TOKEN="your-long-lived-access-token"
   ```

4. Verify the connection:

   ```bash
   hmr info
   ```

## Installation

### From Source

```bash
cargo install --path .
```

### Development

```bash
cargo run -- info
```

## Configuration

hmr uses a TOML config file with environment variable overrides.

### Config File Location

- Linux/macOS: `$XDG_CONFIG_HOME/hmr/config.toml` (defaults to `~/.config/hmr/config.toml`)
- Windows: `%APPDATA%\hmr\config.toml`

Override with `--config <path>` or `HMR_CONFIG` env var.

### Environment Variables

| Variable      | Description                                                   |
| ------------- | ------------------------------------------------------------- |
| `HASS_SERVER` | Home Assistant server URL                                     |
| `HASS_TOKEN`  | Long-lived access token                                       |
| `HMR__*`      | Override any config value (e.g., `HMR__LOGGING__LEVEL=debug`) |

### Example Config

See `examples/config.toml` for a complete example with comments.

## Commands

### Entity Management

```bash
hmr entity list                    # List all entities
hmr entity list "light"            # Filter entities (fuzzy match)
hmr entity get light.kitchen       # Get entity details
hmr entity set light.kitchen --state on  # Control devices (automatically calls appropriate service)
hmr entity set switch.outlet --state off
hmr entity set cover.garage --state open
hmr entity history light.kitchen --since 2h
hmr entity watch light.kitchen light.bedroom  # Real-time state changes
```

The `entity set` command intelligently detects controllable entities (lights, switches, fans, covers, locks, media players) and automatically calls the appropriate Home Assistant service. This ensures the physical device is controlled and the state is updated. For sensors and other non-controllable entities, it updates the state directly.

### Service Calls

```bash
hmr service list                   # List all services
hmr service list light             # List services for a domain
hmr service call light.turn_on entity_id=light.kitchen brightness=255
hmr service call light.turn_on --json '{"entity_id": "light.kitchen", "brightness": 255}'
```

Service calls directly invoke Home Assistant services, which both control the physical device and update the entity state.

### Events

```bash
hmr event watch                    # Watch all events
hmr event watch state_changed      # Watch specific event type
hmr event fire my_custom_event --json '{"data": "value"}'
```

### Templates

```bash
hmr template "{{ states('light.kitchen') }}"
hmr template --file my_template.j2
```

### Areas and Devices

```bash
hmr area list
hmr area create "Living Room"
hmr area delete "Living Room"

hmr device list
hmr device assign "Living Room" <device_id>
hmr device update <device_id> --json '{"name_by_user": "My Device"}'
```

### Configuration

```bash
hmr config show                    # Show effective config
hmr config path                    # Print config file path
hmr config get homeassistant.timeout
hmr config reset                   # Reset to defaults
```

### Shell Completions

```bash
hmr completions bash > ~/.local/share/bash-completion/completions/hmr
hmr completions zsh > ~/.zfunc/_hmr
hmr completions fish > ~/.config/fish/completions/hmr.fish
```

## Global Options

| Option                  | Description                                         |
| ----------------------- | --------------------------------------------------- |
| `-o, --output <FORMAT>` | Output format: `json`, `yaml`, `table`, `auto`      |
| `--json`                | Output as JSON (shorthand for `-o json`)            |
| `-s, --server <URL>`    | Home Assistant server URL                           |
| `--token <TOKEN>`       | Authentication token                                |
| `--timeout <SECONDS>`   | Request timeout                                     |
| `--insecure`            | Skip SSL certificate verification                   |
| `--config <PATH>`       | Override config file path                           |
| `-q, --quiet`           | Reduce output to errors only                        |
| `-v, --verbose`         | Increase verbosity (stackable: `-v`, `-vv`, `-vvv`) |
| `--debug`               | Enable debug logging                                |
| `--trace`               | Enable trace logging                                |
| `--no-color`            | Disable colored output                              |
| `--columns <COLS>`      | Custom table columns (comma-separated)              |
| `--no-headers`          | Hide table headers                                  |
| `--sort-by <FIELD>`     | Sort table output by field                          |

## Piping and Scripting

hmr supports Unix-style piping for both input and output.

### JSON Input

Commands that accept JSON data can read from stdin when piped:

```bash
# Pipe JSON data to service call
echo '{"entity_id": "light.kitchen", "brightness": 255}' | hmr service call light.turn_on

# Pipe entity state update
cat state.json | hmr entity set light.kitchen

# Use jq to transform and pipe data
hmr entity get sensor.temperature --json | jq '.attributes' | hmr event fire my_event
```

JSON input can also be provided via:

- `--json '{"key": "value"}'` - inline JSON
- `--json @file.json` - read from file
- `--json -` - explicitly read from stdin

### JSON Output

When piped to another command, hmr automatically outputs compact JSON (with `auto` format):

```bash
# Pipe entity list to jq
hmr entity list | jq '.[].entity_id'

# Chain with other tools
hmr entity list --json | jq -r '.[] | select(.state == "on") | .entity_id'

# Explicit JSON output with --json flag
hmr entity get light.kitchen --json
```

### Template Piping

Templates can be piped via stdin:

```bash
echo '{{ states("light.kitchen") }}' | hmr template
cat complex_template.j2 | hmr template
```

## Development

```bash
cargo fmt                          # Format code
cargo test                         # Run tests
cargo clippy --all-targets         # Lint
cargo run -- --help                # Run in development
```

## License

MIT
