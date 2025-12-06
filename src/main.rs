//! hmr - A slim, fast CLI for Home Assistant
//!
//! Provides essential functionality to interact with local or remote
//! Home Assistant instances from the terminal.

mod api;
mod cli;
mod commands;
mod config;
mod output;
mod websocket;

use std::io::{self, Write};
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Command};
use crate::config::RuntimeContext;

fn main() -> ExitCode {
    match try_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let _ = writeln!(io::stderr(), "Error: {err:#}");
            ExitCode::from(1)
        }
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();
    let ctx = RuntimeContext::new(&cli.global)?;
    ctx.init_logging()?;

    log::debug!("Config loaded from: {:?}", ctx.config_path());

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(run_command(&ctx, cli.command))
}

async fn run_command(ctx: &RuntimeContext, command: Command) -> Result<()> {
    match command {
        Command::Info => commands::info::run(ctx).await,
        Command::Entity { command } => commands::entity::run(ctx, command).await,
        Command::Service { command } => commands::service::run(ctx, command).await,
        Command::Event { command } => commands::event::run(ctx, command).await,
        Command::Template(cmd) => commands::template::run(ctx, cmd).await,
        Command::Area { command } => commands::area::run(ctx, command).await,
        Command::Device { command } => commands::device::run(ctx, command).await,
        Command::Config { command } => commands::config::run(ctx, command),
        Command::Completions { shell } => commands::completions::run(shell),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Cli::command().debug_assert();
    }
}
