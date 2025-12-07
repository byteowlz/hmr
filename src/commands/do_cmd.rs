//! Natural language command execution

use anyhow::{anyhow, Result};

use crate::api::HassClient;
use crate::cache::CacheManager;
use crate::cli::{DoCommand, OutputFormat};
use crate::config::RuntimeContext;
use crate::history::{History, HistoryEntry};
use crate::nl::NLParser;
use crate::output::print_output;

/// Execute a natural language command
pub async fn execute(ctx: &RuntimeContext, cmd: DoCommand) -> Result<()> {
    // Build the input string from words
    let input = cmd.words.join(" ");

    if input.is_empty() {
        return Err(anyhow!("No command provided"));
    }

    // Load cache (refresh if needed)
    let mut cache_manager = CacheManager::new(ctx)?;

    // Ensure we have cached entities for matching
    if !cache_manager.cache().has_entities() {
        if !ctx.global.quiet {
            eprintln!("Refreshing entity cache...");
        }
        cache_manager.refresh_entities().await?;
    }

    // Parse the natural language input
    let parser = NLParser::new();
    let parsed = parser.parse(&input, cache_manager.cache())?;

    // Handle output formats
    match ctx.output_format() {
        OutputFormat::Json => {
            if cmd.dry_run {
                print_output(ctx, &parsed)?;
                return Ok(());
            }

            // Convert to service call and output
            let service_call = parsed.to_service_call()?;
            print_output(ctx, &service_call)?;

            if !cmd.dry_run {
                execute_service_call(ctx, &service_call).await?;
                record_success(ctx, &input, &parsed, &service_call)?;
            }
            return Ok(());
        }
        OutputFormat::Yaml => {
            if cmd.dry_run {
                println!("{}", serde_yaml::to_string(&parsed)?);
                return Ok(());
            }
        }
        _ => {}
    }

    // Check if we have actionable results
    if parsed.targets.is_empty() {
        record_failure(&input, "No matching entities found")?;
        return Err(anyhow!(
            "Could not find any matching entities for: {}\n\
            Try refreshing the cache with: hmr cache refresh",
            input
        ));
    }

    // Show interpretation
    if !ctx.global.quiet {
        println!("Interpreted as: {}", parsed.interpretation);

        if !parsed.notes.is_empty() {
            for note in &parsed.notes {
                println!("Note: {}", note);
            }
        }
    }

    // Show what would be done
    if !ctx.global.quiet || cmd.dry_run {
        println!();
        println!("Targets ({}):", parsed.targets.len());
        for target in &parsed.targets {
            let name = target.friendly_name.as_deref().unwrap_or(&target.entity_id);
            println!("  {} ({})", target.entity_id, name);
        }

        if !parsed.parameters.is_empty() {
            println!();
            println!("Parameters:");
            for (key, value) in &parsed.parameters {
                println!("  {}: {}", key, value);
            }
        }
    }

    // Dry run stops here
    if cmd.dry_run {
        println!();
        println!("(dry run - no action taken)");
        return Ok(());
    }

    // Execute the service call
    let service_call = parsed.to_service_call()?;

    if !ctx.global.quiet {
        println!();
        println!(
            "Calling {}.{} on {} entities...",
            service_call.domain,
            service_call.service,
            service_call.target.entity_id.len()
        );
    }

    match execute_service_call(ctx, &service_call).await {
        Ok(()) => {
            record_success(ctx, &input, &parsed, &service_call)?;
            if !ctx.global.quiet {
                println!("Done.");
            }
        }
        Err(e) => {
            record_failure(&input, &e.to_string())?;
            return Err(e);
        }
    }

    Ok(())
}

async fn execute_service_call(ctx: &RuntimeContext, call: &crate::nl::ServiceCall) -> Result<()> {
    let client = HassClient::new(ctx)?;

    // Build the service data
    let mut data = serde_json::Map::new();

    // Add entity_id to target (HA REST API style)
    if call.target.entity_id.len() == 1 {
        data.insert(
            "entity_id".to_string(),
            serde_json::Value::String(call.target.entity_id[0].clone()),
        );
    } else {
        data.insert(
            "entity_id".to_string(),
            serde_json::Value::Array(
                call.target
                    .entity_id
                    .iter()
                    .map(|s| serde_json::Value::String(s.clone()))
                    .collect(),
            ),
        );
    }

    // Add any additional data
    for (key, value) in &call.data {
        data.insert(key.clone(), value.clone());
    }

    let body = serde_json::Value::Object(data);

    client
        .call_service(&call.domain, &call.service, &body)
        .await?;

    Ok(())
}

fn record_success(
    _ctx: &RuntimeContext,
    input: &str,
    parsed: &crate::nl::ParsedCommand,
    service_call: &crate::nl::ServiceCall,
) -> Result<()> {
    let mut history = History::new()?;

    // Create history entry
    let entry = HistoryEntry::new(input, &parsed.interpretation)
        .with_service(&service_call.domain, &service_call.service)
        .with_targets(service_call.target.entity_id.clone())
        .with_success();

    history.append(&entry)?;

    // Update context
    let domain = parsed
        .targets
        .first()
        .map(|t| t.entity_id.split('.').next().unwrap_or("").to_string());

    history.update_context(
        service_call.target.entity_id.clone(),
        None, // TODO: extract area from parsed
        domain,
        parsed.action.clone(),
    )?;

    // Update stats
    history.stats_mut().record_exact(); // TODO: track actual match type
    for entity in &service_call.target.entity_id {
        history.stats_mut().record_entity_use(entity);
    }
    history.save_stats()?;

    Ok(())
}

fn record_failure(input: &str, error: &str) -> Result<()> {
    let history = History::new()?;

    let entry = HistoryEntry::new(input, "").with_error(error);

    history.append(&entry)?;

    Ok(())
}
