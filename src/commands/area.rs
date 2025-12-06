//! Area command implementations
//!
//! Area management uses the WebSocket API to interact with Home Assistant's
//! area registry for listing, creating, and deleting areas.

use anyhow::{anyhow, Result};
use serde::Serialize;
use tabled::Tabled;

use crate::cli::AreaCommand;
use crate::config::RuntimeContext;
use crate::output;
use crate::websocket::{Area, CreateAreaRequest, WsClient};

pub async fn run(ctx: &RuntimeContext, command: AreaCommand) -> Result<()> {
    match command {
        AreaCommand::List => list(ctx).await,
        AreaCommand::Create { name, data } => create(ctx, &name, data).await,
        AreaCommand::Delete { name } => delete(ctx, &name).await,
    }
}

#[derive(Debug, Clone, Serialize, Tabled)]
struct AreaRow {
    #[tabled(rename = "AREA ID")]
    area_id: String,
    #[tabled(rename = "NAME")]
    name: String,
    #[tabled(rename = "FLOOR")]
    floor: String,
    #[tabled(rename = "ALIASES")]
    aliases: String,
}

impl From<Area> for AreaRow {
    fn from(area: Area) -> Self {
        Self {
            area_id: area.area_id,
            name: area.name,
            floor: area.floor_id.unwrap_or_else(|| "-".to_string()),
            aliases: if area.aliases.is_empty() {
                "-".to_string()
            } else {
                area.aliases.join(", ")
            },
        }
    }
}

async fn list(ctx: &RuntimeContext) -> Result<()> {
    let mut client = WsClient::connect(ctx).await?;
    let areas = client.list_areas().await?;

    // Convert to rows for table display
    let rows: Vec<AreaRow> = areas.into_iter().map(AreaRow::from).collect();

    output::print_table(ctx, &rows)?;
    Ok(())
}

async fn create(ctx: &RuntimeContext, name: &str, data_input: Option<String>) -> Result<()> {
    let mut client = WsClient::connect(ctx).await?;

    // Build request from JSON input if provided, otherwise use just the name
    let request = if let Some(data_str) = data_input {
        let json = output::parse_json_input(&data_str)?;
        let mut req: CreateAreaRequest = serde_json::from_value(json)?;
        // Override name with the positional argument
        req.name = name.to_string();
        req
    } else {
        CreateAreaRequest::new(name.to_string())
    };

    let area = client.create_area(&request).await?;

    output::print_output(ctx, &area)?;
    Ok(())
}

async fn delete(ctx: &RuntimeContext, name: &str) -> Result<()> {
    let mut client = WsClient::connect(ctx).await?;

    // First, list all areas to find the ID by name
    let areas = client.list_areas().await?;
    let area = areas
        .iter()
        .find(|a| a.name == name || a.area_id == name)
        .ok_or_else(|| anyhow!("Area not found: {name}"))?;

    client.delete_area(&area.area_id).await?;

    println!("Area '{}' deleted successfully", area.name);
    Ok(())
}
