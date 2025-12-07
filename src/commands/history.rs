//! History command implementations

use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, Utc};
use tabled::{Table, Tabled};

use crate::cli::{HistoryCommand, OutputFormat};
use crate::config::RuntimeContext;
use crate::history::History;
use crate::output::print_output;

/// Execute history commands
pub async fn execute(ctx: &RuntimeContext, command: HistoryCommand) -> Result<()> {
    match command {
        HistoryCommand::List { limit, filter } => list(ctx, limit, filter),
        HistoryCommand::Again => again(ctx).await,
        HistoryCommand::Context => context(ctx),
        HistoryCommand::ClearContext => clear_context(ctx),
        HistoryCommand::Stats => stats(ctx),
        HistoryCommand::Clear => clear(ctx),
        HistoryCommand::Compact => compact(ctx),
        HistoryCommand::Path => path(ctx),
    }
}

fn list(ctx: &RuntimeContext, limit: usize, filter: Option<String>) -> Result<()> {
    let history = History::new()?;

    let entries = if let Some(ref pattern) = filter {
        history.search(pattern)?
    } else {
        history.recent(limit)?
    };

    if entries.is_empty() {
        if !ctx.global.quiet {
            println!("No history entries found.");
        }
        return Ok(());
    }

    match ctx.output_format() {
        OutputFormat::Json => {
            print_output(ctx, &entries)?;
        }
        OutputFormat::Yaml => {
            println!("{}", serde_yaml::to_string(&entries)?);
        }
        _ => {
            #[derive(Tabled)]
            struct HistoryRow {
                #[tabled(rename = "Time")]
                time: String,
                #[tabled(rename = "Input")]
                input: String,
                #[tabled(rename = "Targets")]
                targets: String,
                #[tabled(rename = "Status")]
                status: String,
            }

            let rows: Vec<HistoryRow> = entries
                .iter()
                .map(|e| {
                    let dt = DateTime::<Utc>::from_timestamp(e.timestamp as i64, 0)
                        .map(|dt| dt.with_timezone(&Local))
                        .map(|dt| dt.format("%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "?".to_string());

                    let targets = if e.targets.is_empty() {
                        "-".to_string()
                    } else if e.targets.len() == 1 {
                        e.targets[0].clone()
                    } else {
                        format!("{} entities", e.targets.len())
                    };

                    let status = if e.success {
                        "ok".to_string()
                    } else if let Some(ref err) = e.error {
                        format!("err: {}", truncate(err, 30))
                    } else {
                        "failed".to_string()
                    };

                    HistoryRow {
                        time: dt,
                        input: truncate(&e.input, 40),
                        targets,
                        status,
                    }
                })
                .collect();

            let table = Table::new(rows).to_string();
            println!("{table}");
        }
    }

    Ok(())
}

async fn again(ctx: &RuntimeContext) -> Result<()> {
    let history = History::new()?;

    let last = history
        .last_entry()?
        .ok_or_else(|| anyhow!("No previous command found"))?;

    if !ctx.global.quiet {
        println!("Repeating: {}", last.input);
    }

    // Re-execute the command through the do handler
    let words: Vec<String> = last.input.split_whitespace().map(String::from).collect();

    let cmd = crate::cli::DoCommand {
        words,
        dry_run: false,
        yes: true,
        exact: false,
    };

    crate::commands::do_cmd::execute(ctx, cmd).await
}

fn context(ctx: &RuntimeContext) -> Result<()> {
    let history = History::new()?;

    match history.context() {
        Some(ctx_data) => match ctx.output_format() {
            OutputFormat::Json => {
                print_output(ctx, ctx_data)?;
            }
            OutputFormat::Yaml => {
                println!("{}", serde_yaml::to_string(ctx_data)?);
            }
            _ => {
                println!("Current context (age: {}s):", ctx_data.age().as_secs());
                println!();

                if !ctx_data.last_entities.is_empty() {
                    println!("Last entities:");
                    for entity in &ctx_data.last_entities {
                        println!("  {}", entity);
                    }
                }

                if let Some(ref area) = ctx_data.last_area {
                    println!("Last area: {}", area);
                }

                if let Some(ref domain) = ctx_data.last_domain {
                    println!("Last domain: {}", domain);
                }

                if let Some(ref action) = ctx_data.last_action {
                    println!("Last action: {}", action);
                }
            }
        },
        None => {
            if !ctx.global.quiet {
                println!("No active context (expired or not set).");
            }
        }
    }

    Ok(())
}

fn clear_context(ctx: &RuntimeContext) -> Result<()> {
    let mut history = History::new()?;
    history.clear_context()?;

    if !ctx.global.quiet {
        println!("Context cleared.");
    }

    Ok(())
}

fn stats(ctx: &RuntimeContext) -> Result<()> {
    let history = History::new()?;
    let stats = history.stats();

    match ctx.output_format() {
        OutputFormat::Json => {
            print_output(ctx, stats)?;
        }
        OutputFormat::Yaml => {
            println!("{}", serde_yaml::to_string(stats)?);
        }
        _ => {
            println!("Command Statistics");
            println!("==================");
            println!();
            println!("Total commands:      {}", stats.total_commands);
            println!("Exact matches:       {}", stats.exact_matches);
            println!("Fuzzy matches:       {}", stats.fuzzy_matches);
            println!("Typo corrections:    {}", stats.typo_corrections);
            println!("Ambiguous prompts:   {}", stats.ambiguous_prompts);
            println!("Failures:            {}", stats.failures);
            println!();
            println!("Success rate:        {:.1}%", stats.success_rate());

            if !stats.correction_map.is_empty() {
                println!();
                println!("Common corrections:");
                for (typo, correction) in stats.correction_map.iter().take(10) {
                    println!("  {} -> {}", typo, correction);
                }
            }

            let top = stats.top_entities(5);
            if !top.is_empty() {
                println!();
                println!("Most used entities:");
                for (entity, count) in top {
                    println!("  {} ({})", entity, count);
                }
            }
        }
    }

    Ok(())
}

fn clear(ctx: &RuntimeContext) -> Result<()> {
    let history = History::new()?;
    history.clear()?;

    if !ctx.global.quiet {
        println!("History cleared.");
    }

    Ok(())
}

fn compact(ctx: &RuntimeContext) -> Result<()> {
    let history = History::new()?;
    let removed = history.compact()?;

    if !ctx.global.quiet {
        if removed > 0 {
            println!("Compacted history: removed {} old entries.", removed);
        } else {
            println!("History is already compact.");
        }
    }

    Ok(())
}

fn path(ctx: &RuntimeContext) -> Result<()> {
    use crate::history::history_path;

    let path = history_path()?;

    match ctx.output_format() {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::json!({ "path": path.display().to_string() })
            );
        }
        OutputFormat::Yaml => {
            println!("path: {}", path.display());
        }
        _ => {
            println!("{}", path.display());
        }
    }

    Ok(())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len <= 3 {
        "...".to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}
