//! Service command implementations

use anyhow::{Context, Result};
use serde::Serialize;
use tabled::Tabled;

use crate::api::HassClient;
use crate::cli::ServiceCommand;
use crate::config::RuntimeContext;
use crate::output::{
    output_for_format, parse_json_input, parse_key_value_args, print_table, truncate,
};

#[derive(Debug, Tabled, Serialize)]
struct ServiceRow {
    domain: String,
    service: String,
    description: String,
}

pub async fn run(ctx: &RuntimeContext, command: ServiceCommand) -> Result<()> {
    match command {
        ServiceCommand::List { domain } => list(ctx, domain.as_deref()).await,
        ServiceCommand::Call {
            service,
            json,
            args,
        } => call(ctx, &service, json.as_deref(), &args).await,
    }
}

async fn list(ctx: &RuntimeContext, domain_filter: Option<&str>) -> Result<()> {
    let client = HassClient::new(ctx)?;
    let services = client.get_services().await?;

    let filtered: Vec<_> = if let Some(filter) = domain_filter {
        services
            .iter()
            .filter(|s| s.domain.contains(filter))
            .collect()
    } else {
        services.iter().collect()
    };

    output_for_format(ctx, &filtered, || {
        let rows: Vec<ServiceRow> = filtered
            .iter()
            .flat_map(|domain| {
                domain.services.iter().map(|(name, info)| ServiceRow {
                    domain: domain.domain.clone(),
                    service: name.clone(),
                    description: truncate(&info.description, 50),
                })
            })
            .collect();

        if rows.is_empty() {
            if domain_filter.is_some() {
                println!("No services found matching filter");
            } else {
                println!("No services found");
            }
        } else {
            print_table(ctx, &rows)?;
        }
        Ok(())
    })
}

async fn call(
    ctx: &RuntimeContext,
    service: &str,
    json_input: Option<&str>,
    args: &[String],
) -> Result<()> {
    let client = HassClient::new(ctx)?;

    // Parse service name (domain.service)
    let (domain, service_name) = service.split_once('.').ok_or_else(|| {
        anyhow::anyhow!(
            "Invalid service format: {service}. Expected format: domain.service (e.g., light.turn_on)"
        )
    })?;

    // Build service data
    let data = if let Some(json_str) = json_input {
        parse_json_input(json_str).context("parsing JSON input")?
    } else if !args.is_empty() {
        parse_key_value_args(args).context("parsing key=value arguments")?
    } else {
        serde_json::json!({})
    };

    log::debug!("Calling {}.{} with data: {:?}", domain, service_name, data);

    let result = client.call_service(domain, service_name, &data).await?;

    output_for_format(ctx, &result, || {
        // For table output, show a success message
        if result.is_array() && !result.as_array().unwrap().is_empty() {
            println!("Service {service} called successfully");
            println!("Affected entities:");
            if let Some(arr) = result.as_array() {
                for entity in arr {
                    if let Some(id) = entity.get("entity_id").and_then(|v| v.as_str()) {
                        let state = entity.get("state").and_then(|v| v.as_str()).unwrap_or("?");
                        println!("  {} -> {}", id, state);
                    }
                }
            }
        } else {
            println!("Service {service} called successfully");
        }
        Ok(())
    })
}
