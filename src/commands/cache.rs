//! Cache management commands

use anyhow::Result;
use tabled::{Table, Tabled};

use crate::cache::{cache_dir, cache_status, clear_cache, CacheManager};
use crate::cli::{CacheCommand, OutputFormat};
use crate::config::RuntimeContext;
use crate::output::print_output;

/// Execute cache commands
pub async fn execute(ctx: &RuntimeContext, command: CacheCommand) -> Result<()> {
    match command {
        CacheCommand::Status => status(ctx).await,
        CacheCommand::Refresh {
            all,
            entities,
            areas,
            services,
            devices,
        } => refresh(ctx, all, entities, areas, services, devices).await,
        CacheCommand::Clear => clear(ctx),
        CacheCommand::Path => path(ctx),
    }
}

async fn status(ctx: &RuntimeContext) -> Result<()> {
    let server_url = ctx.server_url().unwrap_or("unknown");
    let status = cache_status(server_url)?;

    match ctx.output_format() {
        OutputFormat::Json => {
            print_output(ctx, &status)?;
        }
        OutputFormat::Yaml => {
            println!("{}", serde_yaml::to_string(&status)?);
        }
        _ => {
            println!("Cache directory: {}", status.cache_dir.display());
            println!("Server: {}", server_url);
            println!();

            #[derive(Tabled)]
            struct CacheRow {
                #[tabled(rename = "Type")]
                cache_type: String,
                #[tabled(rename = "Items")]
                items: String,
                #[tabled(rename = "Size")]
                size: String,
                #[tabled(rename = "Age")]
                age: String,
                #[tabled(rename = "Expires In")]
                expires: String,
                #[tabled(rename = "Status")]
                status: String,
            }

            let mut rows = Vec::new();

            // Entities
            if let Some(ref e) = status.entities {
                let is_current_server = e.server_url == server_url;
                rows.push(CacheRow {
                    cache_type: "Entities".to_string(),
                    items: format_count(&status, "entities"),
                    size: format_bytes(e.size_bytes),
                    age: format_duration(e.age_secs),
                    expires: e
                        .expires_in_secs
                        .map(format_duration)
                        .unwrap_or_else(|| "expired".to_string()),
                    status: if is_current_server {
                        "valid".to_string()
                    } else {
                        "different server".to_string()
                    },
                });
            } else {
                rows.push(CacheRow {
                    cache_type: "Entities".to_string(),
                    items: "-".to_string(),
                    size: "-".to_string(),
                    age: "-".to_string(),
                    expires: "-".to_string(),
                    status: "not cached".to_string(),
                });
            }

            // Areas
            if let Some(ref a) = status.areas {
                let is_current_server = a.server_url == server_url;
                rows.push(CacheRow {
                    cache_type: "Areas".to_string(),
                    items: format_count(&status, "areas"),
                    size: format_bytes(a.size_bytes),
                    age: format_duration(a.age_secs),
                    expires: a
                        .expires_in_secs
                        .map(format_duration)
                        .unwrap_or_else(|| "expired".to_string()),
                    status: if is_current_server {
                        "valid".to_string()
                    } else {
                        "different server".to_string()
                    },
                });
            } else {
                rows.push(CacheRow {
                    cache_type: "Areas".to_string(),
                    items: "-".to_string(),
                    size: "-".to_string(),
                    age: "-".to_string(),
                    expires: "-".to_string(),
                    status: "not cached".to_string(),
                });
            }

            // Services
            if let Some(ref s) = status.services {
                let is_current_server = s.server_url == server_url;
                rows.push(CacheRow {
                    cache_type: "Services".to_string(),
                    items: format_count(&status, "services"),
                    size: format_bytes(s.size_bytes),
                    age: format_duration(s.age_secs),
                    expires: s
                        .expires_in_secs
                        .map(format_duration)
                        .unwrap_or_else(|| "expired".to_string()),
                    status: if is_current_server {
                        "valid".to_string()
                    } else {
                        "different server".to_string()
                    },
                });
            } else {
                rows.push(CacheRow {
                    cache_type: "Services".to_string(),
                    items: "-".to_string(),
                    size: "-".to_string(),
                    age: "-".to_string(),
                    expires: "-".to_string(),
                    status: "not cached".to_string(),
                });
            }

            // Devices
            if let Some(ref d) = status.devices {
                let is_current_server = d.server_url == server_url;
                rows.push(CacheRow {
                    cache_type: "Devices".to_string(),
                    items: format_count(&status, "devices"),
                    size: format_bytes(d.size_bytes),
                    age: format_duration(d.age_secs),
                    expires: d
                        .expires_in_secs
                        .map(format_duration)
                        .unwrap_or_else(|| "expired".to_string()),
                    status: if is_current_server {
                        "valid".to_string()
                    } else {
                        "different server".to_string()
                    },
                });
            } else {
                rows.push(CacheRow {
                    cache_type: "Devices".to_string(),
                    items: "-".to_string(),
                    size: "-".to_string(),
                    age: "-".to_string(),
                    expires: "-".to_string(),
                    status: "not cached".to_string(),
                });
            }

            let table = Table::new(rows).to_string();
            println!("{table}");
            println!();
            println!("Total size: {}", format_bytes(status.total_size_bytes));
        }
    }

    Ok(())
}

