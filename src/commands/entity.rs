//! Entity command implementations

use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use serde::Serialize;
use serde_json::json;
use tabled::Tabled;

use crate::api::{EntityState, HassClient};
use crate::cli::{EntityCommand, OutputFormat};
use crate::config::RuntimeContext;
use crate::output::{get_json_input, output_for_format, print_output, print_table};
use crate::websocket;

#[derive(Debug, Tabled, Serialize)]
struct EntityRow {
    entity_id: String,
    state: String,
    #[tabled(rename = "friendly_name")]
    friendly_name: String,
    last_changed: String,
}

impl From<&EntityState> for EntityRow {
    fn from(state: &EntityState) -> Self {
        let friendly_name = state
            .attributes
            .get("friendly_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Format the timestamp for display
        let last_changed = state
            .last_changed
            .split('.')
            .next()
            .unwrap_or(&state.last_changed);

        Self {
            entity_id: state.entity_id.clone(),
            state: state.state.clone(),
            friendly_name,
            last_changed: last_changed.to_string(),
        }
    }
}

pub async fn run(ctx: &RuntimeContext, command: EntityCommand) -> Result<()> {
    match command {
        EntityCommand::List { filter } => list(ctx, filter).await,
        EntityCommand::Get { entity_id } => get(ctx, &entity_id).await,
        EntityCommand::Set {
            entity_id,
            data,
            state,
        } => set(ctx, &entity_id, data.as_deref(), state.as_deref()).await,
        EntityCommand::History { entity_id, since } => history(ctx, &entity_id, &since).await,
        EntityCommand::Watch { entity_ids } => watch(ctx, &entity_ids).await,
    }
}

async fn list(ctx: &RuntimeContext, filter: Option<String>) -> Result<()> {
    let client = HassClient::new(ctx)?;
    // Note: Home Assistant API doesn't support server-side filtering, so we must
    // load all entities and filter client-side. For large installations, this is
    // the only option without caching or a local database.
    let states = client.get_states().await?;

    let filtered: Vec<_> = if let Some(ref filter) = filter {
        let matcher = SkimMatcherV2::default();
        states
            .iter()
            .filter(|s| {
                let friendly = s
                    .attributes
                    .get("friendly_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                matcher.fuzzy_match(&s.entity_id, filter).is_some()
                    || matcher.fuzzy_match(friendly, filter).is_some()
            })
            .collect()
    } else {
        states.iter().collect()
    };

    output_for_format(ctx, &filtered, || {
        let rows: Vec<EntityRow> = filtered.iter().map(|s| EntityRow::from(*s)).collect();
        if rows.is_empty() {
            if filter.is_some() {
                println!("No entities found matching filter");
            } else {
                println!("No entities found");
            }
        } else {
            print_table(ctx, &rows)?;
        }
        Ok(())
    })
}

async fn get(ctx: &RuntimeContext, entity_id: &str) -> Result<()> {
    let client = HassClient::new(ctx)?;
    let state = client.get_state(entity_id).await?;

    output_for_format(ctx, &state, || {
        println!("Entity: {}", state.entity_id);
        println!("State:  {}", state.state);
        println!();
        println!("Attributes:");
        if let Some(attrs) = state.attributes.as_object() {
            for (key, value) in attrs {
                println!("  {}: {}", key, value);
            }
        }
        println!();
        println!("Last Changed: {}", state.last_changed);
        println!("Last Updated: {}", state.last_updated);
        Ok(())
    })
}

async fn set(
    ctx: &RuntimeContext,
    entity_id: &str,
    data_input: Option<&str>,
    state_input: Option<&str>,
) -> Result<()> {
    let client = HassClient::new(ctx)?;

    // Build data: prefer explicit --data, then --state, then piped stdin
    let data = if let Some(json_value) = get_json_input(data_input).context("parsing JSON input")? {
        json_value
    } else if let Some(state) = state_input {
        json!({ "state": state })
    } else {
        anyhow::bail!("Either --data, --state, or piped JSON input must be provided");
    };

    // For controllable entities (lights, switches, etc.), use service calls instead of direct state updates
    // Direct state updates only modify the state database without triggering device actions
    if let Some(state_str) = state_input {
        if let Some((domain, service_name)) = map_state_to_service(entity_id, state_str) {
            log::debug!(
                "Using service call {}.{} instead of direct state update",
                domain,
                service_name
            );

            let service_data = json!({ "entity_id": entity_id });
            let result = client
                .call_service(&domain, &service_name, &service_data)
                .await?;
            print_output(ctx, &result)?;
            return Ok(());
        }
    }

    // Fall back to direct state update for non-controllable entities (sensors, etc.)
    log::debug!("Using direct state update for {}", entity_id);
    let result = client.set_state(entity_id, &data).await?;
    print_output(ctx, &result)?;

    Ok(())
}

/// Maps entity_id domain and desired state to the appropriate service call.
/// Returns (domain, service_name) if a service call should be used, None otherwise.
fn map_state_to_service(entity_id: &str, desired_state: &str) -> Option<(String, String)> {
    let domain = entity_id.split('.').next()?;
    let state_lower = desired_state.to_lowercase();

    match domain {
        "light" | "switch" | "fan" | "cover" | "lock" | "media_player" => {
            let service = match state_lower.as_str() {
                "on" | "true" | "1" => "turn_on",
                "off" | "false" | "0" => "turn_off",
                "toggle" => "toggle",
                "open" if domain == "cover" => "open_cover",
                "close" | "closed" if domain == "cover" => "close_cover",
                "lock" | "locked" if domain == "lock" => "lock",
                "unlock" | "unlocked" if domain == "lock" => "unlock",
                "play" if domain == "media_player" => "media_play",
                "pause" if domain == "media_player" => "media_pause",
                "stop" if domain == "media_player" => "media_stop",
                _ => return None,
            };
            Some((domain.to_string(), service.to_string()))
        }
        _ => None, // Sensors and other non-controllable entities use direct state updates
    }
}

async fn history(ctx: &RuntimeContext, entity_id: &str, since: &str) -> Result<()> {
    let client = HassClient::new(ctx)?;

    // Parse duration string (e.g., "2h", "1d", "30m")
    let duration = parse_duration(since)?;
    let start_time = Utc::now() - duration;
    let start_str = start_time.format("%Y-%m-%dT%H:%M:%S").to_string();

    let history = client.get_history(entity_id, &start_str).await?;

    output_for_format(ctx, &history, || {
        if history.is_empty() || history[0].is_empty() {
            println!("No history found for {} in the last {}", entity_id, since);
        } else {
            let rows: Vec<EntityRow> = history[0].iter().map(EntityRow::from).collect();
            print_table(ctx, &rows)?;
        }
        Ok(())
    })
}

async fn watch(ctx: &RuntimeContext, entity_ids: &[String]) -> Result<()> {
    println!("Watching entities: {}", entity_ids.join(", "));
    println!("Press Ctrl+C to stop\n");

    let output_format = ctx.output_format();

    websocket::watch_entities(ctx, entity_ids, |data| {
        match output_format {
            OutputFormat::Json => {
                println!("{}", serde_json::to_string(data)?);
            }
            OutputFormat::Yaml => {
                println!("{}", serde_yaml::to_string(data)?);
            }
            OutputFormat::Table | OutputFormat::Auto => {
                let entity_id = data
                    .get("entity_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let new_state = data
                    .get("new_state")
                    .and_then(|v| v.get("state"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let old_state = data
                    .get("old_state")
                    .and_then(|v| v.get("state"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");

                println!("{}: {} -> {}", entity_id, old_state, new_state);
            }
        }
        Ok(true) // Continue watching
    })
    .await
}

fn parse_duration(s: &str) -> Result<Duration> {
    let duration =
        humantime::parse_duration(s).with_context(|| format!("parsing duration '{s}'"))?;

    Ok(Duration::from_std(duration)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert!(parse_duration("1h").is_ok());
        assert!(parse_duration("2h30m").is_ok());
        assert!(parse_duration("1d").is_ok());
        assert!(parse_duration("invalid").is_err());
    }

    #[test]
    fn test_entity_row_from_state() {
        let state = EntityState {
            entity_id: "light.kitchen".to_string(),
            state: "on".to_string(),
            attributes: serde_json::json!({"friendly_name": "Kitchen Light"}),
            last_changed: "2025-01-15T10:30:00.123456+00:00".to_string(),
            last_updated: "2025-01-15T10:30:00.123456+00:00".to_string(),
            context: serde_json::Value::Null,
        };

        let row = EntityRow::from(&state);
        assert_eq!(row.entity_id, "light.kitchen");
        assert_eq!(row.state, "on");
        assert_eq!(row.friendly_name, "Kitchen Light");
    }

    #[test]
    fn test_map_state_to_service_light() {
        assert_eq!(
            map_state_to_service("light.kitchen", "on"),
            Some(("light".to_string(), "turn_on".to_string()))
        );
        assert_eq!(
            map_state_to_service("light.bedroom", "off"),
            Some(("light".to_string(), "turn_off".to_string()))
        );
        assert_eq!(
            map_state_to_service("light.spots", "toggle"),
            Some(("light".to_string(), "toggle".to_string()))
        );
    }

    #[test]
    fn test_map_state_to_service_switch() {
        assert_eq!(
            map_state_to_service("switch.outlet", "on"),
            Some(("switch".to_string(), "turn_on".to_string()))
        );
        assert_eq!(
            map_state_to_service("switch.outlet", "off"),
            Some(("switch".to_string(), "turn_off".to_string()))
        );
    }

    #[test]
    fn test_map_state_to_service_cover() {
        assert_eq!(
            map_state_to_service("cover.garage", "open"),
            Some(("cover".to_string(), "open_cover".to_string()))
        );
        assert_eq!(
            map_state_to_service("cover.garage", "close"),
            Some(("cover".to_string(), "close_cover".to_string()))
        );
    }

    #[test]
    fn test_map_state_to_service_lock() {
        assert_eq!(
            map_state_to_service("lock.front_door", "lock"),
            Some(("lock".to_string(), "lock".to_string()))
        );
        assert_eq!(
            map_state_to_service("lock.front_door", "unlock"),
            Some(("lock".to_string(), "unlock".to_string()))
        );
    }

    #[test]
    fn test_map_state_to_service_sensor() {
        // Sensors are not controllable, should return None
        assert_eq!(map_state_to_service("sensor.temperature", "25"), None);
        assert_eq!(map_state_to_service("binary_sensor.motion", "on"), None);
    }

    #[test]
    fn test_map_state_to_service_case_insensitive() {
        assert_eq!(
            map_state_to_service("light.kitchen", "ON"),
            Some(("light".to_string(), "turn_on".to_string()))
        );
        assert_eq!(
            map_state_to_service("light.kitchen", "Off"),
            Some(("light".to_string(), "turn_off".to_string()))
        );
    }
}
