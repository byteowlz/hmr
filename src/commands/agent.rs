//! Agent command - leverage Home Assistant's conversation agent

use anyhow::Result;

use crate::{api::HassClient, cli::AgentCommand, config::RuntimeContext, output};

pub async fn handle(client: &HassClient, cmd: &AgentCommand, ctx: &RuntimeContext) -> Result<()> {
    let text = cmd.words.join(" ");

    log::debug!(
        "Processing conversation: text='{}', lang={}, agent_id={:?}, conversation_id={:?}",
        text,
        cmd.lang,
        cmd.agent_id,
        cmd.conversation_id
    );

    let response = client
        .process_conversation(
            &text,
            Some(&cmd.lang),
            cmd.agent_id.as_deref(),
            cmd.conversation_id.as_deref(),
        )
        .await?;

    // Extract the speech response
    let speech_text = response
        .response
        .speech
        .as_ref()
        .map(|s| s.plain.speech.as_str())
        .unwrap_or("No response from agent");

    // Use output_for_format to respect output format settings
    output::output_for_format(ctx, &response, || {
        // For human-readable output, just print the speech
        println!("{}", speech_text);

        // If there's a conversation ID, show it for follow-up
        if let Some(conv_id) = &response.conversation_id {
            log::debug!("Conversation ID: {}", conv_id);
        }

        Ok(())
    })?;

    Ok(())
}
