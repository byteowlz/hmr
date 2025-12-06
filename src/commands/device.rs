//! Device command implementations
//!
//! Device management uses the WebSocket API to interact with Home Assistant's
//! device registry for listing, assigning to areas, and updating metadata.

use anyhow::{anyhow, Result};
use serde::Serialize;
use tabled::Tabled;

use crate::cli::DeviceCommand;
use crate::config::RuntimeContext;
use crate::output;
use crate::websocket::{Device, UpdateDeviceRequest, WsClient};

pub async fn run(ctx: &RuntimeContext, command: DeviceCommand) -> Result<()> {
    match command {
        DeviceCommand::List => list(ctx).await,
        DeviceCommand::Assign { area, device } => assign(ctx, &area, &device).await,
        DeviceCommand::Update { device_id, data } => update(ctx, &device_id, data.as_deref()).await,
    }
}

#[derive(Debug, Clone, Serialize, Tabled)]
struct DeviceRow {
    #[tabled(rename = "DEVICE ID")]
    id: String,
    #[tabled(rename = "NAME")]
    name: String,
    #[tabled(rename = "MANUFACTURER")]
    manufacturer: String,
    #[tabled(rename = "MODEL")]
    model: String,
    #[tabled(rename = "AREA")]
    area: String,
}

impl From<Device> for DeviceRow {
    fn from(device: Device) -> Self {
        Self {
            id: output::truncate(&device.id, 20),
            name: device
                .name_by_user
                .or(device.name)
                .unwrap_or_else(|| "-".to_string()),
            manufacturer: device.manufacturer.unwrap_or_else(|| "-".to_string()),
            model: device.model.unwrap_or_else(|| "-".to_string()),
            area: device.area_id.unwrap_or_else(|| "-".to_string()),
        }
    }
}

async fn list(ctx: &RuntimeContext) -> Result<()> {
    let mut client = WsClient::connect(ctx).await?;
    let devices = client.list_devices().await?;

    // Convert to rows for table display
    let rows: Vec<DeviceRow> = devices.into_iter().map(DeviceRow::from).collect();

    output::print_table(ctx, &rows)?;
    Ok(())
}

async fn assign(ctx: &RuntimeContext, area: &str, device_id: &str) -> Result<()> {
    let mut client = WsClient::connect(ctx).await?;

    // First, list all areas to find the area ID by name
    let areas = client.list_areas().await?;
    let area_obj = areas
        .iter()
        .find(|a| a.name == area || a.area_id == area)
        .ok_or_else(|| anyhow!("Area not found: {area}"))?;

    // Update the device with the area assignment
    let request =
        UpdateDeviceRequest::new(device_id.to_string()).with_area_id(area_obj.area_id.clone());

    let device = client.update_device(&request).await?;

    println!(
        "Device '{}' assigned to area '{}'",
        device
            .name_by_user
            .or(device.name)
            .unwrap_or_else(|| device_id.to_string()),
        area_obj.name
    );
    Ok(())
}

async fn update(ctx: &RuntimeContext, device_id: &str, data_input: Option<&str>) -> Result<()> {
    let mut client = WsClient::connect(ctx).await?;

    // Validate JSON input (prefer explicit --data, then piped stdin)
    let data = output::get_json_input(data_input)?
        .ok_or_else(|| anyhow!("JSON input required via --data or piped stdin"))?;

    // Parse the update request
    let mut request: UpdateDeviceRequest = serde_json::from_value(data)?;
    // Override device_id with the positional argument
    request.device_id = device_id.to_string();

    let device = client.update_device(&request).await?;

    output::print_output(ctx, &device)?;
    Ok(())
}