async fn refresh(
    ctx: &RuntimeContext,
    all: bool,
    entities: bool,
    areas: bool,
    services: bool,
    devices: bool,
) -> Result<()> {
    let mut manager = CacheManager::new(ctx)?;

    // If no specific flags, refresh all
    let refresh_all = all || (!entities && !areas && !services && !devices);

    if refresh_all {
        println!("Refreshing all caches...");
        manager.refresh_all().await?;
        println!("All caches refreshed.");
    } else {
        if entities {
            println!("Refreshing entities...");
            manager.refresh_entities().await?;
            println!(
                "Entities refreshed: {} items",
                manager.cache().entities().len()
            );
        }
        if areas {
            println!("Refreshing areas...");
            manager.refresh_areas().await?;
            println!("Areas refreshed: {} items", manager.cache().areas().len());
        }
        if services {
            println!("Refreshing services...");
            manager.refresh_services().await?;
            println!(
                "Services refreshed: {} items",
                manager.cache().services().len()
            );
        }
        if devices {
            println!("Refreshing devices...");
            manager.refresh_devices().await?;
            println!(
                "Devices refreshed: {} items",
                manager.cache().devices().len()
            );
        }
    }

    Ok(())
}

fn clear(ctx: &RuntimeContext) -> Result<()> {
    let dir = cache_dir()?;

    if !dir.exists() {
        if !ctx.global.quiet {
            println!("Cache directory does not exist: {}", dir.display());
        }
        return Ok(());
    }

    clear_cache()?;

    if !ctx.global.quiet {
        println!("Cache cleared: {}", dir.display());
    }

    Ok(())
}

fn path(ctx: &RuntimeContext) -> Result<()> {
    let dir = cache_dir()?;

    match ctx.output_format() {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::json!({ "path": dir.display().to_string() })
            );
        }
        OutputFormat::Yaml => {
            println!("path: {}", dir.display());
        }
        _ => {
            println!("{}", dir.display());
        }
    }

    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

fn format_count(status: &crate::cache::CacheStatus, cache_type: &str) -> String {
    // We need to load the cache to get counts, so for now show "?"
    // The actual count will be shown after loading
    match cache_type {
        "entities" => status
            .entities
            .as_ref()
            .map(|_| "?".to_string())
            .unwrap_or("-".to_string()),
        "areas" => status
            .areas
            .as_ref()
            .map(|_| "?".to_string())
            .unwrap_or("-".to_string()),
        "services" => status
            .services
            .as_ref()
            .map(|_| "?".to_string())
            .unwrap_or("-".to_string()),
        "devices" => status
            .devices
            .as_ref()
            .map(|_| "?".to_string())
            .unwrap_or("-".to_string()),
        _ => "-".to_string(),
    }
}
