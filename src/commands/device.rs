//! Device command implementations
//!
//! Note: Device management requires the WebSocket API for full functionality.

use anyhow::{anyhow, Result};

use crate::cli::DeviceCommand;
use crate::config::RuntimeContext;
use crate::output::parse_json_input;

pub async fn run(ctx: &RuntimeContext, command: DeviceCommand) -> Result<()> {
    match command {
        DeviceCommand::List => list(ctx).await,
        DeviceCommand::Assign { area, device } => assign(ctx, &area, &device).await,
        DeviceCommand::Update { device_id, json } => update(ctx, &device_id, &json).await,
    }
}

async fn list(_ctx: &RuntimeContext) -> Result<()> {
    Err(anyhow!(
        "Device listing requires WebSocket API commands.\n\
        This feature is planned for a future release.\n\
        \n\
        Workaround: Use 'hmr entity list' to see entities associated with devices."
    ))
}

async fn assign(_ctx: &RuntimeContext, area: &str, device: &str) -> Result<()> {
    Err(anyhow!(
        "Device assignment requires WebSocket API commands.\n\
        This feature is planned for a future release.\n\
        \n\
        Requested: Assign device '{device}' to area '{area}'"
    ))
}

async fn update(_ctx: &RuntimeContext, device_id: &str, json: &str) -> Result<()> {
    // Validate JSON input
    let _data = parse_json_input(json)?;

    Err(anyhow!(
        "Device update requires WebSocket API commands.\n\
        This feature is planned for a future release.\n\
        \n\
        Device to update: {device_id}"
    ))
}
