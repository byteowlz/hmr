//! Template command implementation

use std::fs;

use anyhow::{Context, Result};

use crate::api::HassClient;
use crate::cli::TemplateCommand;
use crate::config::RuntimeContext;
use crate::output::read_stdin;

pub async fn run(ctx: &RuntimeContext, cmd: TemplateCommand) -> Result<()> {
    let client = HassClient::new(ctx)?;

    let template = if let Some(ref file_path) = cmd.file {
        fs::read_to_string(file_path)
            .with_context(|| format!("reading template file: {}", file_path.display()))?
    } else if let Some(ref template_str) = cmd.template {
        template_str.clone()
    } else {
        // Read from stdin
        read_stdin()?.ok_or_else(|| {
            anyhow::anyhow!(
                "No template provided. Use a template argument, --file, or pipe via stdin"
            )
        })?
    };

    let result = client.render_template(&template).await?;
    println!("{result}");

    Ok(())
}
