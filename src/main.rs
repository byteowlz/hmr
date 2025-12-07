//! hmr - A slim, fast CLI for Home Assistant
//!
//! Provides essential functionality to interact with local or remote
//! Home Assistant instances from the terminal.

mod api;
mod cache;
mod cli;
mod commands;
mod config;
mod fuzzy;
mod history;
mod natural_args;
mod nl;
mod output;
mod websocket;

use std::io::{self, Write};
use std::process::ExitCode;

use anyhow::Result;
use clap::{CommandFactory, Parser};

use crate::cli::{Cli, Command};
use crate::config::RuntimeContext;

fn main() -> ExitCode {
    // Reset SIGPIPE to default behavior to avoid panics on broken pipes
    // This prevents the "failed printing to stdout: Broken pipe" panic when
    // piping output to commands that don't read all the data (e.g., `hmr config path | cd`)
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    match try_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let _ = writeln!(io::stderr(), "Error: {err:#}");
            ExitCode::from(1)
        }
    }
}

fn try_main() -> Result<()> {
    // Normalize natural command variations before parsing
    let normalized_args = natural_args::normalize_args();
    let cli = Cli::parse_from(normalized_args);

    // If no command is provided, print help and exit
    let Some(command) = cli.command else {
        let _ = Cli::command().print_help();
        return Ok(());
    };

    let ctx = RuntimeContext::new(&cli.global)?;
    ctx.init_logging()?;

    log::debug!("Config loaded from: {:?}", ctx.config_path());

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(run_command(&ctx, command))
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
        Command::Cache { command } => commands::cache::execute(ctx, command).await,
        Command::Do(cmd) => commands::do_cmd::execute(ctx, cmd).await,
        Command::History { command } => commands::history::execute(ctx, command).await,
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
