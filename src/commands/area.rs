//! Area command implementations
//!
//! Note: Area management requires the WebSocket API for full functionality.
//! These commands use WebSocket calls wrapped in REST-like interface.

use anyhow::{anyhow, Result};

use crate::cli::AreaCommand;
use crate::config::RuntimeContext;

pub async fn run(_ctx: &RuntimeContext, command: AreaCommand) -> Result<()> {
    match command {
        AreaCommand::List => list().await,
        AreaCommand::Create { name, json: _ } => create(&name).await,
        AreaCommand::Delete { name } => delete(&name).await,
    }
}

async fn list() -> Result<()> {
    // Area listing requires WebSocket API
    // For now, provide a helpful message
    Err(anyhow!(
        "Area listing requires WebSocket API commands.\n\
        This feature is planned for a future release.\n\
        \n\
        Workaround: Use 'hmr event watch' to see area-related events,\n\
        or access the Home Assistant UI for area management."
    ))
}

async fn create(name: &str) -> Result<()> {
    Err(anyhow!(
        "Area creation requires WebSocket API commands.\n\
        This feature is planned for a future release.\n\
        \n\
        Area name requested: {name}"
    ))
}

async fn delete(name: &str) -> Result<()> {
    Err(anyhow!(
        "Area deletion requires WebSocket API commands.\n\
        This feature is planned for a future release.\n\
        \n\
        Area to delete: {name}"
    ))
}
