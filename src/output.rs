//! Output formatting utilities
//!
//! Handles JSON, YAML, and table output formats.

use std::io::IsTerminal;

use anyhow::Result;
use serde::Serialize;
use tabled::{settings::Style, Table, Tabled};

use crate::cli::OutputFormat;
use crate::config::RuntimeContext;

/// Format and print data according to the configured output format
pub fn print_output<T: Serialize>(ctx: &RuntimeContext, data: &T) -> Result<()> {
    let output = format_output(ctx, data)?;
    println!("{output}");
    Ok(())
}

/// Format data according to the configured output format
pub fn format_output<T: Serialize>(ctx: &RuntimeContext, data: &T) -> Result<String> {
    let format = ctx.output_format();
    let is_tty = std::io::stdout().is_terminal();

    match format {
        OutputFormat::Json => {
            if is_tty {
                Ok(serde_json::to_string_pretty(data)?)
            } else {
                Ok(serde_json::to_string(data)?)
            }
        }
        OutputFormat::Yaml => Ok(serde_yaml::to_string(data)?),
        OutputFormat::Table | OutputFormat::Auto => {
            // For auto, use JSON when piped
            if !is_tty && matches!(format, OutputFormat::Auto) {
                Ok(serde_json::to_string(data)?)
            } else {
                // Fall back to JSON for non-table-compatible types
                Ok(serde_json::to_string_pretty(data)?)
            }
        }
    }
}

/// Print a table from items that implement Tabled
pub fn print_table<T: Tabled>(ctx: &RuntimeContext, items: &[T]) -> Result<()> {
    let format = ctx.output_format();
    let is_tty = std::io::stdout().is_terminal();

    // Use JSON/YAML for non-table formats or when piped with auto
    if matches!(format, OutputFormat::Json) || (matches!(format, OutputFormat::Auto) && !is_tty) {
        // Tabled items might not impl Serialize, so we need to handle this differently
        // For now, just print the table even for JSON requests
        let table = build_table(ctx, items);
        println!("{table}");
        return Ok(());
    }

    if matches!(format, OutputFormat::Yaml) {
        let table = build_table(ctx, items);
        println!("{table}");
        return Ok(());
    }

    let table = build_table(ctx, items);
    println!("{table}");
    Ok(())
}

fn build_table<T: Tabled>(ctx: &RuntimeContext, items: &[T]) -> Table {
    let mut table = Table::new(items);
    table.with(Style::sharp());

    if ctx.global.no_headers || ctx.config.output.no_headers {
        table.with(tabled::settings::Remove::row(
            tabled::settings::object::Rows::first(),
        ));
    }

    table
}

/// Parse JSON input from various sources (inline, file, stdin)
pub fn parse_json_input(input: &str) -> Result<serde_json::Value> {
    let input = input.trim();

    // Check for stdin indicator
    if input == "-" {
        use std::io::Read;
        let mut buffer = String::new();
        std::io::stdin().read_to_string(&mut buffer)?;
        return Ok(serde_json::from_str(&buffer)?);
    }

    // Check for file path indicator
    if let Some(path) = input.strip_prefix('@') {
        let content = std::fs::read_to_string(path)?;
        return Ok(serde_json::from_str(&content)?);
    }

    // Parse as inline JSON
    Ok(serde_json::from_str(input)?)
}

/// Parse key=value pairs into a JSON object
pub fn parse_key_value_args(args: &[String]) -> Result<serde_json::Value> {
    let mut map = serde_json::Map::new();

    for arg in args {
        let (key, value) = arg
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("Invalid argument format: {arg}. Expected KEY=VALUE"))?;

        // Try to parse as JSON value, fall back to string
        let json_value = serde_json::from_str(value)
            .unwrap_or_else(|_| serde_json::Value::String(value.to_string()));

        map.insert(key.to_string(), json_value);
    }

    Ok(serde_json::Value::Object(map))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_value_args() {
        let args = vec![
            "entity_id=light.kitchen".to_string(),
            "brightness=255".to_string(),
            "color_temp=400".to_string(),
        ];

        let result = parse_key_value_args(&args).unwrap();
        assert_eq!(result["entity_id"], "light.kitchen");
        assert_eq!(result["brightness"], 255);
        assert_eq!(result["color_temp"], 400);
    }

    #[test]
    fn test_parse_key_value_with_json() {
        let args = vec![r#"data={"nested": true}"#.to_string()];

        let result = parse_key_value_args(&args).unwrap();
        assert!(result["data"]["nested"].as_bool().unwrap());
    }

    #[test]
    fn test_parse_json_input_inline() {
        let json = r#"{"state": "on"}"#;
        let result = parse_json_input(json).unwrap();
        assert_eq!(result["state"], "on");
    }
}
