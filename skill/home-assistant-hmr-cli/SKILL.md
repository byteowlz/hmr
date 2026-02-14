---
name: home-assistant-hmr-cli
description: |
  Control Home Assistant from the terminal using hmr (homer), a fast Rust CLI.
  Use this skill when users ask to control smart home devices, check entity states,
  manage areas/devices, call services, watch events, render templates, or interact
  with Home Assistant in any way via the command line. Triggers on: "turn on the lights",
  "what's the temperature", "list entities", "call a service", "watch events",
  "Home Assistant", "smart home", "hmr", "homer".
---

# hmr - Home Assistant CLI

Fast, scriptable CLI for Home Assistant. Supports natural language commands, fuzzy entity matching, real-time event streaming, and machine-readable output.

Binary: `hmr`. Always use `--json` for programmatic output.

Priority: CLI flags > env vars (`HASS_SERVER`, `HASS_TOKEN`, `HMR__*`) > config file.

## Natural Language Commands (Recommended for Agents)

The `do` command (alias: `run`) parses natural language locally -- no LLM required.

```bash
# Basic control
hmr do turn on kitchen light
hmr do turn off all living room lights
hmr do toggle bedroom fan

# Dimming and brightness
hmr do dim office light to 50%
hmr do brighten hallway to 80%
hmr do set bedroom light to 25%

# Colors
hmr do set living room lights to blue
hmr do set desk lamp to red

# Covers and locks
hmr do open garage door
hmr do close living room blinds
hmr do lock front door
hmr do unlock back door

# Media
hmr do volume up living room speaker
hmr do mute kitchen speaker

# Dry run (preview without executing)
hmr do turn on all lights --dry-run

# Skip confirmation
hmr do turn off everything --yes

# Exact matching (disable fuzzy)
hmr do turn on light.kitchen_main --exact
```

Supported actions: on, off, toggle, open, close, dim, brighten, set, volume up, volume down, mute, unmute.

Fuzzy matching resolves partial names, typos, and plurals automatically.

## Entity Management

```bash
# List entities (all or filtered by domain)
hmr entity list
hmr entity list light
hmr entity list sensor
hmr entity list --json

# Get entity state
hmr entity get light.kitchen
hmr entity get sensor.outdoor_temperature --json

# Set entity state (maps to appropriate service call)
hmr entity set light.kitchen on
hmr entity set light.bedroom off
hmr entity set cover.garage open

# Entity state history
hmr entity history sensor.temperature
hmr entity history sensor.temperature --start "2h ago"

# Watch entity state changes in real-time
hmr entity watch light.kitchen
hmr entity watch sensor.temperature --json
```

## Services

```bash
# List available services
hmr service list
hmr service list light
hmr service list --json

# Call a service with key=value arguments
hmr service call light.turn_on entity_id=light.kitchen brightness=255
hmr service call light.turn_on entity_id=light.bedroom color_name=blue

# Call with JSON data
hmr service call climate.set_temperature '{"entity_id":"climate.living_room","temperature":22}'

# Call with JSON from file
hmr service call automation.trigger @payload.json

# Call with JSON from stdin
echo '{"entity_id":"light.all"}' | hmr service call light.turn_on
```

## Events

```bash
# Watch all events
hmr event watch

# Watch specific event type
hmr event watch state_changed
hmr event watch automation_triggered --json

# Fire a custom event
hmr event fire my_custom_event '{"key":"value"}'
```

## Areas and Devices

```bash
# Areas
hmr area list
hmr area list --json
hmr area create "Guest Room"
hmr area delete "Guest Room"

# Devices
hmr device list
hmr device list --json
hmr device assign <device-id> "Living Room"
hmr device update <device-id> --name "New Name"
```

## Templates (Jinja2)

```bash
# Render a template
hmr template "{{ states('sensor.temperature') }}"
hmr template "{{ state_attr('light.kitchen', 'brightness') }}"

# From file
hmr template --file template.j2

# From stdin
echo "{{ now() }}" | hmr template
```

## Conversation Agent (Server-Side NLP)

```bash
# Ask Home Assistant's built-in conversation agent
hmr agent what is the temperature in the living room
hmr ask turn on the kitchen lights

# Specify language
hmr agent --lang de Wie warm ist es im Wohnzimmer

# Continue a conversation
hmr agent --conversation-id abc123 and the bedroom
```

## Instance Info

```bash
hmr info
hmr info --json
```

## Cache Management

```bash
hmr cache status          # show cache freshness
hmr cache refresh         # force refresh all caches
hmr cache clear           # clear all cached data
hmr cache path            # show cache directory
hmr cache entity-info light.kitchen   # cached entity details
hmr cache area-info "Kitchen"         # cached area details
```

## Command History

```bash
hmr history list          # recent commands
hmr history again         # repeat last command
hmr history stats         # match accuracy statistics
hmr history context       # current context (for follow-up commands)
hmr history clear         # clear history
hmr history compact       # compact history file
hmr history path          # show history file path
```

Context memory (5 min TTL) enables follow-up commands: after `hmr do dim kitchen light`, saying `hmr do brighter` applies to the same entity.

## Natural Argument Normalization

hmr accepts natural word order. These are equivalent:

```bash
hmr list entities    # => hmr entity list
hmr entities list    # => hmr entity list
hmr show services    # => hmr service list
hmr show info        # => hmr info
```

## Global Flags

| Flag                                  | Description                                          |
| ------------------------------------- | ---------------------------------------------------- |
| `-o/--output json\|yaml\|table\|auto` | Output format (auto = table on TTY, JSON when piped) |
| `--json`                              | Shorthand for `-o json`                              |
| `-s/--server URL`                     | Override HA server URL                               |
| `--token TOKEN`                       | Override auth token                                  |
| `--timeout SECS`                      | Request timeout (default: 30)                        |
| `--insecure`                          | Skip TLS verification                                |
| `--config PATH`                       | Override config file path                            |
| `-q/--quiet`                          | Suppress non-essential output                        |
| `-v/-vv/-vvv`                         | Increase verbosity                                   |
| `--debug`                             | Debug logging                                        |
| `--trace`                             | Trace logging                                        |
| `--no-color`                          | Disable colored output                               |
| `--columns COL1,COL2`                 | Select table columns                                 |
| `--no-headers`                        | Hide table headers                                   |
| `--sort-by COLUMN`                    | Sort table output                                    |

## Agent Patterns

### Check state then act

```bash
STATE=$(hmr entity get sensor.door_sensor --json | jq -r '.state')
if [ "$STATE" = "open" ]; then
  hmr do lock front door --yes
fi
```

### Batch operations with JSON piping

```bash
# Get all lights, filter those that are on, turn them off
hmr entity list light --json | jq -r '.[] | select(.state=="on") | .entity_id' | \
  xargs -I{} hmr service call light.turn_off entity_id={}
```

### Monitor and react

```bash
# Watch for motion, trigger automation
hmr entity watch binary_sensor.motion --json | \
  jq --unbuffered 'select(.new_state.state=="on")' | \
  while read -r event; do
    hmr do turn on hallway light --yes
  done
```

### Dry run before executing

```bash
hmr do turn off all lights --dry-run   # preview what would happen
hmr do turn off all lights --yes       # execute without confirmation
```
