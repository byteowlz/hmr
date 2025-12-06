//! Event command implementations

use anyhow::{Context, Result};

use crate::api::HassClient;
use crate::cli::{EventCommand, OutputFormat};
use crate::config::RuntimeContext;
use crate::output::{get_json_input, output_for_format};
use crate::websocket;

pub async fn run(ctx: &RuntimeContext, command: EventCommand) -> Result<()> {
    match command {
        EventCommand::Watch { event_type } => watch(ctx, event_type.as_deref()).await,
        EventCommand::Fire { event_type, json } => fire(ctx, &event_type, json.as_deref()).await,
    }
}

async fn watch(ctx: &RuntimeContext, event_type: Option<&str>) -> Result<()> {
    if let Some(et) = event_type {
        println!("Watching events of type: {}", et);
    } else {
        println!("Watching all events");
    }
    println!("Press Ctrl+C to stop\n");

    let output_format = ctx.output_format();

    websocket::watch_events(ctx, event_type, |event| {
        match output_format {
            OutputFormat::Json => {
                println!("{}", serde_json::to_string(event)?);
            }
            OutputFormat::Yaml => {
                println!("{}", serde_yaml::to_string(event)?);
            }
            OutputFormat::Table | OutputFormat::Auto => {
                println!(
                    "[{}] {} ({})",
                    event
                        .time_fired
                        .split('.')
                        .next()
                        .unwrap_or(&event.time_fired),
                    event.event_type,
                    event.origin
                );
                if !event.data.is_null() && event.data != serde_json::json!({}) {
                    // Print compact data summary
                    if let Some(entity_id) = event.data.get("entity_id").and_then(|v| v.as_str()) {
                        println!("  entity: {}", entity_id);
                    }
                    if let Some(domain) = event.data.get("domain").and_then(|v| v.as_str()) {
                        println!("  domain: {}", domain);
                    }
                    if let Some(service) = event.data.get("service").and_then(|v| v.as_str()) {
                        println!("  service: {}", service);
                    }
                }
            }
        }
        Ok(true) // Continue watching
    })
    .await
}

async fn fire(ctx: &RuntimeContext, event_type: &str, json_input: Option<&str>) -> Result<()> {
    let client = HassClient::new(ctx)?;

    // Build data: prefer explicit --json, then piped stdin, otherwise empty object
    let data = get_json_input(json_input)
        .context("parsing JSON input")?
        .unwrap_or_else(|| serde_json::json!({}));

    log::debug!("Firing event {} with data: {:?}", event_type, data);

    let result = client.fire_event(event_type, &data).await?;

    output_for_format(ctx, &result, || {
        println!("Event '{}' fired successfully", event_type);
        Ok(())
    })
}
