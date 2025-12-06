//! Info command implementation

use anyhow::Result;

use crate::api::HassClient;
use crate::config::RuntimeContext;
use crate::output::output_for_format;

pub async fn run(ctx: &RuntimeContext) -> Result<()> {
    let client = HassClient::new(ctx)?;
    let info = client.get_info().await?;

    output_for_format(ctx, &info, || {
        println!("Home Assistant Information");
        println!("==========================");
        println!("Version:      {}", info.version);
        println!("Location:     {}", info.location_name);
        println!("Time Zone:    {}", info.time_zone);
        println!("State:        {}", info.state);
        println!("Config Dir:   {}", info.config_dir);
        println!("Latitude:     {}", info.latitude);
        println!("Longitude:    {}", info.longitude);
        println!("Elevation:    {} m", info.elevation);
        println!();
        println!("Unit System:");
        println!("  Temperature: {}", info.unit_system.temperature);
        println!("  Length:      {}", info.unit_system.length);
        println!("  Mass:        {}", info.unit_system.mass);
        println!("  Volume:      {}", info.unit_system.volume);
        println!();
        println!("Components:   {} loaded", info.components.len());
        Ok(())
    })
}
