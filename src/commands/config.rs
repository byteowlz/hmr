//! Config command implementations

use anyhow::Result;

use crate::cli::ConfigCommand;
use crate::config::{self as app_config, RuntimeContext};
use crate::output::print_output;

pub fn run(ctx: &RuntimeContext, command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Show => show(ctx),
        ConfigCommand::Path => path(ctx),
        ConfigCommand::Get { key } => get(ctx, key.as_deref()),
        ConfigCommand::Reset => reset(ctx),
    }
}

fn show(ctx: &RuntimeContext) -> Result<()> {
    print_output(ctx, &ctx.config)?;
    Ok(())
}

fn path(ctx: &RuntimeContext) -> Result<()> {
    println!("{}", ctx.config_path().display());
    Ok(())
}

fn get(ctx: &RuntimeContext, key: Option<&str>) -> Result<()> {
    if let Some(key) = key {
        // Get a specific key using dot notation
        let value = get_config_value(&ctx.config, key)?;
        println!("{value}");
    } else {
        // Show all config
        show(ctx)?;
    }
    Ok(())
}

fn reset(ctx: &RuntimeContext) -> Result<()> {
    app_config::write_default_config(ctx.config_path())?;
    println!(
        "Configuration reset to defaults at: {}",
        ctx.config_path().display()
    );
    Ok(())
}

fn get_config_value(config: &app_config::AppConfig, key: &str) -> Result<String> {
    // Convert config to JSON for easy traversal
    let json = serde_json::to_value(config)?;

    let parts: Vec<&str> = key.split('.').collect();
    let mut current = &json;

    for part in &parts {
        current = current
            .get(part)
            .ok_or_else(|| anyhow::anyhow!("Configuration key not found: {key}"))?;
    }

    Ok(match current {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "null".to_string(),
        other => serde_json::to_string(other)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn test_get_config_value() {
        let config = AppConfig::default();

        assert_eq!(
            get_config_value(&config, "homeassistant.timeout").unwrap(),
            "30"
        );
        assert_eq!(
            get_config_value(&config, "websocket.reconnect").unwrap(),
            "true"
        );
        assert!(get_config_value(&config, "nonexistent.key").is_err());
    }
}
